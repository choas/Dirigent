mod commit;
mod diff;
mod history;
mod status;
mod worktree;

pub(crate) use commit::{
    jj_commit_all, jj_commit_diff, jj_create_bookmark, jj_delete_bookmark, jj_pull, jj_push,
    jj_revert_files, jj_set_bookmark, jj_squash_bookmark, jj_undo,
};
pub(crate) use diff::jj_get_working_diff;
pub(crate) use history::{jj_count_commits, jj_get_commit_diff, jj_read_commit_history};
pub(crate) use status::{jj_get_ahead_of_remote, jj_get_dirty_files, jj_read_info};
pub(crate) use worktree::{
    cue_bookmark_name, cue_workspace_name, jj_checkout_bookmark, jj_create_workspace,
    jj_list_bookmarks, jj_list_workspaces, jj_remove_workspace,
};

fn jj_cmd(jj_path: &str) -> std::process::Command {
    if jj_path.is_empty() {
        std::process::Command::new("jj")
    } else {
        std::process::Command::new(jj_path)
    }
}
