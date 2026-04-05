mod archive;
mod commit;
mod diff;
mod history;
mod merge;
mod pr;
mod status;
mod worktree;

/// Shorten a full git hash to 7 characters.
fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

pub(crate) use archive::{archive_worktree_db, list_archived_dbs, ArchivedDb};
pub(crate) use commit::{
    commit_all, commit_diff, generate_commit_message, git_pull, git_push, revert_files,
    PullStrategy,
};
pub(crate) use diff::{get_working_diff, parse_diff_file_paths_for_repo};
pub(crate) use history::{count_commits, get_commit_diff, read_commit_history, CommitInfo};
pub(crate) use merge::{
    detect_merge_operation, get_conflicted_files, merge_abort, merge_continue, rebase_abort,
    rebase_continue, stage_files, MergeOperation,
};
pub(crate) use pr::{build_pr_body, create_pull_request, get_default_branch, main_worktree_path};
pub(crate) use status::{
    format_status_summary, get_ahead_of_remote, get_dirty_files, read_git_info, GitInfo,
};
pub(crate) use worktree::{
    checkout_branch, create_worktree, list_branches, list_worktrees, move_commits_to_branch,
    remove_worktree, WorktreeInfo,
};
