use std::io::Write;
use std::path::{Path, PathBuf};

const HOOK_MARKER: &str = "dirigent-pty-done";

pub(super) struct DoneHook {
    sentinel: PathBuf,
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

        let claude_dir = project_root.join(".claude");
        let settings_path = claude_dir.join("settings.local.json");

        if std::fs::create_dir_all(&claude_dir).is_err() {
            return None;
        }
        if upsert_stop_hook(&settings_path, &sentinel).is_err() {
            return None;
        }

        Some(Self {
            sentinel,
            settings_path,
        })
    }

    pub fn sentinel_path(&self) -> &Path {
        &self.sentinel
    }
}

impl Drop for DoneHook {
    fn drop(&mut self) {
        let _ = remove_stop_hook(&self.settings_path);
        let _ = std::fs::remove_file(&self.sentinel);
    }
}

fn upsert_stop_hook(settings_path: &Path, sentinel: &Path) -> anyhow::Result<()> {
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
        arr.retain(|h| !h.to_string().contains(HOOK_MARKER));
        arr.push(serde_json::json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": format!("touch {}", shell_escape(sentinel))
            }]
        }));
    }

    let json = serde_json::to_string_pretty(&root)?;
    atomic_write(settings_path, json.as_bytes())
}

fn remove_stop_hook(settings_path: &Path) -> anyhow::Result<()> {
    if !settings_path.exists() {
        return Ok(());
    }
    let mut root = read_json_object(settings_path);

    let changed = if let Some(hooks) = root.get_mut("hooks") {
        if let Some(stop) = hooks.get_mut("Stop") {
            if let Some(arr) = stop.as_array_mut() {
                let before = arr.len();
                arr.retain(|h| !h.to_string().contains(HOOK_MARKER));
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
