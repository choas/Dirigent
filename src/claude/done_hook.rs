use std::io::Write;
use std::path::{Path, PathBuf};

use claude_pty::StopHookSummary;

const HOOK_MARKER: &str = "dirigent-pty-done";

pub(super) struct DoneHook {
    sentinel: PathBuf,
    payload: PathBuf,
    settings_path: PathBuf,
}

impl DoneHook {
    /// Install a Claude Code `Stop` hook that writes a sentinel file when
    /// Claude's turn ends. Returns a guard that removes the hook on drop.
    pub fn install(project_root: &Path) -> Option<Self> {
        let sentinel = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            HOOK_MARKER,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let payload = sentinel.with_extension("json");

        let claude_dir = project_root.join(".claude");
        let settings_path = claude_dir.join("settings.local.json");

        if std::fs::create_dir_all(&claude_dir).is_err() {
            return None;
        }
        if upsert_stop_hook(&settings_path, &sentinel, &payload).is_err() {
            return None;
        }

        Some(Self {
            sentinel,
            payload,
            settings_path,
        })
    }

    pub fn sentinel_path(&self) -> &Path {
        &self.sentinel
    }

    pub fn payload_path(&self) -> &Path {
        &self.payload
    }

    pub fn read_summary(&self) -> Option<StopHookSummary> {
        read_stop_hook_summary(&self.payload)
    }
}

impl Drop for DoneHook {
    fn drop(&mut self) {
        let _ = remove_stop_hook(&self.settings_path, &self.sentinel);
        let _ = std::fs::remove_file(&self.sentinel);
        let _ = std::fs::remove_file(&self.payload);
    }
}

fn upsert_stop_hook(settings_path: &Path, sentinel: &Path, payload: &Path) -> anyhow::Result<()> {
    let mut root = read_json_object(settings_path);
    if !root.is_object() {
        root = serde_json::json!({});
    }

    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }

    let stop = hooks
        .as_object_mut()
        .unwrap()
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]));
    if let Some(arr) = stop.as_array_mut() {
        // Only drop a stale entry that belongs to *this* run's sentinel so that
        // concurrent runs in the same repo keep each other's hooks intact.
        let token = shell_escape(sentinel);
        arr.retain(|h| !h.to_string().contains(&token));
        arr.push(serde_json::json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": format!(
                    "sh -c 'cat > \"$1\"; touch \"$2\"' {} {} {}",
                    HOOK_MARKER,
                    shell_escape(payload),
                    shell_escape(sentinel),
                )
            }]
        }));
    }

    let json = serde_json::to_string_pretty(&root)?;
    atomic_write(settings_path, json.as_bytes())
}

fn remove_stop_hook(settings_path: &Path, sentinel: &Path) -> anyhow::Result<()> {
    if !settings_path.exists() {
        return Ok(());
    }
    let mut root = read_json_object(settings_path);

    let token = shell_escape(sentinel);
    let changed = if let Some(hooks) = root.get_mut("hooks") {
        if let Some(stop) = hooks.get_mut("Stop") {
            if let Some(arr) = stop.as_array_mut() {
                let before = arr.len();
                // Remove only this run's hook, leaving any concurrent run's hook.
                arr.retain(|h| !h.to_string().contains(&token));
                arr.len() != before
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if changed {
        let json = serde_json::to_string_pretty(&root)?;
        atomic_write(settings_path, json.as_bytes())?;
    }
    Ok(())
}

fn shell_escape(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains('\'') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        format!("'{s}'")
    }
}

fn read_stop_hook_summary(path: &Path) -> Option<StopHookSummary> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_stop_hook_summary(&raw)
}

pub(super) fn parse_stop_hook_summary(raw: &str) -> Option<StopHookSummary> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let event_name = value
        .get("hook_event_name")
        .or_else(|| value.get("event_name"))
        .or_else(|| value.get("event"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let last_assistant_message = value
        .get("last_assistant_message")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let failure = value
        .get("stop_failure")
        .or_else(|| value.get("StopFailure"))
        .or_else(|| value.get("error"))
        .map(|v| match v.as_str() {
            Some(s) => s.to_string(),
            None => v.to_string(),
        });
    let session_id = value
        .get("session_id")
        .or_else(|| value.get("sessionId"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let timestamp = value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    Some(StopHookSummary {
        event_name,
        last_assistant_message,
        failure,
        session_id,
        timestamp,
    })
}

fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().unwrap_or(path);
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}

fn read_json_object(path: &Path) -> serde_json::Value {
    if path.exists() {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stop_hook_summary_reads_last_message_and_failure() {
        let summary = parse_stop_hook_summary(
            r#"{
                "hook_event_name": "Stop",
                "last_assistant_message": "Done. Anything else?",
                "stop_failure": {"message": "tool failed"},
                "session_id": "abc",
                "timestamp": "2026-06-18T10:00:00Z"
            }"#,
        )
        .unwrap();
        assert_eq!(summary.event_name.as_deref(), Some("Stop"));
        assert_eq!(
            summary.last_assistant_message.as_deref(),
            Some("Done. Anything else?")
        );
        assert!(summary.failure.as_deref().unwrap().contains("tool failed"));
        assert_eq!(summary.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn upsert_stop_hook_writes_payload_capture_command() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join("settings.local.json");
        let sentinel = tmp.path().join("sentinel");
        let payload = tmp.path().join("payload.json");
        upsert_stop_hook(&settings, &sentinel, &payload).unwrap();
        let json = std::fs::read_to_string(settings).unwrap();
        assert!(json.contains("cat >"));
        assert!(json.contains(payload.to_str().unwrap()));
        assert!(json.contains(sentinel.to_str().unwrap()));
    }

    fn stop_hook_count(settings: &Path) -> usize {
        let json = std::fs::read_to_string(settings).unwrap();
        let root: serde_json::Value = serde_json::from_str(&json).unwrap();
        root["hooks"]["Stop"].as_array().map_or(0, Vec::len)
    }

    #[test]
    fn concurrent_runs_preserve_each_others_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join("settings.local.json");
        let sentinel_a = tmp.path().join("dirigent-pty-done-1-100");
        let payload_a = sentinel_a.with_extension("json");
        let sentinel_b = tmp.path().join("dirigent-pty-done-1-1000");
        let payload_b = sentinel_b.with_extension("json");

        // Two overlapping runs install their hooks.
        upsert_stop_hook(&settings, &sentinel_a, &payload_a).unwrap();
        upsert_stop_hook(&settings, &sentinel_b, &payload_b).unwrap();

        // Both hooks must coexist; installing B must not drop A. Match on the
        // quoted argument tokens since A's sentinel is a string prefix of B's.
        let token_a = shell_escape(&sentinel_a);
        let token_b = shell_escape(&sentinel_b);
        assert_eq!(stop_hook_count(&settings), 2);
        let json = std::fs::read_to_string(&settings).unwrap();
        assert!(json.contains(&token_a));
        assert!(json.contains(&token_b));

        // Dropping run A removes only A's hook, leaving B untouched.
        remove_stop_hook(&settings, &sentinel_a).unwrap();
        assert_eq!(stop_hook_count(&settings), 1);
        let json = std::fs::read_to_string(&settings).unwrap();
        assert!(!json.contains(&token_a));
        assert!(json.contains(&token_b));

        remove_stop_hook(&settings, &sentinel_b).unwrap();
        assert_eq!(stop_hook_count(&settings), 0);
    }
}
