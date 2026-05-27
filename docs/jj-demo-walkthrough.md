# jj Demo Walkthrough — hello-cli in Dirigent

Build a Node.js CLI app from scratch using Dirigent and jj. Each step creates
code through cues, commits via Dirigent, and uses jj bookmarks to organize
features. This walkthrough doubles as an integration test — if any expectation
doesn't match, it's a bug in Dirigent's jj support.

---

## Prerequisites

- Dirigent installed and running
- Node.js 18+ installed
- jj configured in Dirigent (Settings > VCS Backend > jj)

---

## Step 1 — Create a new project

### 1.1 Create an empty folder

Create a folder for the demo project (e.g. `~/jj-hello-demo`). This is the
only terminal step:

```bash
mkdir ~/jj-hello-demo
```

### 1.2 Open the project in Dirigent

**Click:** File > Open

**Navigate to:** `~/jj-hello-demo`

**Click:** Open

### 1.3 Set jj as the VCS backend

**Click:** Settings (gear icon)

**Navigate to:** VCS Backend

**Select:** jj

**Click:** Save / Apply

Dirigent will initialize the repository with jj automatically.

**Expected:**
- The **file tree** on the left is empty (no files yet)
- The **repo bar** at the top shows `jj-hello-demo`
- The **status bar** at the bottom shows a jj change ID
- The **jj Logs** shows a single root commit

> **Bug?** If the status bar doesn't show a change ID or shows a git hash
> instead, the jj backend detection may not be working.

---

## Step 2 — Create the initial "hello world" app

### 2.1 Enter the prompt

**Type** in the **prompt field** at the bottom:

```text
Create a Node.js CLI app. Create these files:

package.json with name "hello-cli", version "1.0.0", main "index.js",
bin entry "hello" pointing to "./index.js", and scripts: start "node index.js"
and test "node test.js".

index.js that prints "hello world!" to the console. Make it executable
with a #!/usr/bin/env node shebang.

.gitignore with node_modules/
```

**Click:** Send (or press Enter)

### 2.2 Review the result

**Expected:** The cue moves to **Review** status. The diff preview shows three
new files:

| File           | Content                            |
|----------------|------------------------------------|
| `package.json` | Project metadata with bin entry    |
| `index.js`     | `console.log("hello world!")`     |
| `.gitignore`   | `node_modules/`                    |

**Click:** `index.js` in the diff to verify it contains:

```js
#!/usr/bin/env node

console.log("hello world!");
```

> **Bug?** If the diff preview doesn't appear, or files show as empty, there
> may be a workspace sync issue.

### 2.3 Commit

**Click:** Commit

**Type** the commit message: `feat: initial hello world CLI`

**Click:** Commit

**Expected:**
- The **status bar** updates to a new change ID
- The **jj Logs** shows the commit with the message
- The **file tree** shows `index.js`, `package.json`, `.gitignore`
- The cue moves to **Done**

> **Bug?** If the commit doesn't appear in the jj Logs, or the change ID
> doesn't update, the jj commit flow may not be triggering correctly.

### 2.4 Create the `main` bookmark

**Click:** **jj** menu > **Create Bookmark**

**Type:** `main`

**Click:** Create

**Expected:** The **jj Logs** shows the `main` label on the commit.
The **status bar** shows `main`.

> **Bug?** If the bookmark doesn't appear in the jj Logs, bookmark
> creation or display may be broken.

---

## Step 3 — Create the random-planet feature

### 3.1 Create a new bookmark for the feature

**Click:** **jj** menu > **Create Bookmark**

**Type:** `feature/random-planet`

**Click:** Create

**Expected:** The **status bar** shows `feature/random-planet main` (both
bookmarks appear because `feature/random-planet` was just created on the same
commit as `main`). The jj Logs shows the new bookmark label on the
current (empty) working-copy commit.

### 3.2 Enter the prompt

**Type** in the **prompt field:**

```
Update index.js: instead of printing "hello world!", pick a random planet
from this list: Mercury, Venus, Mars, Jupiter, Saturn, Uranus, Neptune, Pluto.
Print "hello <planet>!" where <planet> is the randomly selected one.
Keep the shebang line.
```

**Click:** Send

### 3.3 Review the result

**Expected:** The diff shows `index.js` changed. The new version should have:
- A `planets` array with 8 planets
- A function to pick a random planet
- `console.log` using the random planet

**Verify** the diff shows removed line: `console.log("hello world!");`
and added lines with the planets array and random selection.

> **Bug?** If the diff shows the entire file as added (instead of showing the
> changes relative to the previous version), the parent commit resolution may
> be wrong.

### 3.4 Commit

**Click:** Commit

**Type:** `feat: greet a random planet instead of world`

**Click:** Commit

