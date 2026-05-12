use std::collections::HashMap;
use std::path::Path;

use crate::git;
use crate::jj;
use crate::settings::VcsBackend;

/// VCS-backend-aware dispatch for working-tree diff.
pub(super) fn get_working_diff(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
    files: &[String],
) -> Option<String> {
    match backend {
        VcsBackend::Jj => jj::jj_get_working_diff(repo_path, files, jj_path),
        VcsBackend::Git => git::get_working_diff(repo_path, files),
    }
}

/// VCS-backend-aware dispatch for commit all.
pub(super) fn commit_all(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
    commit_message: &str,
) -> crate::error::Result<String> {
    match backend {
        VcsBackend::Jj => jj::jj_commit_all(repo_path, commit_message, jj_path),
        VcsBackend::Git => git::commit_all(repo_path, commit_message),
    }
}

/// VCS-backend-aware dispatch for commit diff.
pub(super) fn commit_diff(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
) -> crate::error::Result<String> {
    match backend {
        VcsBackend::Jj => jj::jj_commit_diff(repo_path, diff_text, commit_message, jj_path),
        VcsBackend::Git => git::commit_diff(repo_path, diff_text, commit_message),
    }
}

/// VCS-backend-aware dispatch for revert files.
pub(super) fn revert_files(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
    file_paths: &[String],
) -> crate::error::Result<()> {
    match backend {
        VcsBackend::Jj => jj::jj_revert_files(repo_path, file_paths, jj_path),
        VcsBackend::Git => git::revert_files(repo_path, file_paths),
    }
}

/// VCS-backend-aware dispatch for commit diff lookup.
pub(super) fn get_commit_diff(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
    commit_hash: &str,
) -> Option<String> {
    match backend {
        VcsBackend::Jj => jj::jj_get_commit_diff(repo_path, commit_hash, jj_path),
        VcsBackend::Git => git::get_commit_diff(repo_path, commit_hash),
    }
}

/// VCS-backend-aware dispatch for dirty files.
pub(super) fn get_dirty_files(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
) -> HashMap<String, char> {
    match backend {
        VcsBackend::Jj => jj::jj_get_dirty_files(repo_path, jj_path),
        VcsBackend::Git => git::get_dirty_files(repo_path),
    }
}

/// VCS-backend-aware dispatch for ahead-of-remote count.
pub(super) fn get_ahead_of_remote(backend: &VcsBackend, jj_path: &str, repo_path: &Path) -> usize {
    match backend {
        VcsBackend::Jj => jj::jj_get_ahead_of_remote(repo_path, jj_path),
        VcsBackend::Git => git::get_ahead_of_remote(repo_path),
    }
}

/// VCS-backend-aware dispatch for listing branches/bookmarks.
pub(super) fn list_branches(
    backend: &VcsBackend,
    jj_path: &str,
    repo_path: &Path,
) -> crate::error::Result<Vec<String>> {
    match backend {
        VcsBackend::Jj => jj::jj_list_bookmarks(repo_path, jj_path),
        VcsBackend::Git => git::list_branches(repo_path),
    }
}
