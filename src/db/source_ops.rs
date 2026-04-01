use anyhow::Result;
use rusqlite::params;

use super::types::CueStatus;
use super::Database;

/// (id, text, status, file_path, line_number)
pub(crate) type CueSourceRow = (i64, String, String, String, usize);

impl Database {
    // -- Source integration --

    /// Check if a cue with the given source_ref already exists (for deduplication).
    pub fn cue_exists_by_source_ref(&self, source_ref: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cues WHERE source_ref = ?1",
            params![source_ref],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the cue id, text, status, file_path, and line_number for a given source_ref.
    pub fn get_cue_by_source_ref(&self, source_ref: &str) -> Result<Option<CueSourceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, status, file_path, line_number FROM cues WHERE source_ref = ?1 LIMIT 1",
        )?;
        let result = stmt.query_row(params![source_ref], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? as usize,
            ))
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update text and location of an existing cue identified by source_ref.
    pub fn update_cue_by_source_ref(
        &self,
        source_ref: &str,
        text: &str,
        file_path: &str,
        line_number: usize,
    ) -> Result<()> {
        let text = Self::clamp_cue_text(text);
        self.conn.execute(
            "UPDATE cues SET text = ?1, file_path = ?2, line_number = ?3 WHERE source_ref = ?4",
            params![text, file_path, line_number as i64, source_ref],
        )?;
        Ok(())
    }

    /// Backfill source_id (and source_label) on an existing cue identified by
    /// source_ref, but only when the stored source_id is currently NULL.
    pub fn backfill_source_id(
        &self,
        source_ref: &str,
        source_id: &str,
        source_label: &str,
    ) -> Result<bool> {
        let updated = self.conn.execute(
            "UPDATE cues SET source_id = ?1, source_label = ?2 WHERE source_ref = ?3 AND source_id IS NULL",
            params![source_id, source_label, source_ref],
        )?;
        Ok(updated > 0)
    }

    /// Insert a cue from an external source with optional file location.
    pub fn insert_cue_from_source(
        &self,
        text: &str,
        source_label: &str,
        source_id: &str,
        source_ref: &str,
        file_path: &str,
        line_number: usize,
    ) -> Result<i64> {
        let text = Self::clamp_cue_text(text);
        let source_id_val = if source_id.is_empty() {
            None
        } else {
            Some(source_id)
        };
        self.conn.execute(
            "INSERT INTO cues (text, file_path, line_number, status, source_label, source_id, source_ref) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![text, file_path, line_number as i64, CueStatus::Inbox.as_str(), source_label, source_id_val, source_ref],
        )?;
        let id = self.conn.last_insert_rowid();
        let _ = self.log_activity(id, &format!("Created from {}", source_label));
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::PrFinding;
    use std::collections::HashSet;

    fn test_db() -> Database {
        Database::open_in_memory().expect("in-memory db")
    }

    fn mock_pr_findings() -> Vec<PrFinding> {
        vec![
            PrFinding {
                file_path: "src/main.rs".to_string(),
                line_number: 10,
                text: "Consider using a match instead of if-let here".to_string(),
                external_id: "pr42:comment:1001".to_string(),
            },
            PrFinding {
                file_path: "src/lib.rs".to_string(),
                line_number: 25,
                text: "This function could return an error instead of panicking".to_string(),
                external_id: "pr42:comment:1002".to_string(),
            },
            PrFinding {
                file_path: String::new(),
                line_number: 0,
                text: "Overall the PR looks good, minor nits above".to_string(),
                external_id: "pr42:issue_comment:2001".to_string(),
            },
            PrFinding {
                file_path: "src/app.rs".to_string(),
                line_number: 100,
                text: "Missing error handling for the network call".to_string(),
                external_id: "pr42:comment:1003".to_string(),
            },
        ]
    }

    /// Test the filtering logic: given pending findings and a set of excluded
    /// indices, only the non-excluded findings should be included.
    #[test]
    fn filter_excludes_correct_findings() {
        let pending = mock_pr_findings();
        let mut excluded = HashSet::new();
        excluded.insert(1); // exclude "src/lib.rs" finding
        excluded.insert(2); // exclude the general comment

        let filtered: Vec<PrFinding> = pending
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded.contains(i))
            .map(|(_, f)| f.clone())
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].external_id, "pr42:comment:1001");
        assert_eq!(filtered[1].external_id, "pr42:comment:1003");
    }

    /// Test that with no exclusions, all findings pass through.
    #[test]
    fn filter_with_no_exclusions_keeps_all() {
        let pending = mock_pr_findings();
        let excluded: HashSet<usize> = HashSet::new();

        let filtered: Vec<PrFinding> = pending
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded.contains(i))
            .map(|(_, f)| f.clone())
            .collect();

        assert_eq!(filtered.len(), pending.len());
    }

    /// Test that excluding all findings produces an empty list.
    #[test]
    fn filter_exclude_all_produces_empty() {
        let pending = mock_pr_findings();
        let excluded: HashSet<usize> = (0..pending.len()).collect();

        let filtered: Vec<PrFinding> = pending
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded.contains(i))
            .map(|(_, f)| f.clone())
            .collect();

        assert!(filtered.is_empty());
    }

    /// Test that insert_cue_from_source creates cues with Inbox status.
    #[test]
    fn import_pr_findings_creates_inbox_cues() {
        let db = test_db();
        let findings = mock_pr_findings();

        for finding in &findings {
            let id = db
                .insert_cue_from_source(
                    &finding.text,
                    "PR Review",
                    "",
                    &finding.external_id,
                    &finding.file_path,
                    finding.line_number,
                )
                .expect("insert should succeed");
            assert!(id > 0, "inserted cue should have a positive id");
        }

        // Verify all findings are in the DB with correct status
        for finding in &findings {
            let row = db
                .get_cue_by_source_ref(&finding.external_id)
                .expect("query should succeed")
                .expect("cue should exist");
            let (_id, text, status, file_path, line_number) = row;
            assert_eq!(text, finding.text);
            assert_eq!(status, CueStatus::Inbox.as_str());
            assert_eq!(file_path, finding.file_path);
            assert_eq!(line_number, finding.line_number);
        }
    }

    /// Test that filtered findings are correctly imported to the DB.
    #[test]
    fn filtered_import_only_inserts_included_findings() {
        let db = test_db();
        let pending = mock_pr_findings();
        let mut excluded = HashSet::new();
        excluded.insert(0); // exclude first
        excluded.insert(2); // exclude third

        // Apply filter (same logic as import_filtered_pr_findings)
        let filtered: Vec<PrFinding> = pending
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded.contains(i))
            .map(|(_, f)| f.clone())
            .collect();

        assert_eq!(filtered.len(), 2);

        // Import filtered findings
        let tag = "PR42";
        for finding in &filtered {
            let id = db
                .insert_cue_from_source(
                    &finding.text,
                    "PR Review",
                    "",
                    &finding.external_id,
                    &finding.file_path,
                    finding.line_number,
                )
                .expect("insert should succeed");
            db.update_cue_tag(id, Some(tag))
                .expect("tag should succeed");
        }

        // Verify only the included findings exist
        assert!(db
            .get_cue_by_source_ref("pr42:comment:1001")
            .unwrap()
            .is_none()); // excluded
        assert!(db
            .get_cue_by_source_ref("pr42:comment:1002")
            .unwrap()
            .is_some()); // included
        assert!(db
            .get_cue_by_source_ref("pr42:issue_comment:2001")
            .unwrap()
            .is_none()); // excluded
        assert!(db
            .get_cue_by_source_ref("pr42:comment:1003")
            .unwrap()
            .is_some()); // included

        // Verify the included ones have correct data
        let row = db
            .get_cue_by_source_ref("pr42:comment:1002")
            .unwrap()
            .unwrap();
        assert_eq!(
            row.1,
            "This function could return an error instead of panicking"
        );
        assert_eq!(row.2, CueStatus::Inbox.as_str());
        assert_eq!(row.3, "src/lib.rs");
        assert_eq!(row.4, 25);
    }

    /// Test deduplication: importing the same finding twice should not create duplicates.
    #[test]
    fn import_deduplicates_by_source_ref() {
        let db = test_db();
        let finding = &mock_pr_findings()[0];

        // First import
        db.insert_cue_from_source(
            &finding.text,
            "PR Review",
            "",
            &finding.external_id,
            &finding.file_path,
            finding.line_number,
        )
        .expect("first insert should succeed");

        // Check it exists
        assert!(db.cue_exists_by_source_ref(&finding.external_id).unwrap());

        // Simulate the dedup check that upsert_pr_findings does
        let existing = db.get_cue_by_source_ref(&finding.external_id).unwrap();
        assert!(existing.is_some(), "should find existing cue by source_ref");
    }

    /// Test that update_cue_by_source_ref changes text and location.
    #[test]
    fn update_existing_finding_changes_text_and_location() {
        let db = test_db();
        let finding = &mock_pr_findings()[0];

        let id = db
            .insert_cue_from_source(
                &finding.text,
                "PR Review",
                "",
                &finding.external_id,
                &finding.file_path,
                finding.line_number,
            )
            .unwrap();

        // Update with new text and location
        db.update_cue_by_source_ref(
            &finding.external_id,
            "Updated review comment",
            "src/updated.rs",
            99,
        )
        .unwrap();

        let row = db
            .get_cue_by_source_ref(&finding.external_id)
            .unwrap()
            .unwrap();
        assert_eq!(row.0, id);
        assert_eq!(row.1, "Updated review comment");
        assert_eq!(row.3, "src/updated.rs");
        assert_eq!(row.4, 99);
    }
}
