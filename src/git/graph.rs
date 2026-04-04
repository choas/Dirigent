use super::CommitInfo;

/// What a single lane does on a given row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaneSegment {
    /// Nothing in this lane on this row.
    Empty,
    /// A passing-through line (│).
    Straight,
    /// The commit sits in this lane (● with lines above/below).
    Commit,
    /// A new branch forks to the right from this lane (╲).
    ForkRight,
    /// A branch merges into this lane from the right (╱).
    MergeLeft,
}

/// Graph layout for a single row (one commit).
#[derive(Debug, Clone)]
pub(crate) struct GraphRow {
    /// Which lane column the commit dot sits in.
    pub column: usize,
    /// What each lane does on this row (length = max active lanes for this row).
    pub lanes: Vec<LaneSegment>,
    /// Connections: (from_lane, to_lane) pairs for diagonal lines on this row.
    pub connections: Vec<(usize, usize)>,
}

/// Compute the graph layout for a list of commits (in topological order, newest first).
///
/// Returns one `GraphRow` per commit, plus the maximum number of simultaneous lanes.
///
/// Handles edge cases: detached HEAD (orphan commits get their own lane),
/// octopus merges (3+ parents each get a lane), and orphan branches
/// (multiple disconnected histories coexist in separate lanes).
pub(crate) fn compute_graph(commits: &[CommitInfo]) -> (Vec<GraphRow>, usize) {
    if commits.is_empty() {
        return (Vec::new(), 0);
    }

    // Active lanes: each slot tracks the commit hash that lane is "waiting for".
    // When a commit appears whose hash matches a lane slot, it is placed there.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut max_lanes: usize = 0;

    for commit in commits {
        let hash = &commit.full_hash;

        // Find which lane this commit occupies (if any lane is waiting for it).
        let commit_lane = lanes.iter().position(|slot| slot.as_deref() == Some(hash));

        let col = if let Some(c) = commit_lane {
            c
        } else {
            // No lane waiting for this commit — allocate a new one.
            // This happens for the first commit, orphan branch roots,
            // and detached HEAD commits.
            let new_col = lanes
                .iter()
                .position(|s| s.is_none())
                .unwrap_or(lanes.len());
            if new_col == lanes.len() {
                lanes.push(Some(hash.clone()));
            } else {
                lanes[new_col] = Some(hash.clone());
            }
            new_col
        };

        // Build the lane segments for this row.
        let lane_count = lanes.len();
        let mut segments = vec![LaneSegment::Empty; lane_count];
        let mut connections: Vec<(usize, usize)> = Vec::new();

        // Mark all active lanes as straight-through (except the commit lane).
        for (i, slot) in lanes.iter().enumerate() {
            if slot.is_some() && i != col {
                segments[i] = LaneSegment::Straight;
            }
        }
        segments[col] = LaneSegment::Commit;

        // Collapse any OTHER lanes that were also waiting for this commit.
        // This happens when multiple child commits listed this commit as a parent,
        // creating duplicate lane reservations.
        for i in 0..lanes.len() {
            if i != col && lanes[i].as_deref() == Some(hash) {
                connections.push((i, col));
                segments[i] = LaneSegment::MergeLeft;
                lanes[i] = None;
            }
        }

        // Now route parents into lanes.
        let parents = &commit.parent_hashes;

        if parents.is_empty() {
            // Root commit (or orphan branch root) — clear this lane.
            lanes[col] = None;
        } else {
            // First parent continues in the same lane.
            lanes[col] = Some(parents[0].clone());

            // Additional parents (merge/octopus) need their own lanes.
            for (pidx, parent_hash) in parents.iter().enumerate().skip(1) {
                // Skip duplicate parent hashes (degenerate octopus case).
                if parents[..pidx].iter().any(|p| p == parent_hash) {
                    continue;
                }

                // Check if parent is already in an existing lane.
                let existing = lanes
                    .iter()
                    .position(|slot| slot.as_deref() == Some(parent_hash.as_str()));

                if let Some(parent_lane) = existing {
                    // Parent already tracked — draw a merge line.
                    connections.push((col, parent_lane));
                    if segments[parent_lane] == LaneSegment::Straight {
                        segments[parent_lane] = LaneSegment::MergeLeft;
                    }
                } else {
                    // Allocate a new lane for this parent.
                    let new_lane = lanes
                        .iter()
                        .position(|s| s.is_none())
                        .unwrap_or(lanes.len());
                    if new_lane == lanes.len() {
                        lanes.push(Some(parent_hash.clone()));
                        segments.push(LaneSegment::ForkRight);
                    } else {
                        lanes[new_lane] = Some(parent_hash.clone());
                        segments[new_lane] = LaneSegment::ForkRight;
                    }
                    connections.push((col, new_lane));
                }
            }

            // Collapse duplicate lanes for the first parent (if another lane was also
            // waiting for the same hash).
            let first_parent = &parents[0];
            for i in 0..lanes.len() {
                if i != col && lanes[i].as_deref() == Some(first_parent.as_str()) {
                    connections.push((i, col));
                    if segments[i] != LaneSegment::Empty {
                        segments[i] = LaneSegment::MergeLeft;
                    }
                    lanes[i] = None;
                }
            }
        }

        // Trim trailing empty lanes.
        while lanes.last() == Some(&None) {
            lanes.pop();
        }

        max_lanes = max_lanes.max(segments.len());
        rows.push(GraphRow {
            column: col,
            lanes: segments,
            connections,
        });
    }

    (rows, max_lanes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::CommitInfo;

    fn make_commit(hash: &str, parents: &[&str]) -> CommitInfo {
        CommitInfo {
            full_hash: hash.to_string(),
            short_hash: hash[..7.min(hash.len())].to_string(),
            message: format!("commit {hash}"),
            body: String::new(),
            author: "test".to_string(),
            time_ago: "now".to_string(),
            parent_hashes: parents.iter().map(|s| s.to_string()).collect(),
            branch_labels: Vec::new(),
            tag_labels: Vec::new(),
            is_merge: parents.len() > 1,
        }
    }

    #[test]
    fn empty_input() {
        let (rows, max) = compute_graph(&[]);
        assert!(rows.is_empty());
        assert_eq!(max, 0);
    }

    #[test]
    fn linear_history() {
        let commits = vec![
            make_commit("aaa", &["bbb"]),
            make_commit("bbb", &["ccc"]),
            make_commit("ccc", &[]),
        ];
        let (rows, max) = compute_graph(&commits);
        assert_eq!(rows.len(), 3);
        // All commits should be in lane 0.
        for row in &rows {
            assert_eq!(row.column, 0);
        }
        assert_eq!(max, 1);
        // Root commit clears its lane.
        assert_eq!(rows[2].lanes[0], LaneSegment::Commit);
    }

    #[test]
    fn simple_merge() {
        // C merges A and B; A is first parent, B is second.
        let commits = vec![
            make_commit("merge", &["aaa", "bbb"]),
            make_commit("aaa", &["root"]),
            make_commit("bbb", &["root"]),
            make_commit("root", &[]),
        ];
        let (rows, max) = compute_graph(&commits);
        assert_eq!(rows.len(), 4);
        // Merge commit should fork into 2 lanes.
        assert!(max >= 2);
        assert_eq!(rows[0].column, 0);
        // bbb should be in a different lane than aaa.
        assert_ne!(rows[1].column, rows[2].column);
    }

    #[test]
    fn octopus_merge_three_parents() {
        let commits = vec![
            make_commit("oct", &["p1", "p2", "p3"]),
            make_commit("p1", &["root"]),
            make_commit("p2", &["root"]),
            make_commit("p3", &["root"]),
            make_commit("root", &[]),
        ];
        let (rows, max) = compute_graph(&commits);
        assert_eq!(rows.len(), 5);
        // Octopus merge should create 3 lanes (one per parent).
        assert!(max >= 3, "expected at least 3 lanes, got {max}");
        // All three parents should be in distinct columns.
        let parent_cols: Vec<usize> = rows[1..4].iter().map(|r| r.column).collect();
        assert_ne!(parent_cols[0], parent_cols[1]);
        assert_ne!(parent_cols[0], parent_cols[2]);
        assert_ne!(parent_cols[1], parent_cols[2]);
    }

    #[test]
    fn octopus_merge_duplicate_parents() {
        // Degenerate: same parent listed twice — should not allocate two lanes.
        let commits = vec![
            make_commit("oct", &["p1", "p2", "p2"]),
            make_commit("p1", &["root"]),
            make_commit("p2", &["root"]),
            make_commit("root", &[]),
        ];
        let (rows, _max) = compute_graph(&commits);
        assert_eq!(rows.len(), 4);
        // The merge row should have exactly one connection for p2, not two.
        let merge_conns = &rows[0].connections;
        let p2_conns: Vec<_> = merge_conns.iter().filter(|(_, to)| *to != 0).collect();
        // Should be exactly 1 fork for p2 (dedup prevents double allocation).
        assert_eq!(p2_conns.len(), 1);
    }

    #[test]
    fn orphan_branches() {
        // Two disconnected histories interleaved by time.
        let commits = vec![
            make_commit("a1", &["a2"]),
            make_commit("b1", &["b2"]),
            make_commit("a2", &[]),
            make_commit("b2", &[]),
        ];
        let (rows, max) = compute_graph(&commits);
        assert_eq!(rows.len(), 4);
        // Should use 2 lanes — one per branch.
        assert!(max >= 2);
        // a1 and b1 should be in different lanes.
        assert_ne!(rows[0].column, rows[1].column);
        // a2 follows a1's lane, b2 follows b1's lane.
        assert_eq!(rows[0].column, rows[2].column);
        assert_eq!(rows[1].column, rows[3].column);
    }

    #[test]
    fn convergent_branches_collapse_duplicate_lanes() {
        // Two branches (child1, child2) both have the same parent.
        // The duplicate lane for "parent" is collapsed at child2's row
        // (first-parent duplicate collapse), so parent lands in child2's lane.
        let commits = vec![
            make_commit("child1", &["parent"]),
            make_commit("child2", &["parent"]),
            make_commit("parent", &["root"]),
            make_commit("root", &[]),
        ];
        let (rows, _max) = compute_graph(&commits);
        assert_eq!(rows.len(), 4);
        // child1 in lane 0, child2 in lane 1.
        assert_eq!(rows[0].column, 0);
        assert_eq!(rows[1].column, 1);
        // At child2's row, lane 0 (also pointing to "parent") is collapsed
        // into lane 1, so parent arrives in lane 1.
        assert_eq!(rows[2].column, 1);
        // child2's row should show the merge-left at lane 0.
        assert_eq!(rows[1].lanes[0], LaneSegment::MergeLeft);
        // root continues in lane 1, lane 0 is free.
        assert_eq!(rows[3].column, 1);
    }

    #[test]
    fn root_commit_no_dangling_lanes() {
        // Two children both point to the same root.
        // The duplicate lane is collapsed at c2's row.
        let commits = vec![
            make_commit("c1", &["root"]),
            make_commit("c2", &["root"]),
            make_commit("root", &[]),
        ];
        let (rows, _max) = compute_graph(&commits);
        assert_eq!(rows.len(), 3);
        // c2's row collapses the duplicate "root" lane.
        assert_eq!(rows[1].column, 1);
        let has_merge_at_c2 = rows[1].lanes.iter().any(|s| *s == LaneSegment::MergeLeft);
        assert!(has_merge_at_c2, "duplicate lane should merge at c2's row");
        // root lands in lane 1 (after collapse).
        assert_eq!(rows[2].column, 1);
        // root has no parents, so its lane is cleared — only 1 lane remains
        // (or the lane is empty).
    }
}
