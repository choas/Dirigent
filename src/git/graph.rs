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

/// Allocate a lane for `hash`, reusing an empty slot or appending a new one.
fn allocate_lane(lanes: &mut Vec<Option<String>>, hash: String) -> usize {
    let col = lanes
        .iter()
        .position(|s| s.is_none())
        .unwrap_or(lanes.len());
    if col == lanes.len() {
        lanes.push(Some(hash));
    } else {
        lanes[col] = Some(hash);
    }
    col
}

/// Find the lane waiting for `hash`, or allocate a new one.
fn find_or_allocate_lane(lanes: &mut Vec<Option<String>>, hash: &str) -> usize {
    lanes
        .iter()
        .position(|slot| slot.as_deref() == Some(hash))
        .unwrap_or_else(|| allocate_lane(lanes, hash.to_string()))
}

/// Build initial lane segments: active lanes get Straight, commit lane gets Commit.
fn build_segments(lanes: &[Option<String>], col: usize) -> Vec<LaneSegment> {
    let mut segments = vec![LaneSegment::Empty; lanes.len()];
    for (i, slot) in lanes.iter().enumerate() {
        if slot.is_some() && i != col {
            segments[i] = LaneSegment::Straight;
        }
    }
    segments[col] = LaneSegment::Commit;
    segments
}

/// Collapse any lanes (other than `col`) that are waiting for `hash`.
/// Marks collapsed lanes as MergeLeft (unless the segment is Empty).
fn collapse_lanes_for(
    lanes: &mut [Option<String>],
    segments: &mut [LaneSegment],
    connections: &mut Vec<(usize, usize)>,
    col: usize,
    hash: &str,
) {
    for i in 0..lanes.len() {
        if i != col && lanes[i].as_deref() == Some(hash) {
            connections.push((i, col));
            if segments[i] != LaneSegment::Empty {
                segments[i] = LaneSegment::MergeLeft;
            }
            lanes[i] = None;
        }
    }
}

/// Route a single additional parent (not the first) into lanes.
fn route_additional_parent(
    lanes: &mut Vec<Option<String>>,
    segments: &mut Vec<LaneSegment>,
    connections: &mut Vec<(usize, usize)>,
    col: usize,
    parent_hash: &str,
) {
    let existing = lanes
        .iter()
        .position(|slot| slot.as_deref() == Some(parent_hash));

    if let Some(parent_lane) = existing {
        // The parent already has an active lane that continues past this row,
        // so keep it Straight. The (col, parent_lane) connection entry lets
        // paint_graph_connections draw the merge connector separately.
        connections.push((col, parent_lane));
    } else {
        let new_lane = allocate_lane(lanes, parent_hash.to_string());
        if new_lane >= segments.len() {
            segments.push(LaneSegment::ForkRight);
        } else {
            segments[new_lane] = LaneSegment::ForkRight;
        }
        connections.push((col, new_lane));
    }
}

/// Route all parents of a commit into lanes.
fn route_parents(
    lanes: &mut Vec<Option<String>>,
    segments: &mut Vec<LaneSegment>,
    connections: &mut Vec<(usize, usize)>,
    col: usize,
    parents: &[String],
) {
    if parents.is_empty() {
        lanes[col] = None;
        return;
    }

    lanes[col] = Some(parents[0].clone());

    for (pidx, parent_hash) in parents.iter().enumerate().skip(1) {
        if parents[..pidx].iter().any(|p| p == parent_hash) {
            continue;
        }
        route_additional_parent(lanes, segments, connections, col, parent_hash);
    }

    // Collapse duplicate lanes for the first parent.
    collapse_lanes_for(lanes, segments, connections, col, &parents[0]);
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

    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut max_lanes: usize = 0;

    for commit in commits {
        let col = find_or_allocate_lane(&mut lanes, &commit.full_hash);
        let mut segments = build_segments(&lanes, col);
        let mut connections: Vec<(usize, usize)> = Vec::new();

        collapse_lanes_for(
            &mut lanes,
            &mut segments,
            &mut connections,
            col,
            &commit.full_hash,
        );
        route_parents(
            &mut lanes,
            &mut segments,
            &mut connections,
            col,
            &commit.parent_hashes,
        );

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
