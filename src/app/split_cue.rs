use std::sync::mpsc;

use crate::db::{Cue, CueStatus};

use super::{DirigentApp, SplitCueItem};

impl DirigentApp {
    pub(super) fn start_split_cue(&mut self, cue_id: i64) {
        if self.split_cue_generating {
            self.set_status_message("Split already in progress".into());
            return;
        }

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let ref_file = write_reference_file(&self.project_root, &cue);
        let prompt = build_split_prompt(&cue, ref_file.as_deref());

        self.split_cue_generating = true;
        self.split_cue_source_id = Some(cue_id);
        self.split_cue_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let provider = self.settings.cli_provider.clone();
        let project_root = self.project_root.clone();
        let settings = self.settings.clone();
        let cancel = std::sync::Arc::clone(&self.split_cue_cancel);

        let (tx, rx) = mpsc::channel();
        self.split_cue_rx = Some(rx);
        let ctx = self.egui_ctx.clone();

        std::thread::spawn(move || {
            let result = run_split_analysis(&prompt, &provider, &project_root, &settings, cancel);
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });

        self.set_status_message("Splitting cue...".into());
    }

    pub(super) fn process_split_cue_result(&mut self) {
        let rx = match self.split_cue_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.split_cue_generating = false;
                self.split_cue_rx = None;
                self.split_cue_source_id = None;
                self.set_status_message("Split cue failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.split_cue_generating = false;
        self.split_cue_rx = None;
        let source_id = self.split_cue_source_id.take();

        match result {
            Ok(items) if items.is_empty() => {
                self.set_status_message("Split produced no sub-cues".into());
            }
            Ok(items) => {
                let source_cue =
                    source_id.and_then(|id| self.cues.iter().find(|c| c.id == id).cloned());
                let total = items.len();
                let mut success_count = 0usize;
                let mut errors: Vec<String> = Vec::new();
                for item in &items {
                    let text = if let Some(ref r) = item.reference {
                        format!("{}\n\nReference: {}", item.text, r)
                    } else {
                        item.text.clone()
                    };
                    let (file_path, line) = source_cue
                        .as_ref()
                        .map(|c| (c.file_path.as_str(), c.line_number))
                        .unwrap_or(("", 0));
                    match self.db.insert_cue(&text, file_path, line, None, &[]) {
                        Ok(_) => success_count += 1,
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                if let Some(id) = source_id {
                    if errors.is_empty() {
                        let _ = self
                            .db
                            .log_activity(id, &format!("Split into {} cues", success_count));
                        let _ = self.db.update_cue_status(id, CueStatus::Archived);
                        let _ = self.db.log_activity(id, "Archived (split)");
                    } else {
                        let summary = format!(
                            "Split partially failed: {}/{} inserted, errors: {}",
                            success_count,
                            total,
                            errors.join("; ")
                        );
                        let _ = self.db.log_activity(id, &summary);
                    }
                }
                self.reload_cues();
                if errors.is_empty() {
                    self.set_status_message(format!("Split into {} cues", success_count));
                } else {
                    self.set_status_message(format!(
                        "Split: {}/{} cues created, {} failed",
                        success_count,
                        total,
                        errors.len()
                    ));
                }
            }
            Err(e) => {
                self.set_status_message(format!("Split cue failed: {}", e));
            }
        }
    }
}

fn write_reference_file(project_root: &std::path::Path, cue: &Cue) -> Option<String> {
    let text = cue.text.trim();
    if text.len() < 200 {
        return None;
    }
    let dir = project_root.join(".Dirigent");
    let _ = std::fs::create_dir_all(&dir);
    let filename = format!("split-ref-{}.md", cue.id);
    let path = dir.join(&filename);
    if std::fs::write(&path, text).is_ok() {
        Some(format!(".Dirigent/{}", filename))
    } else {
        None
    }
}

fn build_split_prompt(cue: &Cue, ref_file: Option<&str>) -> String {
    let mut prompt = String::from(
        "You are a task splitter. Analyze the following cue and split it into \
         independent, actionable sub-tasks. Each sub-task should be self-contained \
         enough to be executed independently.\n\n",
    );

    if let Some(path) = ref_file {
        prompt.push_str(&format!(
            "The full original text has been saved to \"{}\" for reference. \
             Each sub-cue should reference the relevant section of that file \
             so context is preserved.\n\n",
            path,
        ));
    }

    prompt.push_str("Original cue:\n---\n");
    prompt.push_str(cue.text.trim());
    prompt.push_str("\n---\n");

    if !cue.file_path.is_empty() {
        prompt.push_str(&format!(
            "\nFile context: {} (line {})\n",
            cue.file_path, cue.line_number,
        ));
    }

    prompt.push_str(
        "\nOutput ONLY valid JSON (no markdown fences, no commentary) with this structure:\n\
         {\n  \"cues\": [\n    {\n      \
         \"text\": \"concise task description\",\n      \
         \"reference\": \"optional: file path and section for context\"\n    \
         }\n  ]\n}\n\n\
         Rules:\n\
         - Each sub-cue text should be a clear, actionable instruction.\n\
         - Cover ALL tasks from the original cue — do not drop anything.\n\
         - If the original cue is already a single atomic task, return it as-is in one cue.\n\
         - The \"reference\" field is optional. Use it when a sub-task needs context from \
         a larger document (e.g. \"<file> section <heading>\").\n\
         - Keep sub-cue texts concise but include enough context to stand alone.\n\
         - Do NOT modify any files. Only analyze and return JSON.\n",
    );
    prompt
}

#[derive(serde::Deserialize)]
struct LlmSplitResponse {
    cues: Vec<LlmSplitCue>,
}

#[derive(serde::Deserialize)]
struct LlmSplitCue {
    text: String,
    reference: Option<String>,
}

fn extract_json(s: &str) -> String {
    if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = s.find("```") {
        let after = &s[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if end > start {
                return s[start..=end].to_string();
            }
        }
    }
    s.trim().to_string()
}

fn parse_split_response(response: &str) -> Result<Vec<SplitCueItem>, String> {
    let json_str = extract_json(response);
    let parsed: LlmSplitResponse = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse split JSON: {}", e))?;

    if parsed.cues.is_empty() {
        return Err("LLM returned no sub-cues".into());
    }

    Ok(parsed
        .cues
        .into_iter()
        .map(|c| SplitCueItem {
            text: c.text,
            reference: c.reference,
        })
        .collect())
}

fn run_split_analysis(
    prompt: &str,
    provider: &crate::settings::CliProvider,
    project_root: &std::path::Path,
    settings: &crate::settings::Settings,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Vec<SplitCueItem>, String> {
    use crate::settings::CliProvider;

    let pf = settings.provider_fields(provider);

    let response_text = match provider {
        CliProvider::Claude => {
            let result = crate::claude::invoke_claude_streaming(
                prompt,
                project_root,
                pf.model,
                pf.cli_path,
                pf.extra_args,
                pf.env_vars,
                pf.pre_run_script,
                pf.post_run_script,
                settings.allow_dangerous_skip_permissions,
                |_| {},
                cancel,
            )
            .map_err(|e| format!("Claude invocation failed: {}", e))?;
            result.stdout
        }
        CliProvider::OpenCode => {
            let config = crate::opencode::OpenCodeRunConfig {
                model: pf.model,
                cli_path: pf.cli_path,
                extra_args: pf.extra_args,
                env_vars: pf.env_vars,
                pre_run_script: pf.pre_run_script,
                post_run_script: pf.post_run_script,
            };
            let result = crate::opencode::invoke_opencode_streaming(
                prompt,
                project_root,
                &config,
                |_| {},
                cancel,
            )
            .map_err(|e| format!("OpenCode invocation failed: {}", e))?;
            result.stdout
        }
    };

    parse_split_response(&response_text)
}
