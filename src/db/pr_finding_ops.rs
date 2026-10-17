//! Importing PR review findings as cues.
//!
//! This is pure database work — dedup by source ref, decide update-vs-insert,
//! reset re-opened cues to Inbox, tag them with the PR number. It lived in
//! `app/git_operations.rs`, where it was ~120 lines of `self.db.*` calls
//! wedged into an already-oversized UI module and could only be exercised by
//! running the app against a real pull request. Here it takes a `&Database`
//! and is testable directly.

use anyhow::Result;

use super::Database;
use super::types::CueStatus;
use crate::sources::PrFinding;

/// Outcome of importing a batch of findings.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrImportCounts {
    /// Findings that became new cues.
    pub new: usize,
    /// Existing cues whose text or location changed and were reset to Inbox.
    pub updated: usize,
    /// Findings that could not be written. The import continues past these:
    /// one bad row must not abandon the rest of a review.
    pub errors: usize,
}

impl PrImportCounts {
    pub fn has_changes(&self) -> bool {
        self.new > 0 || self.updated > 0
    }

    /// Human-readable summary, e.g. `"3 new, 1 updated finding(s)"`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.new > 0 {
            parts.push(format!("{} new", self.new));
        }
        if self.updated > 0 {
            parts.push(format!("{} updated", self.updated));
        }
        format!("{} finding(s)", parts.join(", "))
    }
}

/// What happened to one already-imported finding.
#[derive(Debug, PartialEq, Eq)]
enum ExistingOutcome {
    /// Text and location are unchanged; the cue is left exactly as it is —
    /// including a Done or Archived one, which must not be re-opened just
    /// because the reviewer's comment was fetched again.
    Unchanged,
    /// The reviewer changed the comment or moved it; the cue was updated and
    /// reset to Inbox so the change is seen.
    Updated,
}

impl Database {
    /// Import PR review findings as cues, tagging each with `tag`.
    ///
    /// Existing cues are matched by `source_ref` and only rewritten when the
    /// finding's text or location actually changed.
    pub(crate) fn import_pr_findings(&self, findings: &[PrFinding], tag: &str) -> PrImportCounts {
        let mut counts = PrImportCounts::default();
        for finding in findings {
            match self.get_cue_by_source_ref(&finding.external_id) {
                Ok(Some((existing_id, existing_text, _status, existing_path, existing_line))) => {
                    match self.sync_existing_finding(
                        finding,
                        existing_id,
                        &existing_text,
                        &existing_path,
                        existing_line,
                    ) {
                        Ok(ExistingOutcome::Updated) => counts.updated += 1,
                        Ok(ExistingOutcome::Unchanged) => {}
                        Err(e) => {
                            log::error!("DB error updating finding {}: {e}", finding.external_id);
                            counts.errors += 1;
                        }
                    }
                }
                Ok(None) => match self.insert_new_finding(finding, tag) {
                    Ok(()) => counts.new += 1,
                    Err(e) => {
                        log::error!("DB error inserting finding {}: {e}", finding.external_id);
                        counts.errors += 1;
                    }
                },
                Err(e) => {
                    log::error!(
                        "DB error looking up source ref {}: {e}",
                        finding.external_id
                    );
                    counts.errors += 1;
                }
            }
        }
        counts
    }

    fn insert_new_finding(&self, finding: &PrFinding, tag: &str) -> Result<()> {
        let id = self.insert_cue_from_source(
            &finding.text,
            "PR Review",
            "",
            &finding.external_id,
            &finding.file_path,
            finding.line_number,
        )?;
        self.update_cue_tag(id, Some(tag))?;
        Ok(())
    }

