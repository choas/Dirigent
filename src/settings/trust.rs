//! Trust gate for `.Dirigent/settings.json`.
//!
//! A repository's settings file is attacker-controlled input: it arrives with
//! the checkout, before the user has seen a single line of code. Many settings
//! reach process execution or data exfiltration (LSP server commands, source
//! commands, pre/post-run scripts, agent commands, extra CLI args, LLM base
//! URLs, program paths, ...). Blocking individual fields does not scale — any
//! newly added field would silently reopen the hole.
//!
//! This module therefore splits settings by *capability*, deny-by-default:
//!
//! - **Repo-scoped safe fields** ([`REPO_SAFE_FIELDS`]): purely cosmetic /
//!   display settings that are always honored from a repository's file.
//! - **Execution-capable fields**: everything else, including any field added
//!   in the future. They only take effect after the user has explicitly
//!   approved them for this repository.
//!
//! Approval is machine-scoped: it is remembered in the user profile
//! (`~/Library/Application Support/Dirigent/trusted_repos.json`), keyed by
//! repository path, as a content hash of exactly the execution-capable fields.
//! If the file changes those fields (e.g. after a `git pull`), the hash no
//! longer matches and the user is prompted again with the new values.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level `settings.json` fields that are safe to honor from any
/// repository without a trust decision: they only affect how the app looks or
/// which notifications play — they can never cause a process to be spawned or
/// data to leave the machine.
///
/// Everything **not** on this list is treated as execution-capable, including
/// unknown/future fields, so adding a new setting is gated by default and must
/// be consciously added here to become repo-scoped.
const REPO_SAFE_FIELDS: &[&str] = &[
    "theme",
    "custom_themes",
    "diff_color_scheme",
    "font_family",
    "font_size",
    "font_weight",
    "notify_sound",
    "notify_popup",
    "running_animation",
    "heartbeat_style",
    "claude_code_display_name",
    "show_frame_timing",
    "prompt_suggestions_enabled",
    "auto_context_file",
    "auto_context_git_diff",
    "commit_footer",
    "auto_continue_max",
    "recent_repos",
];

pub(crate) fn is_repo_safe_field(key: &str) -> bool {
    REPO_SAFE_FIELDS.contains(&key)
}

/// Extract the execution-capable subset of a parsed settings.json object.
/// A `BTreeMap` keeps the serialization (and thus the hash) deterministic.
fn gated_fields(value: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    let Some(obj) = value.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter(|(k, _)| !is_repo_safe_field(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn gated_hash(fields: &BTreeMap<String, serde_json::Value>) -> String {
    let canonical = serde_json::to_string(fields).unwrap_or_default();
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

// ---------------------------------------------------------------------------
// Machine-scoped trust store (user profile, keyed by repository path)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustStore {
    #[serde(default)]
    repos: BTreeMap<String, TrustedRepo>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustedRepo {
    /// Approved content hashes of the execution-capable field set. A small
    /// history (not just the latest) so switching between branches with
    /// different — but each previously approved — settings does not re-prompt.
    #[serde(default)]
    hashes: Vec<String>,
    #[serde(default)]
    approved_at: String,
}

/// `~/Library/Application Support/Dirigent/trusted_repos.json`
fn store_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/Dirigent/trusted_repos.json"))
}

fn load_store(path: &Path) -> TrustStore {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => TrustStore::default(),
    }
}

fn save_store(path: &Path, store: &TrustStore) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(store)
        && let Err(e) = std::fs::write(path, json)
    {
        log::warn!("[trust] failed to write trust store: {e}");
    }
}

fn canonical_key(repo_root: &Path) -> String {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// For a linked git worktree, the root of its main repository. A worktree of
/// an already-trusted repository should not re-prompt on every checkout.
fn worktree_main_root(repo_root: &Path) -> Option<PathBuf> {
    let repo = git2::Repository::open(repo_root).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    // commondir is the main repository's .git directory.
    repo.commondir().parent().map(Path::to_path_buf)
}

fn lookup_keys(repo_root: &Path) -> Vec<String> {
    let mut keys = vec![canonical_key(repo_root)];
    if let Some(main) = worktree_main_root(repo_root) {
        keys.push(canonical_key(&main));
    }
    keys
}

// ---------------------------------------------------------------------------
// Evaluation and approval
// ---------------------------------------------------------------------------

pub(crate) struct TrustCheck {
    /// Whether the repository's execution-capable settings may be honored.
    pub trusted: bool,
    /// Content hash of the execution-capable field set (what an approval records).
    pub hash: String,
    /// (field name, pretty-printed JSON value) for every execution-capable
    /// field the repository file sets — shown verbatim in the approval dialog.
    pub fields: Vec<(String, String)>,
}

/// Evaluate the trust status of a repository's parsed settings.json
/// (`None` when the file is absent or unparsable).
pub(crate) fn evaluate(repo_root: &Path, file_value: Option<&serde_json::Value>) -> TrustCheck {
    let store = store_path().map(|p| load_store(&p)).unwrap_or_default();
    evaluate_with_store(&store, &lookup_keys(repo_root), file_value)
}

fn evaluate_with_store(
    store: &TrustStore,
    keys: &[String],
    file_value: Option<&serde_json::Value>,
) -> TrustCheck {
    let gated = file_value.map(gated_fields).unwrap_or_default();
    let hash = gated_hash(&gated);
    let entries: Vec<&TrustedRepo> = keys.iter().filter_map(|k| store.repos.get(k)).collect();
    let trusted = if gated.is_empty() {
        // Nothing execution-capable in the file. Still require one explicit
        // first-open approval per repository so `--dangerously-skip-permissions`
        // stays off for repositories the user never trusted.
        !entries.is_empty()
    } else {
        entries.iter().any(|e| e.hashes.iter().any(|h| h == &hash))
    };
    let fields = gated
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::to_string_pretty(v).unwrap_or_default(),
            )
        })
        .collect();
    TrustCheck {
        trusted,
        hash,
        fields,
    }
}

