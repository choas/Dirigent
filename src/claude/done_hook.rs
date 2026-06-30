use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use claude_pty::StopHookSummary;

const HOOK_MARKER: &str = "dirigent-pty-done";

/// Serializes the read/modify/write of `.claude/settings.local.json` so that
/// two PTY runs starting in the same process at nearly the same time cannot both
/// read the same snapshot, append only their own hook, and clobber each other's
/// entry when the later `atomic_write` persists its stale copy.
fn settings_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) struct DoneHook {
    sentinel: PathBuf,
    payload: PathBuf,
    settings_path: PathBuf,
}

impl DoneHook {
    /// Install a Claude Code `Stop` hook that writes a sentinel file when
    /// Claude's turn ends. Returns a guard that removes the hook on drop.
    ///
    /// `session_id` scopes the hook to a single run: Claude Code fires *all*
    /// matching `Stop` hooks in parallel, so when two runs overlap in the same
    /// project the command must only touch its own sentinel when the triggering
    /// session matches. When the id is unknown (`None`) the hook fires
    /// unconditionally, which is safe for the common single-run case.
    pub fn install(project_root: &Path, session_id: Option<&str>) -> Option<Self> {
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
        if upsert_stop_hook(&settings_path, &sentinel, &payload, session_id).is_err() {
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

fn upsert_stop_hook(
    settings_path: &Path,
    sentinel: &Path,
    payload: &Path,
    session_id: Option<&str>,
) -> anyhow::Result<()> {
    // Hold the lock across the whole read/modify/write so a concurrent install
    // or removal cannot interleave and overwrite our entry with a stale file.
    let _guard = settings_lock().lock().unwrap_or_else(|e| e.into_inner());

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
                "command": stop_hook_command(sentinel, payload, session_id),
            }]
        }));
    }

    let json = serde_json::to_string_pretty(&root)?;
    atomic_write(settings_path, json.as_bytes())
}

/// Build the shell command Claude Code runs for this run's `Stop` hook.
///
/// Claude fires every matching `Stop` hook in parallel and feeds each the same
/// JSON on stdin. When `session_id` is known we gate the side effects on that id
/// appearing in the payload, so a sibling run finishing first does not touch
/// *this* run's sentinel and trip its still-running consume loop. Without an id
/// we fall back to the unconditional capture, which is correct when only one run
/// is active.
fn stop_hook_command(sentinel: &Path, payload: &Path, session_id: Option<&str>) -> String {
    match session_id {
        Some(id) if !id.is_empty() => format!(
            // $1 payload, $2 sentinel, $3 session id. Capture stdin once, then
            // only persist it when the triggering session matches ours.
            "sh -c 'input=$(cat); printf %s \"$input\" | grep -Eq \
             \"\\\"session_id\\\"[[:space:]]*:[[:space:]]*\\\"$3\\\"\" || exit 0; \
             printf %s \"$input\" > \"$1\"; touch \"$2\"' {} {} {} {}",
            HOOK_MARKER,
            shell_escape(payload),
            shell_escape(sentinel),
            shell_escape(Path::new(id)),
        ),
        _ => format!(
            "sh -c 'cat > \"$1\"; touch \"$2\"' {} {} {}",
            HOOK_MARKER,
            shell_escape(payload),
            shell_escape(sentinel),
        ),
    }
}

fn remove_stop_hook(settings_path: &Path, sentinel: &Path) -> anyhow::Result<()> {
    // Same lock as the install path: removing our hook also rewrites the whole
    // file, so it must not race with a concurrent run's install/removal.
    let _guard = settings_lock().lock().unwrap_or_else(|e| e.into_inner());

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
        upsert_stop_hook(&settings, &sentinel, &payload, None).unwrap();
        let json = std::fs::read_to_string(settings).unwrap();
        assert!(json.contains("cat >"));
        assert!(json.contains(payload.to_str().unwrap()));
        assert!(json.contains(sentinel.to_str().unwrap()));
    }

    #[test]
    fn upsert_stop_hook_scopes_command_to_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join("settings.local.json");
        let sentinel = tmp.path().join("sentinel");
        let payload = tmp.path().join("payload.json");
        upsert_stop_hook(&settings, &sentinel, &payload, Some("sess-123")).unwrap();
        let json = std::fs::read_to_string(settings).unwrap();
        // The command only touches the sentinel when the triggering session's id
        // appears in the hook payload, so a sibling run cannot trip this one.
        assert!(json.contains("session_id"));
        assert!(json.contains("sess-123"));
        assert!(json.contains("grep -Eq"));
        assert!(json.contains("|| exit 0"));
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
        upsert_stop_hook(&settings, &sentinel_a, &payload_a, Some("sess-a")).unwrap();
        upsert_stop_hook(&settings, &sentinel_b, &payload_b, Some("sess-b")).unwrap();

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

    #[test]
    fn concurrent_installs_do_not_drop_each_other() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join("settings.local.json");

        // Many threads install their hook into the same file at once. Without
        // serialization the read/modify/write races and entries get clobbered.
        const RUNS: usize = 16;
        std::thread::scope(|scope| {
            for i in 0..RUNS {
                let settings = settings.clone();
                let dir = tmp.path().to_path_buf();
                scope.spawn(move || {
                    let sentinel = dir.join(format!("dirigent-pty-done-{i}"));
                    let payload = sentinel.with_extension("json");
                    upsert_stop_hook(&settings, &sentinel, &payload, Some(&format!("sess-{i}")))
                        .unwrap();
                });
            }
        });

        // Every run's hook must have survived the concurrent installs.
        assert_eq!(stop_hook_count(&settings), RUNS);
        let json = std::fs::read_to_string(&settings).unwrap();
        for i in 0..RUNS {
            let sentinel = tmp.path().join(format!("dirigent-pty-done-{i}"));
            assert!(json.contains(&shell_escape(&sentinel)));
        }
    }
}
