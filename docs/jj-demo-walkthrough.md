# jj Demo Walkthrough — hello-cli in Dirigent

A step-by-step walkthrough using Dirigent to explore a Node.js CLI project
managed with jj. The demo repo has multiple bookmarks representing the
evolution of a greeting app from "hello world" to personalized planet greetings.

---

## Prerequisites

- Dirigent installed and running
- Node.js 18+ installed
- jj configured in Dirigent (Settings > VCS Backend > jj)

## Setup

Run the setup script once in a terminal to create the demo repo:

```bash
./scripts/setup_jj_demo.sh
```

This creates a jj repo at `/tmp/jj-hello-demo` with five bookmarks.

---

## Step 1 — Open the demo project in Dirigent

**Click:** File > Open (or drag the folder onto Dirigent)

**Navigate to:** `/tmp/jj-hello-demo`

**Click:** Open

**Expected:** Dirigent opens the project. The **file tree** on the left shows
`index.js`, `package.json`, and `.gitignore`. The **repo bar** at the top shows
the repo name `jj-hello-demo`. The **status bar** at the bottom shows the
current change ID and `main` as the active bookmark.

---

## Step 2 — Explore the bookmarks

### 2.1 Open the history panel

**Click:** The **History** tab (or the history icon in the sidebar)

**Expected:** A commit graph (DAG) appears showing five bookmarks:

| Bookmark label            | Commit message                                    |
|---------------------------|---------------------------------------------------|
| `main`                    | feat: initial hello world CLI                     |
| `feature/random-planet`   | feat: greet a random planet instead of world      |
| `feature/personalized`    | feat: personalized greeting with name argument    |
| `docs`                    | docs: add README with usage instructions          |
| `test`                    | test: add CLI output tests                        |

The graph shows that `docs` branches off `main`, and `feature/random-planet` >
`feature/personalized` > `test` form a chain also branching from `main`.

### 2.2 View the bookmark topology

**Look at** the history panel graph lines:

```
          ┌── docs ──────────── (README.md)
          │
main ─────┤
          │
          └── feature/random-planet ── feature/personalized ── test
                (planets array)          (name argument)       (test.js)
```

The working copy (`@`) sits on top of `main`, shown as the topmost entry
marked `(empty)`.

---

## Step 3 — Browse code on `main` (hello world)

The working copy starts on `main`, so you're already looking at the initial
version.

**Click:** `index.js` in the **file tree**

**Expected:** The **code viewer** shows:

```js
#!/usr/bin/env node

console.log("hello world!");
```

This is the simplest version — it just prints `hello world!`.

---

## Step 4 — Switch to `feature/random-planet`

### 4.1 Open the Worktree Manager

**Click:** **Worktrees** in the **repo bar**

**Expected:** The Worktree Manager dialog opens, showing the bookmark picker.

### 4.2 Select the bookmark

**Click:** The **bookmark picker** dropdown

**Select:** `feature/random-planet`

**Expected:** Dirigent creates a new working-copy commit on top of
`feature/random-planet`. The **status bar** updates to show this bookmark.
The **file tree** refreshes.

### 4.3 View the changed code

**Click:** `index.js` in the **file tree**

**Expected:** The code viewer now shows the updated version with a `planets`
array and a `randomPlanet()` function:

```js
#!/usr/bin/env node

const planets = [
  "Mercury", "Venus", "Mars", "Jupiter",
  "Saturn", "Uranus", "Neptune", "Pluto"
];

function randomPlanet() {
  return planets[Math.floor(Math.random() * planets.length)];
}

const planet = randomPlanet();
console.log(`hello ${planet}!`);
```

"world" has been replaced with a randomly selected planet.

---

## Step 5 — Switch to `feature/personalized`

### 5.1 Switch bookmark

**Click:** **Worktrees** in the **repo bar**

**Select:** `feature/personalized` in the bookmark picker

### 5.2 View the code

**Click:** `index.js` in the **file tree**

**Expected:** The code now includes `process.argv[2]` parsing. When a name is
provided as a CLI argument, it prints `hello <name> from <planet>!`. Without a
name, it falls back to `hello <planet>!`:

```js
const name = process.argv[2];
const planet = randomPlanet();

if (name) {
  console.log(`hello ${name} from ${planet}!`);
} else {
  console.log(`hello ${planet}!`);
}
```

---

## Step 6 — View the diff between bookmarks

### 6.1 See what changed from `main` to `feature/random-planet`

