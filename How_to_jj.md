# How to use Dirigent with jj (Jujutsu)

## Why jj?

jj is a modern version control system that removes friction from your workflow.
The biggest difference from git: **your working copy is always a commit**. There
is no staging area, no `git add`, no "uncommitted changes." Every edit you make
is instantly part of the current working-copy commit. When you're ready to move
on, you *describe* that commit and start a new one on top.

This means fewer steps, fewer mistakes, and you can never lose uncommitted work.

| Git concept                  | jj equivalent                            |
|------------------------------|------------------------------------------|
| Branches                     | Bookmarks                                |
| `git add` + `git commit`    | `jj commit` (one step, no staging)       |
| `git push`                   | `jj git push`                            |
| `git pull`                   | `jj git fetch`                           |
| Worktrees                    | Workspaces                               |

---

## Setting up jj in Dirigent

1. **Install jj** -- see https://martinvonz.github.io/jj/latest/install/
2. Open Dirigent **Settings** (gear icon or menu)
3. Under **VCS Backend**, select **jj**
4. The **jj CLI path** is auto-detected. If not found, set it manually
   (e.g. `/opt/homebrew/bin/jj`)

Dirigent auto-detects `jj` via your login shell, Homebrew paths, and
well-known directories. If you installed jj through `cargo install`, it should
be found automatically.

---

## Your day-to-day workflow in Dirigent

This section walks through the full cycle: starting work, making changes,
committing, pushing, and creating a PR. Everything described here is what you
see and do inside Dirigent -- jj commands are shown in parentheses for context.

### 1. Start from a bookmark

Use the **branch/bookmark picker** in the repo bar and select where you want to
start (e.g. `main`). Dirigent creates a new working-copy commit on top of that
bookmark. You're ready to edit.

> Under the hood: `jj new main`

### 2. Create a cue and let Claude work

Create a cue describing what you want changed. Run Claude on it. Claude edits
files -- those edits are immediately part of your working-copy commit. There is
no staging step. The **file tree** shows dirty-file indicators and the **status
bar** shows counts like `M3 A2 D1` (3 modified, 2 added, 1 deleted).

The cue moves to **Review** status with a diff preview so you can inspect
Claude's changes.

### 3. Commit

When you're happy with the changes, click **Commit** (or use the commit action
on the cue). Enter a commit message. Dirigent does three things:

1. Finalizes the commit message on the current working-copy commit
2. Creates a new empty commit on top, which becomes your new working copy
3. Automatically advances any bookmarks so they follow your latest work

After the commit:
- The **status bar** updates to show the new change ID
- The **history panel** shows your described commit with its bookmark label
- The cue moves from **Review** to **Done**
- The status message confirms: "Committed: a1b2c3d"

Your working directory now looks clean -- not because changes disappeared, but
because you're sitting on a fresh, empty commit. Your work is safely in the
parent commit.

### 4. Keep going -- or push

You have two choices:

**Keep working:** Create another cue, let Claude make more changes, commit
again. Each commit stacks on top of the previous one. The history panel shows
the growing chain. You can make as many commits as you like before pushing.

**Push to the remote:** Click **Push** in the toolbar. Dirigent pushes all
bookmarks that track a remote. The status bar shows "Pushing..." during the
operation, then confirms with "Pushed (updated 1 bookmark)". The history panel
refreshes to show the remote tracking state.

### 5. Create a pull request

Click **Create PR**. Dirigent automatically pushes first (if needed), then
opens the PR creation dialog with a pre-filled branch name and title. After
creation, the PR URL appears in the status bar.

---

## What happens after multiple commits?

A typical session looks like this:

