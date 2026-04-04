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

        // Now route parents into lanes.
        let parents = &commit.parent_hashes;

        if parents.is_empty() {
            // Root commit — just clear this lane.
            lanes[col] = None;
        } else {
            // First parent continues in the same lane.
            lanes[col] = Some(parents[0].clone());

            // Additional parents (merge commit) need their own lanes.
            for parent_hash in parents.iter().skip(1) {
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
        }

        // Close any duplicate lanes for the first parent (if another lane was also
        // waiting for the same hash, collapse it).
        if !parents.is_empty() {
            let first_parent = &parents[0];
            for i in 0..lanes.len() {
                if i != col && lanes[i].as_deref() == Some(first_parent) {
                    // This lane was also waiting for the same parent — merge it.
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