/// Defense-in-depth check for execution sites (custom source commands,
/// runnable-cue auto-execution): whether the repository's on-disk
/// `.Dirigent/settings.json` is currently covered by a recorded user approval
/// in the user-profile trust store. [`load_settings`](super::io::load_settings)
/// already strips unapproved execution-capable fields at load time, but code
/// that spawns processes must not have to trust that every caller (and every
/// future caller) went through it — re-check here immediately before acting.
pub(crate) fn repo_settings_trusted(repo_root: &Path) -> bool {
    let path = repo_root.join(".Dirigent").join("settings.json");
    let file_value: Option<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok());
    evaluate(repo_root, file_value.as_ref()).trusted
}

/// Record the user's approval of an execution-capable field set for a repo.
pub(crate) fn record_approval(repo_root: &Path, hash: &str) {
    let Some(path) = store_path() else { return };
    record_approval_at(&path, &canonical_key(repo_root), hash);
}

/// Record the trust state matching an in-memory `Settings` the app is about to
/// persist. The app only ever saves settings that already passed the gate (or
/// were edited by the user in the UI), so what it writes is user-approved by
/// construction — without this, the app's own saves would re-trigger the prompt.
pub(crate) fn record_settings_state(repo_root: &Path, settings_value: &serde_json::Value) {
    let gated = gated_fields(settings_value);
    record_approval(repo_root, &gated_hash(&gated));
}

fn record_approval_at(store_path: &Path, key: &str, hash: &str) {
    let mut store = load_store(store_path);
    let entry = store.repos.entry(key.to_string()).or_default();
    if !entry.hashes.iter().any(|h| h == hash) {
        entry.hashes.push(hash.to_string());
        let len = entry.hashes.len();
        if len > 16 {
            entry.hashes.drain(..len - 16);
        }
    }
    entry.approved_at = chrono::Utc::now().to_rfc3339();
    save_store(store_path, &store);
}

// ---------------------------------------------------------------------------
// Fast LLM API-key → origin approvals (user profile)
// ---------------------------------------------------------------------------
//
// A repository's settings.json carries both `fast_llm_api_key` and
// `fast_llm_base_url`, so trusting a repository must not be enough to send a
// stored API key to whatever origin the file happens to name — a pulled change
// could redirect the key (and the diffs it authenticates) to an attacker's
// host. The key→origin binding is therefore approved separately, only through
// the settings UI, and remembered here in the user profile. Only a blake3
// fingerprint of the key is stored, never the key itself.

