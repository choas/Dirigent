# jj Demo Walkthrough — hello-cli

This document walks through a Node.js CLI project managed with jj (Jujutsu).
The demo repo has multiple bookmarks that represent the evolution of a simple
greeting app. Each section tells you what to type, what to click, and what to
expect.

---

## Prerequisites

- Node.js 18+ installed
- jj 0.35+ installed (`brew install jj` or see https://jj-vcs.github.io/jj/)
- A terminal

## Setup

```bash
./scripts/setup_jj_demo.sh          # creates /tmp/jj-hello-demo
cd /tmp/jj-hello-demo
```

**Expected:** The script prints the bookmark list and a log graph showing five
bookmarks: `main`, `feature/random-planet`, `feature/personalized`, `docs`,
and `test`.

---

## Step 1 — Explore the repo

### 1.1 View the log

**Type:**
```bash
jj log
```

**Expected:** A DAG (graph) with these commits:

| Bookmark                | Description                               |
|-------------------------|-------------------------------------------|
| `main`                  | feat: initial hello world CLI             |
| `feature/random-planet` | feat: greet a random planet instead of world |
| `feature/personalized`  | feat: personalized greeting with name argument |
| `docs`                  | docs: add README with usage instructions  |
| `test`                  | test: add CLI output tests                |

The working copy (`@`) sits on top of `main`.

### 1.2 List bookmarks

**Type:**
```bash
jj bookmark list
```

**Expected:** Five bookmarks listed:
```
docs
feature/personalized
feature/random-planet
main
test
```

---

## Step 2 — Run the app on each bookmark

### 2.1 Run on `main` (hello world)

**Type:**
```bash
jj new main
node index.js
```

**Expected output:**
```
hello world!
```

### 2.2 Run on `feature/random-planet`

**Type:**
```bash
jj new feature/random-planet
node index.js
```

**Expected output** (planet varies each run):
```
hello Jupiter!
```

The planet is randomly chosen from: Mercury, Venus, Mars, Jupiter, Saturn,
Uranus, Neptune, Pluto.

### 2.3 Run on `feature/personalized`

**Type:**
```bash
jj new feature/personalized
node index.js
```

**Expected output** (no name provided — falls back to planet-only greeting):
```
hello Saturn!
```

**Type:**
```bash
node index.js Alice
```

**Expected output:**
```
hello Alice from Mars!
```

---

## Step 3 — Read the docs bookmark

**Type:**
```bash
jj new docs
cat README.md
```

**Expected:** The README is printed, showing installation and usage
instructions. Note that on this bookmark `index.js` is still the original
"hello world" version because `docs` branches off `main`, not
`feature/personalized`.

---

## Step 4 — Run the tests

**Type:**
```bash
jj new test
npm test
```

**Expected output:**
```
Running tests...

  ✓ no args prints hello <planet>!
  ✓ with name prints hello <name> from <planet>!
  ✓ with another name prints hello <name> from <planet>!

3 passed, 0 failed
```

The `test` bookmark is based on `feature/personalized`, so `index.js` supports
both the planet-only and the personalized greeting.

---

## Step 5 — View diffs between bookmarks

### 5.1 See what `feature/random-planet` changed from `main`

**Type:**
```bash
jj diff -r main -r feature/random-planet
```

**Expected:** A diff showing `index.js` changed from
`console.log("hello world!")` to the random-planet version with the `planets`
array and `randomPlanet()` function.

### 5.2 See what `feature/personalized` added on top

**Type:**
```bash
jj diff -r feature/random-planet -r feature/personalized
```

**Expected:** A diff showing the addition of `process.argv[2]` parsing and the
conditional `if (name)` branch.

---

## Step 6 — Merge features into main

### 6.1 Merge personalized greeting

**Type:**
```bash
jj new main feature/personalized
jj commit -m "merge: personalized greeting into main"
jj bookmark set main -r @-
```

**Expected:** `jj log` now shows `main` pointing at a merge commit with two
parents: the old `main` and `feature/personalized`.

### 6.2 Verify the merge

**Type:**
```bash
jj new main
node index.js World
```

**Expected output:**
```
hello World from Venus!
```

---

## Step 7 — Merge docs and tests

**Type:**
```bash
jj new main docs
jj commit -m "merge: add documentation"
jj bookmark set main -r @-

jj new main test
jj commit -m "merge: add tests"
jj bookmark set main -r @-
```

**Verify everything works together:**
```bash
jj new main
npm test
```

**Expected:** All 3 tests pass.

---

## Step 8 — Clean up merged bookmarks

**Type:**
```bash
jj bookmark delete feature/random-planet
jj bookmark delete feature/personalized
jj bookmark delete docs
jj bookmark delete test
```

**Expected:** `jj bookmark list` shows only `main`.

---

## Bookmark Topology

```
          ┌── docs ──────────── (README.md)
          │
main ─────┤
          │
          └── feature/random-planet ── feature/personalized ── test
                (planets array)          (name argument)       (test.js)
```

## Summary

| Step | Action                        | Command                                | What you see                        |
|------|-------------------------------|----------------------------------------|-------------------------------------|
| 1    | View the log                  | `jj log`                               | DAG with 5 bookmarks                |
| 2.1  | Run hello world               | `jj new main && node index.js`         | `hello world!`                      |
| 2.2  | Run random planet             | `jj new feature/random-planet && node index.js` | `hello Jupiter!`          |
| 2.3  | Run personalized              | `jj new feature/personalized && node index.js Alice` | `hello Alice from Mars!` |
| 3    | Read docs                     | `jj new docs && cat README.md`         | README content                      |
| 4    | Run tests                     | `jj new test && npm test`              | 3 passed, 0 failed                  |
| 5    | View diffs                    | `jj diff -r main -r feature/random-planet` | index.js changes             |
| 6    | Merge into main               | `jj new main feature/personalized ...` | Merge commit in log                 |
| 7    | Merge docs & tests            | `jj new main docs ...`                 | All tests pass on main              |
| 8    | Delete merged bookmarks       | `jj bookmark delete ...`               | Only `main` remains                 |
