# How to use Dirigent with jj (Jujutsu)

## The jj philosophy in 30 seconds

In jj, **your working copy is always a commit**. There is no staging area, no
`git add`, no "uncommitted changes" in the git sense. Every edit you make is
instantly part of the current working-copy commit. When you're ready to move on,
you *describe* that commit and start a new one on top.

This changes everything about how you think about version control:

| Git                          | jj                                      |
|------------------------------|------------------------------------------|
| Edit files, then stage+commit| Edit files -- they're already committed  |
| `git add -A && git commit`   | `jj commit -m "..."`                     |
| Branches                     | Bookmarks                                |
| `git checkout <branch>`      | `jj new <bookmark>`                      |
| `git stash`                  | Not needed -- just `jj new` and come back|
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

## How "committing" works in jj

This is the most important conceptual shift:

### In git (the old way)
```
1. Edit files
2. git add file1.rs file2.rs     # stage
3. git commit -m "my change"     # commit
```

### In jj (the jj way)
```
1. Edit files                     # already part of the working-copy commit (@)
2. jj commit -m "my change"      # describe @ and create a new empty child
```

`jj commit` does two things:
- Sets the description (commit message) on the *current* working-copy commit
- Creates a **new empty change** on top, which becomes your new working copy

After `jj commit`, your working directory is clean -- not because changes were
"saved away", but because you're now sitting on a fresh, empty commit. Your
previous work is the parent (`@-`).

### In Dirigent

When you click **Commit** in Dirigent (or a cue triggers a commit):
- Dirigent calls `jj commit -m "<message>"`
- The status bar updates to show the new (empty) working-copy change ID
- The history view shows your described commit as `@-`

There is no "stage files" step. All your edits are already in the commit.

---

## Core workflows

### Starting fresh work

You're always on a working-copy commit. Just start editing. If you want to
start from a specific bookmark:

```
jj new main
```

In Dirigent: use the **branch/bookmark switcher** in the repo bar and select a
bookmark. Dirigent calls `jj new <bookmark>` under the hood.

### Saving your work (committing)

```
jj commit -m "add user authentication"
```

In Dirigent: enter your commit message and click **Commit**. That's it.

Your work is now a described commit. You're on a new empty change, ready for
the next task.

### Checking what changed

```
jj diff              # what changed in the working copy vs parent
jj log               # commit history
jj status            # summary of working-copy changes
```

In Dirigent: the **file tree** shows dirty-file indicators, the **status bar**
shows modified/added/deleted counts, and the **history panel** shows the full
log.

### Pushing to a remote

```
jj git push
```

In Dirigent: click **Push** in the toolbar. Dirigent calls `jj git push` which
pushes all bookmarks that track a remote.

You need to have a bookmark pointing at the change you want to push. If your
working-copy commit has no bookmark, create one first:

```
jj bookmark create my-feature -r @-
```

### Fetching from a remote

```
jj git fetch
```

In Dirigent: click **Pull/Fetch** in the toolbar.

### Undoing / reverting files

```
jj restore --from @- path/to/file.rs
```

This restores `file.rs` from the parent commit. In Dirigent, use the
**Revert** action on individual files.

### Describing without advancing (amend the message)

If you want to set or change the description of the *current* working-copy
commit without creating a new child:

```
jj describe -m "work in progress on auth"
```

This is useful when you want to annotate what you're doing but aren't done yet.

---

## Key jj concepts mapped to Dirigent

### The `@` symbol

`@` always means "the current working-copy commit." In Dirigent's history view,
this is the topmost entry. Its change ID is shown in the status bar.

### Change IDs vs commit hashes

jj has two identifiers per commit:
- **Change ID** -- stable across rewrites (e.g. `kkmpptqz`). This is jj's
  primary identifier
- **Commit hash** -- like git's SHA, changes when a commit is rewritten

Dirigent shows the short change ID (7 chars) in the history panel and status
bar, similar to how git shows short SHAs.

### Bookmarks (not branches)

jj calls branches **bookmarks**. They are lightweight pointers, like git
branches, but they don't move automatically with new commits. You explicitly
set them:

```
jj bookmark set my-feature        # point bookmark at current change
jj bookmark create my-feature     # create new bookmark at current change
```

In Dirigent, the **branch/bookmark list** shows all bookmarks.

### Workspaces (not worktrees)

jj workspaces are like git worktrees -- separate working directories sharing
the same repo. In Dirigent, the **Worktree Manager** creates and removes
workspaces via `jj workspace add` and `jj workspace forget`.

---

## The jj workflow compared to typical git

### Git workflow
```
git checkout -b feature
# edit files
git add .
git commit -m "implement feature"
# edit more
git add .
git commit -m "fix tests"
git push -u origin feature
```

### jj workflow
```
jj new main                              # start from main
jj bookmark create feature               # name this line of work
# edit files (already in working copy)
jj commit -m "implement feature"         # describe and advance
# edit more (already in new working copy)
jj commit -m "fix tests"                 # describe and advance
jj git push                              # push tracked bookmarks
```

### Dirigent + jj workflow
```
1. Select "main" bookmark in Dirigent's branch picker
2. Edit files in your editor (Dirigent watches for changes)
3. Click Commit, type "implement feature"
4. Continue editing
5. Click Commit, type "fix tests"
6. Click Push
```

---

## Tips for git users switching to jj

1. **Stop thinking about staging.** There is no staging area. All edits are
   part of the current commit. This feels weird for a day, then it feels
   freeing.

2. **`jj commit` != `git commit`.** In jj, `commit` means "finalize the
   description and move on." In git, `commit` means "save staged changes."
   The mental model is different even though the command name is the same.

3. **You can't lose work as easily.** jj's operation log (`jj op log`) records
   every repo mutation. You can undo any operation with `jj op restore`.

4. **Conflicts are not emergencies.** jj can store conflicted states in
   commits. You can commit, rebase, and push conflicted code (to save
   progress) and resolve later.

5. **Bookmarks are explicit.** Unlike git branches, bookmarks don't auto-advance
   when you commit. Set them intentionally with `jj bookmark set`.

6. **No detached HEAD anxiety.** In jj, you're always on a change. There's no
   concept of being "detached" -- every working copy is a first-class commit.

---

## Quick reference

| What you want to do          | jj command                              | Dirigent action           |
|------------------------------|-----------------------------------------|---------------------------|
| Start new work from main     | `jj new main`                           | Branch picker > main      |
| See what changed             | `jj diff`                               | File tree indicators      |
| Commit current work          | `jj commit -m "msg"`                    | Commit button             |
| Describe without committing  | `jj describe -m "msg"`                  | --                        |
| View history                 | `jj log`                                | History panel             |
| Create a bookmark            | `jj bookmark create name`               | --                        |
| Push to remote               | `jj git push`                           | Push button               |
| Fetch from remote            | `jj git fetch`                          | Pull button               |
| Revert a file                | `jj restore --from @- file`             | Revert action on file     |
| Undo last operation          | `jj op restore @-`                      | --                        |
| List workspaces              | `jj workspace list`                     | Worktree Manager          |
| Create workspace             | `jj workspace add --name X path`        | Worktree Manager > Add    |
| Switch bookmark              | `jj new <bookmark>`                     | Branch picker             |