**Expected:** The **jj Logs** shows entries with jj change IDs
(short random strings), descriptions, and bookmark labels at the end:

```
● <id> @ (no description yet) (empty)
● <id> feat: greet a random planet instead of world  feature/random-planet
● <id> feat: initial hello world CLI  main
● zzzzzzz (no description yet) (empty)
```

> **Note:** jj identifies commits by change IDs (e.g. `yrwzwnn`), not by
> bookmark names. Bookmark labels appear at the end of the line. The `@`
> marker indicates the working copy. `zzzzzzz` is always the root commit.

> **Bug?** If `feature/random-planet` doesn't appear as a label, bookmark
> auto-advance after commit may not be working.

---

## Step 4 — Create the personalized greeting feature

### 4.1 Stay on the current line (building on random-planet)

The working copy is already on top of `feature/random-planet`. No need to
switch — the next feature builds on the planet feature.

### 4.2 Create a bookmark

**Click:** **jj** menu > **Create Bookmark**

**Type:** `feature/personalized`

**Click:** Create

### 4.3 Enter the prompt

**Type** in the **prompt field:**

```
Update index.js: accept an optional name as a CLI argument (process.argv[2]).
If a name is provided, print "hello <name> from <planet>!" where planet is
still randomly selected. If no name is given, keep the current behavior
of printing "hello <planet>!".
```

**Click:** Send

### 4.4 Review the result

**Expected:** The diff shows `index.js` changed to add:
- `const name = process.argv[2];`
- An `if (name)` branch printing `hello ${name} from ${planet}!`
- An `else` branch printing `hello ${planet}!`

> **Bug?** If the diff shows changes to the planets array (which shouldn't
> have changed), Claude may have rewritten more than needed — but that's a
> prompt issue, not a Dirigent bug.

### 4.5 Commit

**Click:** Commit

**Type:** `feat: personalized greeting with name argument`

**Click:** Commit

**Expected:** The **jj Logs** now shows a chain:

```
● <id> @ (no description yet) (empty)
● <id> feat: personalized greeting with name argument  feature/personalized
● <id> feat: greet a random planet instead of world  feature/random-planet
● <id> feat: initial hello world CLI  main
● zzzzzzz (no description yet) (empty)
```

---

## Step 5 — Create a docs bookmark (branching from main)

### 5.1 Switch back to main

**Click:** **jj** menu > **Switch Bookmark**

**Select:** `main` in the bookmark picker

**Expected:** The **file tree** refreshes. `index.js` now shows the original
`hello world!` version (not the planet version). The **status bar** shows
we're on top of `main`.

> **Bug?** If `index.js` still shows the planet version after switching to
> `main`, the working-copy checkout is not updating correctly.

### 5.2 Create the docs bookmark

**Click:** **jj** menu > **Create Bookmark**

**Type:** `docs`

**Click:** Create

### 5.3 Enter the prompt

**Type** in the **prompt field:**

```
Create a README.md file for this project. Include:
- Project name: hello-cli
- Description: A friendly greeting CLI built with Node.js
- Installation: npm link
- Usage examples: "node index.js" for basic greeting,
  "node index.js Alice" for personalized greeting
- Example outputs: "hello Jupiter!" and "hello Alice from Saturn!"
```

**Click:** Send

### 5.4 Review and commit