```
You start from main
  |
  v
Cue 1: "add user authentication"
  -> Claude edits files
  -> You review the diff
  -> You commit: "add user authentication"         <- commit a1b2c3d
  |
  v
Cue 2: "add tests for auth module"
  -> Claude edits files
  -> You review the diff
  -> You commit: "add tests for auth module"       <- commit e4f5g6h
  |
  v
Cue 3: "fix edge case in token refresh"
  -> Claude edits files
  -> You review the diff
  -> You commit: "fix edge case in token refresh"  <- commit i7j8k9l
  |
  v
You push -- all three commits go to the remote
  |
  v
You create a PR
```

Each commit is a clean, described unit of work. The history panel shows the
full chain with the commit graph. Your bookmark advances automatically with
each commit, so when you push, all commits behind that bookmark go to the
remote.

**You don't need to push after every commit.** Commits are local. Push when
you're ready to share -- after one commit or after ten.

---

## Why push?

Commits in jj are local until you push. They are safe (jj records every
operation and you can undo anything with `jj op restore`), but they only exist
on your machine.

Push when you want to:
- **Share your work** -- make it visible to collaborators
- **Create a PR** -- Dirigent pushes automatically when you create a PR
- **Back up** -- a remote is your off-machine safety net
- **Trigger CI** -- most CI systems run on push events

You need a **bookmark** pointing at your commits for push to work. Dirigent
handles this automatically: when you commit, it advances the bookmark to your
latest commit. If you started from `main` without creating a bookmark first,
you'll need to create one before pushing (via `jj bookmark create my-feature`
in the terminal).

---

## Understanding the status bar

The status bar gives you a quick read on the state of your repo:

| What you see         | What it means                                         |
|----------------------|-------------------------------------------------------|
| `◉ my-feature`      | Active bookmark name                                  |
| `M3 A2 D1`          | 3 modified, 2 added, 1 deleted files in working copy  |
| `↑2`                | 2 commits ahead of the remote (not yet pushed)        |
| Change ID on hover  | The 7-character jj change ID + description            |

When the status bar shows `↑2`, that means you have 2 local commits that
haven't been pushed yet. This is your signal that a push is available.

---

## Understanding the history panel

The history panel shows your commit history as a graph, similar to `jj log`.
Each entry shows:

- **Change ID** (7 characters) -- jj's stable identifier for the commit
- **Commit message** -- the first line of the description
- **Author and time** -- who and when
- **Bookmark labels** -- colored tags showing which bookmarks point here
- **`@` marker** -- indicates the current working-copy commit (the topmost entry)
- **(empty)** -- marks commits with no file changes (like your fresh working copy after a commit)

The graph lines show parent-child relationships between commits, so you can see
how your work branches and merges.

---

## Key differences from the git workflow

### No staging area

In git, you select which files to include in a commit (`git add`). In jj, all
changes in the working copy are part of the current commit. Dirigent reflects
this: there is no "stage files" step. When you click Commit, everything goes in.

This feels strange for about a day, then it feels freeing.

### Bookmarks don't auto-advance (but Dirigent handles it)

In raw jj, bookmarks are explicit -- they don't follow new commits the way git
branches do. Dirigent bridges this gap: after each commit, it automatically
advances your bookmark to the newly committed change. This gives you the
familiar git-branch-follows-commits behavior without manual bookmark management.

### You can't lose work

jj's operation log records every repo mutation. If something goes wrong, you
can undo any operation with `jj op restore`. Conflicts can be stored in commits
too -- you can commit conflicted code, keep working, and resolve later. There
is no "detached HEAD" anxiety.

---

## One workspace per cue

### Every cue gets its own workspace

