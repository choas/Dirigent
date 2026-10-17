mod archive;
mod commit;
mod diff;
pub(crate) mod forgejo;
pub(crate) mod graph;
mod history;
mod merge;
mod pr;
mod staging;
mod status;
mod worktree;

use git2::{Repository, StatusOptions, Statuses};

/// Build a `git` command with a deterministic, English output locale.
///
/// Dirigent classifies git failures by matching English substrings of stderr
/// ("CONFLICT", "diverged", "unmerged files", ...). Under a non-English locale
/// git translates those messages and every one of those checks silently stops
/// matching — the conflict dialog never opens, retries never fire, and the user
/// just sees a generic failure. Pinning `LC_ALL`/`LANG` makes the matching
/// reliable regardless of the user's environment.
///
/// Use this instead of `Command::new("git")` everywhere.
pub(crate) fn git_cmd() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    force_c_locale(&mut cmd);
    cmd
}

/// Pin a child process to the C locale. `LC_ALL` wins over every other
/// category; `LANG` and `LANGUAGE` are cleared too so gettext cannot fall back
/// to a translated catalog.
pub(crate) fn force_c_locale(cmd: &mut std::process::Command) {
    cmd.env("LC_ALL", "C").env("LANG", "C").env("LANGUAGE", "");
}

/// Shorten a full git hash to 7 characters.
fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

/// Collect git statuses including untracked files (the common case).
fn collect_statuses(repo: &Repository) -> Option<Statuses<'_>> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    repo.statuses(Some(&mut opts)).ok()
}

pub(crate) use archive::{ArchivedDb, archive_worktree_db, list_archived_dbs};
pub(crate) use commit::{
    DIRIGENT_FOOTER, PullStrategy, commit_all, commit_diff, commit_partial, detect_commit_type,
    generate_commit_message, git_pull, git_push, move_to_new_branch, revert_files,
};
pub(crate) use diff::{
    FolderCompare, compare_folders, get_uncommitted_diff, get_working_diff,
    parse_diff_file_paths_for_repo, try_get_range_diff,
};
pub(crate) use history::{
    CommitInfo, count_commits, get_commit_diff, get_commit_message, read_commit_history,
};
pub(crate) use merge::{
    MergeOperation, detect_merge_operation, get_conflicted_files, merge_abort, merge_continue,
    rebase_abort, rebase_continue, stage_files,
};
pub(crate) use pr::{build_pr_body, create_pull_request, get_default_branch, main_worktree_path};
pub(crate) use staging::{
    FileRawDiff, build_hunk_patch, discard_hunk, get_staged_diff, match_hunk_header,
    partially_staged_files, snapshot_for_undo, split_into_file_diffs, stage_hunk, unstage_hunk,
};
pub(crate) use status::{
    GitInfo, format_status_summary, get_ahead_of_remote, get_dirty_files, read_git_info,
};
pub(crate) use worktree::{
    WorktreeInfo, checkout_branch, create_branch, create_worktree, list_all_branches,
    list_branches, list_worktrees, own_branches, prune_worktrees, remove_worktree,
};

#[cfg(test)]
mod locale_tests {
    use super::*;

    /// Every `git` invocation must be locale-pinned: the app's conflict /
    /// diverged / unmerged detection matches English stderr substrings.
    #[test]
    fn git_cmd_forces_c_locale() {
        let cmd = git_cmd();
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            envs.contains(&("LC_ALL".to_string(), Some("C".to_string()))),
            "LC_ALL not pinned: {envs:?}"
        );
        assert!(
            envs.contains(&("LANG".to_string(), Some("C".to_string()))),
            "LANG not pinned: {envs:?}"
        );
        assert!(
            envs.contains(&("LANGUAGE".to_string(), Some(String::new()))),
            "LANGUAGE not cleared: {envs:?}"
        );
    }

    /// Guard against new `Command::new("git")` call sites creeping back in and
    /// bypassing the locale pin.
    #[test]
    fn no_raw_git_command_construction_in_git_module() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("read src/git") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "rs")
                || path.file_name().is_some_and(|n| n == "mod.rs")
            {
                continue;
            }
            let contents = std::fs::read_to_string(&path).expect("read source");
            if contents.contains(r#"Command::new("git")"#) {
                offenders.push(path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "use git_cmd() instead of Command::new(\"git\") in: {offenders:?}"
        );
    }
}