    /// Update an already-imported finding when the reviewer's comment or its
    /// location changed. HTML markup and Dirigent's own context hint are
    /// stripped before comparing, so re-rendering alone is not a "change".
    fn sync_existing_finding(
        &self,
        finding: &PrFinding,
        existing_id: i64,
        existing_text: &str,
        existing_path: &str,
        existing_line: usize,
    ) -> Result<ExistingOutcome> {
        let clean_existing = crate::sources::strip_html_tags(existing_text);
        let clean_finding = crate::sources::strip_html_tags(&finding.text);
        let text_changed = crate::sources::strip_pr_context_hint(&clean_existing)
            != crate::sources::strip_pr_context_hint(&clean_finding);
        let location_changed =
            existing_path != finding.file_path || existing_line != finding.line_number;
        if !text_changed && !location_changed {
            return Ok(ExistingOutcome::Unchanged);
        }
        self.update_cue_by_source_ref(
            &finding.external_id,
            &finding.text,
            &finding.file_path,
            finding.line_number,
        )?;
        self.update_cue_status(existing_id, CueStatus::Inbox)?;
        self.log_activity(existing_id, "PR comment updated, reset to Inbox")?;
        Ok(ExistingOutcome::Updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str, text: &str, path: &str, line: usize) -> PrFinding {
        PrFinding {
            external_id: id.to_string(),
            text: text.to_string(),
            file_path: path.to_string(),
            line_number: line,
        }
    }

    fn db() -> Database {
        Database::open_in_memory().expect("in-memory db")
    }

    #[test]
    fn imports_new_findings_and_tags_them() {
        let db = db();
        let counts = db.import_pr_findings(
            &[
                finding("c1", "Use a const here", "src/a.rs", 10),
                finding("c2", "Missing test", "src/b.rs", 20),
            ],
            "PR42",
        );

        assert_eq!(
            counts,
            PrImportCounts {
                new: 2,
                updated: 0,
                errors: 0
            }
        );
        let cues = db.all_cues_limited_archived(100).expect("cues");
        assert_eq!(cues.len(), 2);
        assert!(cues.iter().all(|c| c.tag.as_deref() == Some("PR42")));
    }

    /// Re-importing the same review must be a no-op, not a duplicate.
    #[test]
    fn reimporting_unchanged_findings_changes_nothing() {
        let db = db();
        let batch = [finding("c1", "Use a const here", "src/a.rs", 10)];
        db.import_pr_findings(&batch, "PR42");

        let counts = db.import_pr_findings(&batch, "PR42");

        assert_eq!(counts, PrImportCounts::default());
        assert_eq!(db.all_cues_limited_archived(100).expect("cues").len(), 1);
    }

    #[test]
    fn changed_text_updates_the_cue_and_resets_it_to_inbox() {
        let db = db();
        db.import_pr_findings(&[finding("c1", "Use a const", "src/a.rs", 10)], "PR42");
        let id = db.all_cues_limited_archived(100).expect("cues")[0].id;
        db.update_cue_status(id, CueStatus::Done)
            .expect("mark done");

        let counts = db.import_pr_findings(
            &[finding("c1", "Actually, inline it", "src/a.rs", 10)],
            "PR42",
        );

        assert_eq!(counts.updated, 1);
        assert_eq!(counts.new, 0);
        let cue = &db.all_cues_limited_archived(100).expect("cues")[0];
        assert_eq!(cue.status, CueStatus::Inbox);
        assert_eq!(cue.text, "Actually, inline it");
    }

    #[test]
    fn a_moved_comment_updates_the_location() {
        let db = db();
        db.import_pr_findings(&[finding("c1", "Same text", "src/a.rs", 10)], "PR42");

        let counts = db.import_pr_findings(&[finding("c1", "Same text", "src/a.rs", 44)], "PR42");

        assert_eq!(counts.updated, 1);
        let cue = &db.all_cues_limited_archived(100).expect("cues")[0];
        assert_eq!(cue.line_number, 44);
    }

    /// A Done cue whose finding is unchanged stays Done — fetching a review
    /// again must not reopen work the user already finished.
    #[test]
    fn unchanged_finding_does_not_reopen_a_done_cue() {
        let db = db();
        db.import_pr_findings(&[finding("c1", "Use a const", "src/a.rs", 10)], "PR42");
        let id = db.all_cues_limited_archived(100).expect("cues")[0].id;
        db.update_cue_status(id, CueStatus::Done)
            .expect("mark done");

        db.import_pr_findings(&[finding("c1", "Use a const", "src/a.rs", 10)], "PR42");

        assert_eq!(
            db.all_cues_limited_archived(100).expect("cues")[0].status,
            CueStatus::Done
        );
    }

    #[test]
    fn summary_reads_naturally_for_each_combination() {
        assert_eq!(
            PrImportCounts {
                new: 3,
                updated: 0,
                errors: 0
            }
            .summary(),
            "3 new finding(s)"
        );
        assert_eq!(
            PrImportCounts {
                new: 3,
                updated: 1,
                errors: 0
            }
            .summary(),
            "3 new, 1 updated finding(s)"
        );
        assert!(!PrImportCounts::default().has_changes());
    }
}