**Expected:** The diff shows a new `README.md` file. The `index.js` file
should NOT appear in the diff (it wasn't changed).

**Click:** Commit

**Type:** `docs: add README with usage instructions`

**Click:** Commit

**Expected:** The **jj Logs** now shows `docs` as a separate branch off
`main`, parallel to the `feature/random-planet` chain:

```
● <id> @ (no description yet) (empty)
● <id> docs: add README with usage instructions  docs
│ ● <id> feat: personalized greeting with name argument  feature/personalized
│ ● <id> feat: greet a random planet instead of world  feature/random-planet
├─╯
● <id> feat: initial hello world CLI  main
● zzzzzzz (no description yet) (empty)
```

> **Bug?** If `docs` appears on top of `feature/personalized` instead of
> branching from `main`, the bookmark switch in step 5.1 didn't work correctly.

---

## Step 6 — Create a test bookmark (branching from feature/personalized)

### 6.1 Switch to feature/personalized

**Click:** **jj** menu > **Switch Bookmark**

**Select:** `feature/personalized` in the bookmark picker

**Expected:** `index.js` in the **file tree** now shows the full version with
name argument and planet selection.

### 6.2 Create the test bookmark

**Click:** **jj** menu > **Create Bookmark**

**Type:** `test`

**Click:** Create

### 6.3 Enter the prompt

**Type** in the **prompt field:**

```
Create a test.js file that tests the CLI. Use child_process execSync to run
"node index.js" and verify the output. Test three cases:

1. No arguments: output matches "hello <planet>!" where planet is one of
   Mercury, Venus, Mars, Jupiter, Saturn, Uranus, Neptune, Pluto
2. With argument "Alice": output matches "hello Alice from <planet>!"
3. With argument "Bob": output matches "hello Bob from <planet>!"

Use regex to validate. Print checkmark for pass, X for fail.
Print summary "N passed, N failed" at the end.
Exit with code 1 if any test fails.
```

**Click:** Send

### 6.4 Review and commit

**Expected:** The diff shows a new `test.js` file with three test cases using
regex matching. `index.js` and `package.json` should NOT be in the diff.

**Click:** Commit

**Type:** `test: add CLI output tests`

**Click:** Commit

---

## Step 7 — Verify the history

### 7.1 Check the full graph

**Click:** The **History** tab

**Expected:** The complete graph should show this topology (each line
shows a jj change ID, the commit description, and the bookmark label):

```
● <id> @ (no description yet) (empty)
● <id> test: add CLI output tests  test
│ ● <id> docs: add README with usage instructions  docs
├─╯
● <id> feat: personalized greeting with name argument  feature/personalized
● <id> feat: greet a random planet instead of world  feature/random-planet
● <id> feat: initial hello world CLI  main
● zzzzzzz (no description yet) (empty)
```

Five bookmarks, each with a descriptive commit message. The working copy sits
on top of `test`.

> **Bug?** If any bookmark is missing, or the graph topology doesn't match
> (e.g. `test` branching from `main` instead of `feature/personalized`),
> there's a bookmark or commit parent issue.

### 7.2 Count the bookmarks

**Look at** the bookmark labels in the jj Logs.

**Expected:** Five bookmarks: `main`, `feature/random-planet`,
`feature/personalized`, `docs`, `test`.

---

## Step 8 — Browse code across bookmarks

This verifies that switching bookmarks updates the working copy correctly.

### 8.1 Switch to `main`

**Click:** **jj** menu > **Switch Bookmark** > `main`

**Click:** `index.js` in the **file tree**

**Expected:** `console.log("hello world!");` — the original version, no planets.

**Expected:** No `README.md` in the file tree (that's only on `docs`).
No `test.js` (that's only on `test`).

### 8.2 Switch to `feature/random-planet`

**Click:** **jj** menu > **Switch Bookmark** > `feature/random-planet`

**Click:** `index.js`

**Expected:** The planets array and `randomPlanet()` function. No
`process.argv` parsing (that's in the next bookmark).

### 8.3 Switch to `feature/personalized`

**Click:** **jj** menu > **Switch Bookmark** > `feature/personalized`

**Click:** `index.js`

**Expected:** Full version with `process.argv[2]`, `if (name)` conditional,
and the planets array.

### 8.4 Switch to `docs`

**Click:** **jj** menu > **Switch Bookmark** > `docs`

**Expected:** `README.md` appears in the **file tree**. `index.js` is the
original `hello world!` version (because `docs` branches from `main`).

> **Bug?** If `index.js` on the `docs` bookmark shows the planet version,
> the bookmark parent is wrong.

### 8.5 Switch to `test`

**Click:** **jj** menu > **Switch Bookmark** > `test`

**Expected:** `test.js` appears in the **file tree**. `index.js` is the full
personalized version (because `test` branches from `feature/personalized`).

---

## Step 9 — View diffs between commits

### 9.1 View the random-planet diff

**Click:** The `feature/random-planet` commit in the **jj Logs**

**Expected:** The diff shows `index.js` changed from `console.log("hello world!")`
to the planet version. Removed lines in red, added lines in green. Stats
show something like `+12 -1`.

> **Bug?** If clicking a commit doesn't open the diff view, or the diff is
> empty, the jj diff integration may not be working.

### 9.2 View the personalized diff

**Click:** The `feature/personalized` commit in the **jj Logs**

**Expected:** The diff shows only the `process.argv` and `if/else` additions.
The planets array should NOT appear in the diff (it didn't change between
`feature/random-planet` and `feature/personalized`).

### 9.3 View the test diff

**Click:** The `test` commit in the **jj Logs**

**Expected:** The diff shows only `test.js` as a new file. No changes to
`index.js` or `package.json`.

---

## Step 10 — Merge features into main

### 10.1 Switch to main

**Click:** **jj** menu > **Switch Bookmark** > `main`

### 10.2 Merge feature/personalized

**Click:** **jj** menu > **Merge Bookmark**

**Select:** `feature/personalized` in the bookmark picker

**Expected:** The status bar shows "Merging 'feature/personalized'..." and then
updates to "Merged 'feature/personalized' into 'main'". The jj Logs
shows a merge commit with two parents.

### 10.3 Verify main has the features

**Click:** `index.js` in the **file tree**

**Expected:** The full personalized version with planets and name argument.

> **Bug?** If `index.js` still shows `hello world!` after the merge, the
> merge didn't apply correctly or the working copy didn't update.

---

## Step 11 — Merge docs and tests

### 11.1 Merge docs

**Click:** **jj** menu > **Merge Bookmark**

**Select:** `docs`

**Expected:** `README.md` now appears in the **file tree** on `main`.

### 11.2 Merge tests

**Click:** **jj** menu > **Merge Bookmark**

**Select:** `test`

**Expected:** `test.js` now appears in the **file tree** on `main`.

---

## Step 12 — Run the tests

### 12.1 Verify everything works

**Type** in the **prompt field:**

```
Run npm test and show me the results.
```

**Click:** Send

**Expected:** Claude runs the tests and reports:

```
Running tests...

  ✓ no args prints hello <planet>!
  ✓ with name prints hello <name> from <planet>!
  ✓ with another name prints hello <name> from <planet>!

3 passed, 0 failed
```

> **Bug?** If tests fail, check whether the merge brought in both `index.js`
> (from `feature/personalized`) and `test.js` (from `test`) correctly.

---

## Step 13 — Clean up merged bookmarks

### 13.1 Delete feature bookmarks

**Click:** **jj** menu > **Delete Bookmark**

**Click:** Delete next to `feature/random-planet`

**Click:** Delete next to `feature/personalized`

**Click:** Delete next to `docs`

**Click:** Delete next to `test`

**Close** the dialog.

### 13.2 Verify

**Look at** the **jj Logs**.

**Expected:** Only `main` remains as a bookmark label. The old commits are
still visible in the graph but have no bookmark labels.

---

## Step 14 — Abandon empty heads

After deleting bookmarks, jj may leave behind empty head commits (changes with
no diff). Dirigent can find and abandon these in one click.

### 14.1 Abandon empty heads

**Click:** **jj** menu > **Abandon Empty Heads**

**Expected:** The status bar shows "Abandoned N empty head commit(s)" (where N
is the number of leftover empties). If there are no empty heads, the status bar
shows "No empty heads found".

### 14.2 Verify

**Look at** the **jj Logs**.

**Expected:** The previously empty head commits are gone from the graph. Only
commits with actual content remain.

> **Bug?** If empty heads still appear after clicking, the abandon command may
> have failed silently. Check that the jj binary is accessible and that the
> repository is in a clean state.

---

## Step 15 — Push to remote

### 15.1 Add a remote (if needed)

If the project doesn't have a remote yet, create a repository on your Git
hosting provider (e.g. GitHub, GitLab) and add the remote:

**Click:** **jj** menu > **Git** > **Add Remote**

**Type name:** `origin`

**Type URL:** `<your-repo-url>`

**Click:** Add

### 15.2 Push main to the remote

**Click:** **jj** menu > **Git** > **Push**

**Expected:** The status bar shows "Pushing to origin..." and then updates to
"Pushed 'main' to origin". The `main` bookmark and all its commits are pushed
to the remote.

> **Bug?** If the push fails with "no remote configured", the remote wasn't
> added correctly in 15.1. If it fails with "bookmark not tracked", the
> bookmark may need to be explicitly tracked on the remote first.

### 15.3 Verify

**Look at** the **jj Logs**.

**Expected:** The `main` bookmark now shows a remote tracking indicator
(e.g. `main@origin`) confirming the push succeeded.

---

## Bug Checklist

Use this as a quick reference when testing. Each row maps to an expectation
from the steps above.

| # | What to check                                          | Step  |
|---|--------------------------------------------------------|-------|
| 1 | jj change ID shown in status bar (not git hash)        | 1.2   |
| 2 | Cue diff preview appears after Claude finishes         | 2.2   |
| 3 | Commit updates change ID and jj Logs             | 2.3   |
| 4 | Bookmark label appears in jj Logs after creation | 2.4   |
| 5 | Bookmark auto-advances after commit                    | 3.4   |
| 6 | Switching bookmarks updates file tree content          | 5.1   |
| 7 | `docs` branches from `main` (not from feature chain)   | 5.4   |
| 8 | File tree shows correct files per bookmark             | 8.1–5 |
| 9 | Clicking a commit opens its diff                       | 9.1   |
| 10| Diff shows only changes relative to parent             | 9.2   |
| 11| Merge Bookmark button merges and updates file tree     | 10.2  |
| 12| Tests pass after merging all bookmarks                 | 12.1  |
| 13| Delete Bookmark button removes labels from history     | 13.1  |
| 14| Abandon Empty Heads removes empty commits from graph   | 14.1  |
| 15| Push sends main bookmark and commits to remote         | 15.2  |
