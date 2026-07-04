# Git Workflows

What's possible with Dirigent's Git integration: AI-assisted commit splitting, hunk-level staging, cross-branch review, and worktree tooling.

The flagship capability: Dirigent can untangle a messy working tree into clean, per-feature commits — the AI proposes the grouping, and you confirm every commit. Around that, this covers hunk-level staging, partial (per-hunk) commits, cross-branch change-set review, worktree robustness, and folder comparison.

Most of these features require a **Git** repository (jj has no staging area, so jj repos fall back to whole-file behavior or hide the affordance) and a configured **coding-agent CLI** (Settings > provider) for the change-set grouping.

---

## Table of Contents

- [Split a dirty tree into logical commits](#split-a-dirty-tree-into-logical-commits)
- [Tidy my working tree](#tidy-my-working-tree)
- [Partial commits: split one file across commits](#partial-commits-split-one-file-across-commits)
- [Hunk-level staging in the diff review](#hunk-level-staging-in-the-diff-review)
- [Analyze changes across branches](#analyze-changes-across-branches)
- [Worktree management](#worktree-management)
- [Compare a worktree to any folder](#compare-a-worktree-to-any-folder)
- [Guarantees](#guarantees)

---

## Split a dirty tree into logical commits

With no files selected, the Git view's **Commit Changes** button no longer means one big commit. Dirigent sends the working diff to your coding-agent CLI, which groups the changes into logical change sets. Each group becomes one entry in a **commit queue** inside the commit dialog ("Commit 1 of N"), scoped to its files, with a drafted, editable message.

How it works:

1. Dirty the working tree with changes from more than one concern.
2. Open the **Git view** (Changes) and click **Commit Changes** with nothing selected.
3. While the analysis runs, the button shows an "Analyzing..." state; then the commit dialog opens on "Commit 1 of N".
4. Page through groups with the ◀ ▶ buttons — each shows its own title, file scope, and message. Edits to a message are preserved when you page away and back.
5. Commit each group in turn (button or Cmd+Enter) — the dialog advances to the next; after the last one it reports "All change sets committed".

If you select files first, the button reads **Commit Selected** and makes a single commit without grouping. On a clean tree there is nothing to commit.

Edge cases are handled safely: oversized diffs degrade to a single "All changes" group, and a model grouping that overlaps or omits files is normalized in code (overlapping groups are merged, leftover files land in an "Other changes" group) so no file is lost and no file lands in two groups.

## Tidy my working tree

The same flow has a discoverable front door: **Git menu > Tidy my working tree (N dirty)**. The entry shows the dirty file count, reads "Tidying..." while an analysis runs, and is disabled on a clean tree.

You can also reshape the proposed commit *order*: the commit queue header has **↑ / ↓** buttons that move the current group earlier or later, keeping its files and any edited message. At the first position ↑ is disabled; at the last, ↓ is. The commits land in `git log` in the order you arranged.

Commit-after-review is the default — no path commits without explicit confirmation. The single exception is a workflow auto-commit you explicitly opted into.

## Partial commits: split one file across commits

*(Git only)*

When a single file mixes unrelated work, the analyzer can assign its **individual hunks** to different groups. Committing such a group stages only its owned hunks (via `git apply --cached`) and commits the staged index — the file's other hunks stay dirty for a later group.

In the commit dialog, such files are shown as `file (N hunks)` and marked with `· split` when shared between groups. After committing the first group, `git show HEAD -- <file>` contains only that group's hunks while `git diff -- <file>` still shows the remaining ones; committing the last group leaves the file clean.

If a hunk no longer maps cleanly to the live diff, Dirigent commits that file whole with a warning — it never stages a guessed range.

## Hunk-level staging in the diff review

*(Git only)*

Opening **all uncommitted changes** from the Git view (the **View All Changes** link above the file list) gives you an interactive staging surface:

- A toggle between **"Unstaged → to stage"** (working tree vs index) and **"Staged → to commit"** (index vs HEAD).
- Per-hunk **Stage / Discard** buttons in the unstaged view, and **Unstage** in the staged view.
- A `[-] partially staged` indicator for files with a mix of staged and unstaged hunks.

Two things worth knowing:

- **Unstaging lives in the "Staged → to commit" view** — toggle to it, then click **Unstage** on a hunk. If *everything* is already staged (for example, after pressing **Stage** in the commit dialog), the review opens directly in the staged view, since the unstaged side would be empty.
- **`[-] partially staged` marks only *mixed* files** — files with some hunks staged and others not. A fully staged file shows no indicator; it simply appears in the staged view and disappears from the unstaged one.

Every operation applies a byte-exact single-hunk patch. A patch that doesn't apply cleanly is rejected wholesale and reported — the index and working tree stay untouched.

## Analyze changes across branches

Review a handed-over branch *without checking it out*. Click the **⇄** compare button in the Git view header, pick a base and a head ref, and Dirigent groups `git diff base...head` (merge-base semantics — "what did this branch introduce") into the same logical change sets.

The result is a **read-only** summary: title, description, and files per group, with a "View full diff" link into the read-only diff review. There are no Commit/Stage actions — neither side is your working tree.

The confirm button is disabled while both sides are the same ref, and picking two identical refs reports "nothing to analyze" without a model call.

## Worktree management

The Worktrees panel (repo bar) includes several robustness features:

- **Checked-out branches are greyed out** in the create picker with "(in a worktree)" — git refuses a second worktree for a checked-out branch anyway; now you see why up front.
- **Orphan detection**: worktrees whose folder was deleted by hand are flagged **(orphaned)** with a ⚠ line and a one-click **Prune** (`git worktree prune`) to remove the stale entry.
- **Icons**: the main checkout shows a **solid ◆**, secondary worktrees a **hollow ◇**.
- **Retire instead of Remove**: retiring a worktree removes its folder, but the branch and its commits are kept — the hover copy says so explicitly.

## Compare a worktree to any folder

Each live worktree row has a **Compare...** button: pick any second folder and Dirigent diffs the two (`git diff --no-index`, off the UI thread) and opens the result read-only in the **side-by-side** view. Identical folders report "No differences between the folders" instead of an empty view.

Optionally, set `external_diff_tool` in `.Dirigent/settings.json` (e.g. `"external_diff_tool": "opendiff"` or `ksdiff`) to launch your own tool with the two paths instead; on launch failure Dirigent falls back to the built-in view.

## Guarantees

- **Nothing commits without your confirmation.** Every path goes through the commit dialog or an equally explicit click; the only auto-commit is a workflow's opt-in.
- **The file partition is enforced in code**, not trusted from the model: overlapping groups are merged, omitted files land in "Other changes", hallucinated files are dropped, and colliding hunk claims resolve first-wins.
- **`git apply` is atomic** — a rejected hunk patch leaves the index and working tree exactly as they were.
- **Fallback over guessing** — when a hunk can't be mapped to the live diff, the whole file is committed with a warning rather than staging a guessed range.