#[derive(Debug, Default, Serialize, Deserialize)]
struct KeyHostStore {
    /// blake3 fingerprint of an API key → origins (`scheme://host:port`) the
    /// user approved for that key.
    #[serde(default)]
    keys: BTreeMap<String, Vec<String>>,
}

/// `~/Library/Application Support/Dirigent/fast_llm_key_hosts.json`
fn key_host_store_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/Dirigent/fast_llm_key_hosts.json"))
}

fn key_fingerprint(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

fn load_key_host_store(path: &Path) -> KeyHostStore {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => KeyHostStore::default(),
    }
}

/// Whether the user has approved sending this API key to this origin. An empty
/// key needs no approval — there is no secret to protect.
pub(crate) fn fast_llm_key_host_approved(api_key: &str, origin: &str) -> bool {
    if api_key.is_empty() {
        return true;
    }
    let Some(path) = key_host_store_path() else {
        return false;
    };
    key_host_approved_at(&path, api_key, origin)
}

fn key_host_approved_at(store_path: &Path, api_key: &str, origin: &str) -> bool {
    load_key_host_store(store_path)
        .keys
        .get(&key_fingerprint(api_key))
        .is_some_and(|origins| origins.iter().any(|o| o == origin))
}

/// Record the user's approval — given through the settings UI, never through
/// repository trust — of sending `api_key` to `origin`.
pub(crate) fn approve_fast_llm_key_host(api_key: &str, origin: &str) {
    if api_key.is_empty() {
        return;
    }
    let Some(path) = key_host_store_path() else {
        return;
    };
    approve_fast_llm_key_host_at(&path, api_key, origin);
}

