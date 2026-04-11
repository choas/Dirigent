use eframe::egui;

use super::super::{icon_small, DirigentApp};
use crate::agents::{AgentKind, AgentStatus};
use crate::git;

/// Return the current process RSS in megabytes (macOS only).
#[cfg(target_os = "macos")]
fn get_memory_usage_mb() -> Option<u64> {
    use std::mem;

    #[allow(non_camel_case_types)]
    type mach_port_t = u32;
    #[allow(non_camel_case_types)]
    type kern_return_t = i32;
    #[allow(non_camel_case_types)]
    type task_flavor_t = u32;
    #[allow(non_camel_case_types)]
    type task_info_t = *mut i32;
    #[allow(non_camel_case_types)]
    type mach_msg_type_number_t = u32;

    const MACH_TASK_BASIC_INFO: task_flavor_t = 20;
    const MACH_TASK_BASIC_INFO_COUNT: mach_msg_type_number_t = 12; // sizeof(mach_task_basic_info) / sizeof(natural_t)

    #[repr(C)]
    #[allow(non_camel_case_types)]
    struct mach_task_basic_info {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: [i32; 2],   // time_value_t
        system_time: [i32; 2], // time_value_t
        policy: i32,
        suspend_count: i32,
    }

    extern "C" {
        fn mach_task_self() -> mach_port_t;
        fn task_info(
            target_task: mach_port_t,
            flavor: task_flavor_t,
            task_info_out: task_info_t,
            task_info_outCnt: *mut mach_msg_type_number_t,
        ) -> kern_return_t;
    }

    unsafe {
        let mut info: mach_task_basic_info = mem::zeroed();
        let mut count = MACH_TASK_BASIC_INFO_COUNT;
        let kr = task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            &mut info as *mut _ as task_info_t,
            &mut count,
        );
        if kr == 0 {
            Some(info.resident_size / (1024 * 1024))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn get_memory_usage_mb() -> Option<u64> {
    None
}

impl DirigentApp {
    pub(in super::super) fn render_status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                self.render_status_bar_git_info(ui);
                self.render_status_bar_db_cost(ui);
                self.render_status_bar_agents(ui, &ctx);
                self.render_status_bar_cached_cost(ui);
                self.render_status_bar_message(ui, &ctx);
            });
        });
    }

    /// Render the git branch and status summary in the status bar.
    fn render_status_bar_git_info(&mut self, ui: &mut egui::Ui) {
        if let Some(ref info) = self.git.info {
            let branch_label = ui.label(icon_small(
                &format!("\u{25CF} {}", info.branch),
                self.settings.font_size,
            ));
            branch_label.on_hover_text(format!(
                "{} {}",
                info.last_commit_hash, info.last_commit_message
            ));
            let summary = git::format_status_summary(info);
            if !summary.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(summary).monospace().small());
            }
        } else if ui
            .add(
                egui::Label::new(
                    egui::RichText::new("not a git repository \u{2014} click to init")
                        .monospace()
                        .small()
                        .color(self.semantic.tertiary_text),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            self.git_init_confirm = Some(self.project_root.clone());
        }
    }

    /// Render the total DB cost (inline, left-aligned) in the status bar.
    fn render_status_bar_db_cost(&self, ui: &mut egui::Ui) {
        if self.cached_total_cost > 0.0 {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("${:.2}", self.cached_total_cost))
                    .monospace()
                    .small()
                    .color(self.semantic.tertiary_text),
            )
            .on_hover_text("Total API cost for this project");
        }
    }

    /// Render agent status indicators and request repaint while running.
    fn render_status_bar_agents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let has_any_status = self
            .settings
            .agents
            .iter()
            .any(|a| a.enabled && self.agent_state.statuses.contains_key(&a.kind));

        if has_any_status {
            ui.separator();
            // Collect agent info to avoid borrowing self.settings while calling &mut self.
            let agent_items: Vec<(AgentKind, String)> = self
                .settings
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| (a.kind, a.display_name().to_string()))
                .collect();
            for (kind, name) in &agent_items {
                self.render_single_agent_status(ui, *kind, name);
            }
        }
        if self
            .agent_state
            .statuses
            .values()
            .any(|s| *s == AgentStatus::Running)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Render a single agent's status indicator in the status bar.
    fn render_single_agent_status(&mut self, ui: &mut egui::Ui, kind: AgentKind, name: &str) {
        let status = self
            .agent_state
            .statuses
            .get(&kind)
            .copied()
            .unwrap_or(AgentStatus::Idle);
        let (icon_str, color) = match status {
            AgentStatus::Idle => return,
            AgentStatus::Running => ("\u{21BB}", self.semantic.accent),
            AgentStatus::Passed => ("\u{2713}", self.semantic.success),
            AgentStatus::Failed => ("\u{2717}", self.semantic.danger),
            AgentStatus::Error => ("!", self.semantic.danger),
        };
        let label_text = format!("{} {}", name, icon_str);
        let mut resp = ui.add(
            egui::Label::new(
                egui::RichText::new(&label_text)
                    .monospace()
                    .small()
                    .color(color),
            )
            .sense(egui::Sense::click()),
        );
        if let Some(output) = self.agent_state.latest_output.get(&kind) {
            let preview = if output.len() > 300 {
                format!("{}...", super::super::truncate_str(output, 300))
            } else {
                output.clone()
            };
            resp = resp.on_hover_text(preview);
        }
        if resp.clicked() {
            if self.agent_state.show_output == Some(kind) {
                self.agent_state.show_output = None;
            } else {
                self.agent_state.show_output = Some(kind);
            }
        }
    }

    /// Render right-aligned info (memory usage, total cost) in the status bar.
    fn render_status_bar_cached_cost(&self, ui: &mut egui::Ui) {
        let mem = get_memory_usage_mb();
        if self.cached_total_cost > 0.0 || mem.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(mb) = mem {
                    ui.label(
                        egui::RichText::new(format!("{mb} MB"))
                            .monospace()
                            .small()
                            .color(self.semantic.muted_text()),
                    )
                    .on_hover_text("Process memory (RSS)");
                }
                if self.cached_total_cost > 0.0 {
                    if mem.is_some() {
                        ui.separator();
                    }
                    ui.label(
                        egui::RichText::new(format!("${:.2}", self.cached_total_cost))
                            .monospace()
                            .small()
                            .color(self.semantic.muted_text()),
                    )
                    .on_hover_text("Total project cost across all runs");
                }
            });
        }
    }

    /// Render the transient status message with auto-dismiss and fade.
    fn render_status_bar_message(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.git.importing_pr
            || self.git.pushing
            || self.git.pulling
            || self.git.creating_pr
            || self.git.notifying_pr
            || self.git.moving_to_branch;
        let expired = !busy
            && matches!(&self.status_message, Some((_, when)) if when.elapsed().as_secs() >= 6);
        if expired {
            self.status_message = None;
        }
        if let Some((ref msg, ref when)) = self.status_message {
            let elapsed = when.elapsed().as_secs_f32();
            let alpha = if elapsed > 4.0 {
                ((6.0 - elapsed) / 2.0).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let color = self.semantic.status_message_with_alpha(alpha);
            ui.separator();
            let resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(msg.as_str())
                        .monospace()
                        .small()
                        .color(color),
                )
                .sense(egui::Sense::click()),
            );
            resp.clone().on_hover_text("Click to copy");
            if resp.clicked() {
                ui.ctx().copy_text(msg.clone());
            }
            if elapsed > 4.0 {
                ctx.request_repaint();
            }
        }
    }
}
