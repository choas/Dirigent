use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// Install or remove the Claude Code home-directory guard hook.
///
/// When `allow_home_folder_access` is **false**, writes a `PreToolUse` hook
/// script and registers it in `.claude/settings.local.json` so Claude Code
/// blocks tool calls that try to read personal directories.
///
/// When `allow_home_folder_access` is **true**, removes the hook script and
/// its registration from the settings file.
pub(crate) fn sync_home_guard_hook(project_root: &Path, allow: bool) -> Result<()> {
    let dirigent_dir = project_root.join(".Dirigent");
    let claude_dir = project_root.join(".claude");
    let hook_script = dirigent_dir.join("home_guard.sh");
    let settings_file = claude_dir.join("settings.local.json");

    if allow {
        // --- Remove the hook ---
        // Ignore "not found" – the hook may already be absent.
        if hook_script.exists() {
            std::fs::remove_file(&hook_script).context("removing home_guard.sh")?;
        }
        remove_hook_from_settings(&settings_file)?;
    } else {
        // --- Install the hook ---
        std::fs::create_dir_all(&dirigent_dir).context("creating .Dirigent directory")?;
        std::fs::write(&hook_script, home_guard_script_content(project_root))
            .context("writing home_guard.sh")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_script, std::fs::Permissions::from_mode(0o755))
                .context("setting home_guard.sh permissions")?;
        }
        std::fs::create_dir_all(&claude_dir).context("creating .claude directory")?;
        upsert_hook_in_settings(&settings_file, &hook_script)?;
    }
    Ok(())
}

/// The shell script that Claude Code runs as a `PreToolUse` hook.
/// It checks every path-like value in the tool input JSON against a set of
/// restricted home sub-directories and exits with code 2 to block the call.
fn home_guard_script_content(project_root: &Path) -> String {
    r#"#!/bin/bash
# Dirigent home-directory guard – Claude Code PreToolUse hook.
# Blocks tool calls that reference personal home directories or
# recursively search from the home directory root.
INPUT=$(cat)
HOME_DIR="${HOME:-/Users/$(whoami)}"
PROJECT_ROOT="__PROJECT_ROOT__"

# 1. Block explicit references to personal sub-directories.
for DIR in Documents Desktop Downloads Photos Pictures Movies Music Library Applications .ssh .gnupg; do
    BLOCKED="$HOME_DIR/$DIR"
    # Skip if the project root lives under this blocked directory.
    case "$PROJECT_ROOT" in "$BLOCKED"|"$BLOCKED"/*) continue ;; esac
    if echo "$INPUT" | grep -qF "$BLOCKED"; then
        echo "Blocked by Dirigent: access to ~/$DIR is restricted. Disable the home-folder guard in Dirigent Settings to override."
        exit 2
    fi
done

# 2. Block recursive commands that start from the home directory itself
#    (e.g. "find /Users/lars -name foo" or "find ~ -type f").
#    These traverse into Documents, Desktop, Photos etc. and trigger macOS
#    permission pop-ups even though those paths aren't named explicitly.
#    We match: find <home> | find ~ | ls -R <home> | grep -r ... <home>
#    but NOT paths that go deeper (e.g. find /Users/lars/prj is fine).
HOME_ESC=$(printf '%s' "$HOME_DIR" | sed 's/[.[\*^$()+?{|]/\\&/g')
if echo "$INPUT" | grep -qE "(find|ls -[a-zA-Z]*R|grep -[a-zA-Z]*r|rg |fd |du |tree )[^\"]*($HOME_ESC|~)(/| |\"|\$)" 2>/dev/null; then
    # Make sure it's not targeting a deeper subdirectory within home
    if ! echo "$INPUT" | grep -qE "(find|ls|grep|rg|fd|du|tree)[^\"]*$HOME_ESC/[A-Za-z0-9._-]+[/ \"]" 2>/dev/null; then
        echo "Blocked by Dirigent: recursive search from home directory is restricted. Use a more specific path or disable the home-folder guard in Dirigent Settings."
        exit 2
    fi
fi

exit 0
"#
    .replace("__PROJECT_ROOT__", &project_root.to_string_lossy())
}

/// Add (or re-add) our guard hook entry to a `.claude/settings.local.json` file.
fn upsert_hook_in_settings(settings_path: &Path, hook_script: &Path) -> Result<()> {
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

    let pre = hooks
        .as_object_mut()
        .unwrap()
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));

    if let Some(arr) = pre.as_array_mut() {
        // Remove any previous guard entry
        arr.retain(|h| !h.to_string().contains("home_guard.sh"));
        // Add the new entry
        arr.push(serde_json::json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": hook_script.to_string_lossy().to_string()
            }]
        }));
    }

    let json = serde_json::to_string_pretty(&root).context("serializing settings JSON")?;
    atomic_write(settings_path, json.as_bytes()).context("writing settings.local.json")?;
    Ok(())
}

/// Remove our guard hook entry from a `.claude/settings.local.json` file.
fn remove_hook_from_settings(settings_path: &Path) -> Result<()> {
    if !settings_path.exists() {
        return Ok(());
    }
    let mut root = read_json_object(settings_path);

    let changed = if let Some(hooks) = root.get_mut("hooks") {
        if let Some(pre) = hooks.get_mut("PreToolUse") {
            if let Some(arr) = pre.as_array_mut() {
                let before = arr.len();
                arr.retain(|h| !h.to_string().contains("home_guard.sh"));
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
        let json = serde_json::to_string_pretty(&root).context("serializing settings JSON")?;
        atomic_write(settings_path, json.as_bytes()).context("writing settings.local.json")?;
    }
    Ok(())
}

/// Write `data` to `path` atomically: write to a temp file in the same
/// directory, fsync, then rename over the target. This prevents
/// crash-corruption of the settings file.
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .context("settings path has no parent directory")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).context("creating temp file")?;
    tmp.write_all(data).context("writing temp file")?;
    tmp.as_file().sync_all().context("fsync temp file")?;
    tmp.persist(path).context("renaming temp file into place")?;
    Ok(())
}

/// Read a JSON file as a `serde_json::Value` object, defaulting to `{}`.
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