fn approve_fast_llm_key_host_at(store_path: &Path, api_key: &str, origin: &str) {
    let mut store = load_key_host_store(store_path);
    let origins = store.keys.entry(key_fingerprint(api_key)).or_default();
    if !origins.iter().any(|o| o == origin) {
        origins.push(origin.to_string());
        let len = origins.len();
        if len > 16 {
            origins.drain(..len - 16);
        }
    }
    if let Some(parent) = store_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&store)
        && let Err(e) = std::fs::write(store_path, json)
    {
        log::warn!("[trust] failed to write fast-llm key-host store: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    fn keys() -> Vec<String> {
        vec!["/repo".to_string()]
    }

    fn store_with(key: &str, hashes: &[&str]) -> TrustStore {
        let mut store = TrustStore::default();
        store.repos.insert(
            key.to_string(),
            TrustedRepo {
                hashes: hashes.iter().map(|h| h.to_string()).collect(),
                approved_at: String::new(),
            },
        );
        store
    }

    #[test]
    fn cosmetic_fields_are_safe_everything_else_is_gated() {
        assert!(is_repo_safe_field("theme"));
        assert!(is_repo_safe_field("font_size"));
        // Known execution-capable fields.
        for gated in [
            "lsp_servers",
            "sources",
            "claude_pre_run_script",
            "claude_extra_args",
            "commands",
            "playbook",
            "agents",
            "agent_shell_init",
            "fast_llm_base_url",
            "allow_home_folder_access",
            "allow_dangerous_skip_permissions",
            "claude_cli_path",
            "external_diff_tool",
            "dirigent_mcp_server_path",
            "acp_cli_path",
            "acp_extra_args",
            "acp_env_vars",
            "acp_pre_run_script",
        ] {
            assert!(!is_repo_safe_field(gated), "{gated} must be gated");
        }
        // Unknown / future fields are gated by default.
        assert!(!is_repo_safe_field("some_future_field"));
    }

    #[test]
    fn gated_fields_extracts_only_execution_capable_keys() {
        let value = obj(r#"{"theme": "Dark", "lsp_servers": [], "new_field": 1}"#);
        let gated = gated_fields(&value);
        assert_eq!(
            gated.keys().collect::<Vec<_>>(),
            vec!["lsp_servers", "new_field"]
        );
    }

    #[test]
    fn hash_is_deterministic_and_content_sensitive() {
        let a = gated_fields(&obj(r#"{"agent_shell_init": "source x"}"#));
        let b = gated_fields(&obj(r#"{"agent_shell_init": "source x"}"#));
        let c = gated_fields(&obj(r#"{"agent_shell_init": "curl evil"}"#));
        assert_eq!(gated_hash(&a), gated_hash(&b));
        assert_ne!(gated_hash(&a), gated_hash(&c));
    }

    #[test]
    fn first_open_is_untrusted_even_without_gated_fields() {
        let check = evaluate_with_store(&TrustStore::default(), &keys(), None);
        assert!(!check.trusted);
        assert!(check.fields.is_empty());

        let check = evaluate_with_store(
            &TrustStore::default(),
            &keys(),
            Some(&obj(r#"{"theme": "Dark"}"#)),
        );
        assert!(!check.trusted);
        assert!(check.fields.is_empty());
    }

    #[test]
    fn known_repo_without_gated_fields_is_trusted() {
        let store = store_with("/repo", &["whatever"]);
        let check = evaluate_with_store(&store, &keys(), Some(&obj(r#"{"theme": "Dark"}"#)));
        assert!(check.trusted);
    }

    #[test]
    fn gated_fields_require_matching_hash() {
        let value = obj(r#"{"claude_extra_args": "--mcp-config /tmp/evil.json"}"#);
        let hash = gated_hash(&gated_fields(&value));

        let check = evaluate_with_store(&store_with("/repo", &[&hash]), &keys(), Some(&value));
        assert!(check.trusted);

        let check =
            evaluate_with_store(&store_with("/repo", &["stale-hash"]), &keys(), Some(&value));
        assert!(!check.trusted);
        assert_eq!(check.fields.len(), 1);
        assert_eq!(check.fields[0].0, "claude_extra_args");
        assert!(check.fields[0].1.contains("--mcp-config"));
    }

    #[test]
    fn repo_settings_trusted_is_false_for_unapproved_repo() {
        // A fresh directory can never be in the user-profile trust store, so
        // the execution-site check must deny — with and without a settings
        // file carrying execution-capable fields.
        let dir = tempfile::tempdir().unwrap();
        assert!(!repo_settings_trusted(dir.path()));

        let dot = dir.path().join(".Dirigent");
        std::fs::create_dir_all(&dot).unwrap();
        std::fs::write(
            dot.join("settings.json"),
            r#"{"sources": [{"command": "sh .Dirigent/x.sh", "allow_runnable": true}]}"#,
        )
        .unwrap();
        assert!(!repo_settings_trusted(dir.path()));
    }

    #[test]
    fn key_host_approval_round_trips_and_is_origin_specific() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fast_llm_key_hosts.json");

        assert!(!key_host_approved_at(
            &path,
            "sk-secret",
            "https://api.openai.com:443"
        ));

        approve_fast_llm_key_host_at(&path, "sk-secret", "https://api.openai.com:443");
        assert!(key_host_approved_at(
            &path,
            "sk-secret",
            "https://api.openai.com:443"
        ));
        // A different origin for the same key is not covered.
        assert!(!key_host_approved_at(
            &path,
            "sk-secret",
            "https://evil.tld:443"
        ));
        // A different key for the approved origin is not covered.
        assert!(!key_host_approved_at(
            &path,
            "sk-other",
            "https://api.openai.com:443"
        ));
        // Only a fingerprint is stored, never the key itself.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("sk-secret"));
    }

    #[test]
    fn approval_round_trips_through_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_repos.json");
        let value = obj(r#"{"sources": [{"command": "evil"}]}"#);
        let hash = gated_hash(&gated_fields(&value));

        record_approval_at(&path, "/repo", &hash);
        let store = load_store(&path);
        let check = evaluate_with_store(&store, &keys(), Some(&value));
        assert!(check.trusted);

        // A different field set is not covered by the old approval.
        let other = obj(r#"{"sources": [{"command": "other"}]}"#);
        let check = evaluate_with_store(&store, &keys(), Some(&other));
        assert!(!check.trusted);
    }
}