**Click:** The `feature/random-planet` commit in the **history panel**

**Expected:** The diff view opens, showing `index.js` changed from the
one-liner `console.log("hello world!")` to the version with the `planets` array
and `randomPlanet()` function. Added lines are highlighted in green, removed
lines in red. The diff stats show something like `+12 -1`.

### 6.2 See what changed from `feature/random-planet` to `feature/personalized`

**Click:** The `feature/personalized` commit in the **history panel**

**Expected:** The diff shows the addition of `process.argv[2]` parsing and the
conditional `if (name)` branch. The `planets` array and `randomPlanet()`
function are unchanged (not shown in the diff).

---

## Step 7 — Browse the docs bookmark

### 7.1 Switch to docs

**Click:** **Worktrees** in the **repo bar**

**Select:** `docs` in the bookmark picker

### 7.2 Read the README

**Click:** `README.md` in the **file tree** (this file only exists on the
`docs` bookmark)

**Expected:** The code viewer shows the project README with installation and
usage instructions. Note that `index.js` on this bookmark is still the original
"hello world" version, because `docs` branches off `main`, not off
`feature/personalized`.

**Click:** `index.js` to verify — it should show just `console.log("hello world!")`.

---

## Step 8 — Browse the test bookmark

### 8.1 Switch to test

**Click:** **Worktrees** in the **repo bar**

**Select:** `test` in the bookmark picker

### 8.2 Read the test file

**Click:** `test.js` in the **file tree** (this file only exists on the `test`
bookmark)

**Expected:** The code viewer shows three test cases:

1. No arguments — expects `hello <planet>!`
2. With name "Alice" — expects `hello Alice from <planet>!`
3. With name "Bob" — expects `hello Bob from <planet>!`

Each test uses regex matching to validate the output against the known planet
list.

### 8.3 Verify index.js has all features

**Click:** `index.js` in the **file tree**

**Expected:** The full personalized version (with `process.argv[2]` and the
planet array). The `test` bookmark is based on `feature/personalized`, so it
has all the features needed to make the tests pass.

---

## Step 9 — Create a cue to merge features into main

### 9.1 Switch back to main

**Click:** **Worktrees** in the **repo bar**

**Select:** `main` in the bookmark picker

### 9.2 Create a merge cue

**Type** in the **prompt field** at the bottom:

```
Merge the feature/personalized bookmark into main using jj. 
Then merge the docs and test bookmarks as well. 
Finally run npm test to verify everything works.
```

**Click:** Send (or press Enter)

**Expected:** The cue appears in the **Inbox** column of the cue pool. Dirigent
creates a workspace for the cue and Claude executes the merge operations.

### 9.3 Review the result

**Expected:** After Claude finishes, the cue moves to **Review** status. The
diff preview shows the merged changes. The history panel shows merge commits
with `main` now containing all features, docs, and tests.

**Click:** Accept to commit the merge.

---

## Step 10 — Verify with tests

### 10.1 Check the history

**Click:** The **History** tab

**Expected:** `main` now sits at the top of the graph, with merge commits
connecting it to the feature bookmarks. All the code from `feature/personalized`,
`docs`, and `test` is now part of `main`.

### 10.2 Run tests via a cue

**Type** in the **prompt field:**

```
Run npm test and show me the output
```

**Expected:** Claude runs the tests and reports:

```
Running tests...

  ✓ no args prints hello <planet>!
  ✓ with name prints hello <name> from <planet>!
  ✓ with another name prints hello <name> from <planet>!

3 passed, 0 failed
```

---

## Summary

| Step | What you do in Dirigent                        | What you see                              |
|------|------------------------------------------------|-------------------------------------------|
| 1    | Open `/tmp/jj-hello-demo`                      | File tree with index.js, package.json     |
| 2    | Open History panel                             | DAG with 5 bookmarks                      |
| 3    | Click `index.js` on main                       | `hello world!` source code                |
| 4    | Switch to `feature/random-planet`              | Code with planets array, randomPlanet()   |
| 5    | Switch to `feature/personalized`               | Code with name argument and planet        |
| 6    | Click commits in History                       | Diffs showing changes between bookmarks   |
| 7    | Switch to `docs`, read README.md               | Project documentation, original index.js  |
| 8    | Switch to `test`, read test.js                 | Three test cases with regex validation    |
| 9    | Create a cue to merge all into main            | Merge commits in history                  |
| 10   | Run tests via prompt                           | 3 passed, 0 failed                        |
