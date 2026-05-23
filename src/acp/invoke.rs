use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::claude::RunMetrics;

use super::client::AcpConnection;
use super::types::{AcpError, DiffContent};

/// Configuration for an ACP agent invocation.
pub(crate) struct AcpRunConfig<'a> {
    pub binary: &'a str,
    pub args: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
}

/// Result of an ACP invocation, compatible with the existing result pipeline.
pub(crate) struct AcpRunResult {
    pub response_text: String,
    pub edited_files: Vec<String>,
    pub diffs: Vec<DiffContent>,
    pub metrics: RunMetrics,
}

/// Invoke an ACP-compatible agent with the given prompt.
///
/// This handles the full lifecycle: spawn → initialize → session/new → session/prompt → shutdown.
/// Streams progress via `on_log` callback (same pattern as Claude/OpenCode/Gemini providers).
pub(crate) fn invoke_acp_agent(
    prompt: &str,
    project_root: &Path,
    config: &AcpRunConfig,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<AcpRunResult, AcpError> {
    use crate::claude::{run_lifecycle_script, ClaudeError};

    // Run pre-run script.
    if !config.pre_run_script.is_empty() {
        let script_result = run_lifecycle_script(
            config.pre_run_script,
            "pre-run",
            project_root,
            &mut on_log,
            true,
        );
        if let Err(ClaudeError::SpawnFailed(e)) = script_result {
            return Err(AcpError::SpawnFailed(e));
        }
    }

    let binary = resolve_acp_binary(config.binary)?;
    let args_owned: Vec<String> = if config.args.is_empty() {
        Vec::new()
    } else {
        shlex::split(config.args).unwrap_or_default()
    };
    let args_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();

    if cancel.load(Ordering::Relaxed) {
        return Err(AcpError::Cancelled);
    }

    let start = std::time::Instant::now();

    // Spawn and initialize the ACP agent.
    let mut conn = AcpConnection::spawn_and_initialize(&binary, &args_refs, project_root, &mut on_log)?;

    if cancel.load(Ordering::Relaxed) {
        conn.shutdown();
        return Err(AcpError::Cancelled);
    }

    // Create a session.
    on_log("[ACP] Creating session...");
    let _session_id = conn.create_session(project_root)?;
    on_log("[ACP] Session created. Sending prompt...");

    if cancel.load(Ordering::Relaxed) {
        conn.shutdown();
        return Err(AcpError::Cancelled);
    }

    // Send the prompt and stream updates.
    let mut collected_diffs: Vec<DiffContent> = Vec::new();
    let mut collected_edited: Vec<String> = Vec::new();

    let result = conn.send_prompt(
        prompt,
        &cancel,
        &mut on_log,
        &mut |diff| {
            collected_diffs.push(diff);
        },
        &mut |path| {
            if !collected_edited.contains(&path.to_string()) {
                collected_edited.push(path.to_string());
            }
        },
    );

    conn.shutdown();

    let duration_ms = start.elapsed().as_millis() as u64;

    // Run post-run script.
    if !config.post_run_script.is_empty() {
        let _ = run_lifecycle_script(
            config.post_run_script,
            "post-run",
            project_root,
            &mut on_log,
            false,
        );
    }

    match result {
        Ok(response) => {
            let mut all_edited = response.edited_files;
            for f in collected_edited {
                if !all_edited.contains(&f) {
                    all_edited.push(f);
                }
            }
            let mut all_diffs = response.diffs;
            all_diffs.extend(collected_diffs);

            on_log(&format!(
                "\n[ACP] Completed: {} tool calls, {} files edited",
                response.tool_calls_completed,
                all_edited.len()
            ));

            Ok(AcpRunResult {
                response_text: response.text,
                edited_files: all_edited,
                diffs: all_diffs,
                metrics: RunMetrics {
                    cost_usd: 0.0,
                    duration_ms,
                    num_turns: response.tool_calls_completed,
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }
        Err(e) => Err(e),
    }
}

/// Resolve the ACP agent binary path.
fn resolve_acp_binary(path: &str) -> Result<String, AcpError> {
    if path.is_empty() {
        return Err(AcpError::NotFound("(empty path)".into()));
    }
    if std::path::Path::new(path).is_absolute() && std::path::Path::new(path).exists() {
        return Ok(path.to_string());
    }
    which::which(path)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| AcpError::NotFound(path.to_string()))
}

/// Convert ACP diffs into a unified diff string for the existing diff viewer.
pub(crate) fn diffs_to_unified(diffs: &[DiffContent]) -> Option<String> {
    if diffs.is_empty() {
        return None;
    }

    let mut output = String::new();
    for diff in diffs {
        output.push_str(&format!("--- a/{}\n", diff.path));
        output.push_str(&format!("+++ b/{}\n", diff.path));

        let old_lines: Vec<&str> = diff.old_text.lines().collect();
        let new_lines: Vec<&str> = diff.new_text.lines().collect();

        // Simple unified diff: show entire file as one hunk.
        output.push_str(&format!(
            "@@ -1,{} +1,{} @@\n",
            old_lines.len(),
            new_lines.len()
        ));
        for line in &old_lines {
            output.push_str(&format!("-{line}\n"));
        }
        for line in &new_lines {
            output.push_str(&format!("+{line}\n"));
        }
    }

    Some(output)
}