Dirigent creates a dedicated jj workspace for *every* cue -- even when only one
is running. This keeps the design simple and consistent: the default workspace
stays clean (it's where you browse code), and each cue's changes are fully
isolated from the start. There's no special "single cue" vs "parallel cues" mode.

```
repo/                          <- default workspace (browsing, no cue runs here)
repo-workspace-add-auth/       <- workspace for cue "add authentication"
repo-workspace-add-logging/    <- workspace for cue "add logging"
```

jj workspaces are lightweight -- they share the repo store, each has an
independent working-copy commit, and creating or tearing down a workspace is
fast with no side effects on other workspaces. This makes "one workspace per cue"
cheap enough to be the default, not a special case.

### Auto-commit and bookmarks

When a cue finishes, Dirigent automatically commits the changes in that cue's
workspace and sets a **bookmark** pointing to the new commit. The user doesn't
have to remember to commit -- it just happens. The bookmark is named after the
cue (e.g., `cue/add-authentication`), so every cue's result is easy to find in
the log.

> Under the hood:
> ```
> jj workspace add ../repo-workspace-add-auth   # Dirigent creates workspace
> # ... Claude runs the cue ...
> jj commit -m "add authentication"             # Dirigent auto-commits
> jj bookmark set cue/add-auth                  # Dirigent sets bookmark
> jj workspace forget add-auth                  # Dirigent cleans up workspace
> ```

After both cues are committed, the user can review them independently. If they
look good, stack them (rebase one on top of the other) or merge them.

### Reverting a cue is just moving a bookmark

This is where jj shines compared to git. If the user doesn't like what a cue
produced, reverting is trivial: move the bookmark back (or delete it). In jj,
bookmarks are just pointers -- the commit still exists in the log but is no
longer referenced. Nothing is destroyed, and the operation is instant.

```
jj bookmark delete cue/add-auth     # drop the bookmark, changes become hidden
```

If the user changes their mind later, the commit is still in `jj log --revisions
'hidden()'` and can be restored. Compare this to git, where reverting means
creating a new revert commit (which clutters history) or using `git reset --hard`
(which can lose work if you're not careful). jj's approach is non-destructive by
default.

Even for partial reverts, jj makes it easy:

```
jj backout -r <cue-commit>         # create an inverse commit, keeping history clean
```

### Why not git worktrees?

Git worktrees achieve similar isolation, but they're heavier: each worktree is a
full checkout with its own `.git` link, and branch management gets awkward (you
can't have the same branch checked out in two worktrees). jj workspaces share
the repo store, have independent working-copy commits, and have no branch
conflicts. The "one workspace per cue" model would be cumbersome with git
worktrees -- with jj workspaces it's natural.

---

## Common tasks

### Revert a file

In the file tree, use the **Revert** action on individual files. This restores
the file from the parent commit, undoing changes in the working copy.

> Under the hood: `jj restore --from @- path/to/file.rs`

### Switch to a different bookmark

Use the **branch/bookmark picker** in the repo bar. Selecting a bookmark
creates a new working-copy commit at that bookmark's position.

> Under the hood: `jj new <bookmark>`

### Fetch from the remote

Click **Pull/Fetch** in the toolbar to get the latest changes from the remote.

> Under the hood: `jj git fetch`

### Work on multiple things at once (workspaces)

Use the **Worktree Manager** to create and manage jj workspaces. Each workspace
is a separate working directory sharing the same repo -- like git worktrees.

> Under the hood: `jj workspace add`, `jj workspace forget`

---

## Quick reference

| What you want to do          | Dirigent action                | jj command (FYI)                   |
|------------------------------|--------------------------------|------------------------------------|
| Start new work from main     | Branch picker > main           | `jj new main`                      |
| See what changed             | File tree + status bar         | `jj diff`, `jj status`            |
| Commit current work          | Commit button + message        | `jj commit -m "msg"`              |
| View history                 | History panel                  | `jj log`                           |
| Push to remote               | Push button                    | `jj git push`                      |
| Fetch from remote            | Pull/Fetch button              | `jj git fetch`                     |
| Revert a file                | Revert action on file          | `jj restore --from @- file`       |
| Switch bookmark              | Branch picker                  | `jj new <bookmark>`               |
| Manage workspaces            | Worktree Manager               | `jj workspace add/forget`          |
| Create a bookmark            | *(terminal)*                   | `jj bookmark create name`         |
| Undo last operation          | *(terminal)*                   | `jj op restore @-`                |
