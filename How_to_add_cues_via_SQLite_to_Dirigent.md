# How to add cues via SQLite to Dirigent

Dirigent stores all of its state in a SQLite database. If you want to bulk-create
cues, script them from another tool, or import them from an external system, you
can write directly into that database instead of clicking through the UI.

This guide shows how the `cues` table is structured and how to insert rows safely.

> ⚠️ **Back up first.** Close Dirigent (or at least be aware it may be running)
> before editing the database by hand, and keep a copy of the file. Writing bad
> data can confuse the UI. Re-launching Dirigent picks up newly inserted cues.

---

## Where the database lives

Dirigent creates one database **per project**, inside a hidden `.Dirigent`
folder at the root of the repository you opened:

```
<project-root>/.Dirigent/Dirigent.db
```

It runs in WAL mode with foreign keys enabled, so you may also see
`Dirigent.db-wal` and `Dirigent.db-shm` sidecar files. Leave those alone — they
are managed by SQLite.

Open it with the standard CLI:

```sh
sqlite3 "<project-root>/.Dirigent/Dirigent.db"
```

---

## The `cues` table

A cue is the central coordination unit in Dirigent. The schema (after all
migrations) looks like this:

| Column            | Type      | Required | Meaning                                                                 |
| ----------------- | --------- | -------- | ----------------------------------------------------------------------- |
| `id`              | INTEGER   | auto     | Primary key (autoincrement) — leave it out and SQLite assigns it.       |
| `text`            | TEXT      | **yes**  | The cue's prompt / instruction text. Max ~100,000 bytes.                |
| `file_path`       | TEXT      | **yes**  | Repo-relative file the cue is attached to. Use `''` for a global cue.   |
| `line_number`     | INTEGER   | **yes**  | 1-based start line. Use `0` for a global (non-file) cue.                |
| `line_number_end` | INTEGER   | no       | Optional end line for a range. `NULL` for a single line.                |
| `status`          | TEXT      | **yes**  | Kanban column (see below). Defaults to `'inbox'`.                       |
| `source_label`    | TEXT      | no       | Human label of an external source (e.g. `"GitHub Issues"`).            |
| `source_id`       | TEXT      | no       | Source-specific identifier.                                             |
| `source_ref`      | TEXT      | no       | Source reference (indexed; e.g. issue number / URL).                    |
| `attached_images` | TEXT      | no       | JSON array of image file paths, e.g. `'["/tmp/a.png"]'`. `NULL` if none.|
| `tag`             | TEXT      | no       | Optional user tag for grouping.                                         |
| `plan_path`       | TEXT      | no       | Path to a Claude Code plan file. `NULL` normally.                       |
| `has_question`    | INTEGER   | no       | `0`/`1` flag, defaults `0`. Set by the agent, not by you.              |
| `workflow`        | INTEGER   | no       | `0`/`1` flag, defaults `0`. Set when part of a workflow.               |
| `auto_commit`     | INTEGER   | no       | `0`/`1` flag, defaults `0`. Auto-commit on completion.                 |

### Valid `status` values

The status maps to a kanban column in the Cue Pool:

| Value        | Column label |
| ------------ | ------------ |
| `inbox`      | Inbox        |
| `ready`      | Running      |
| `review`     | Review       |
| `done`       | Done         |
| `archived`   | Archived     |
| `backlog`    | Backlog      |

New cues should almost always start as `inbox`. An unknown status falls back to
`inbox` when Dirigent reads the row.

---

## Inserting a cue

### A global cue (not attached to any file)

This is the simplest case — equivalent to typing a prompt into the global prompt
field:

```sql
INSERT INTO cues (text, file_path, line_number, status)
VALUES ('Update the README with installation instructions', '', 0, 'inbox');
```

### A cue attached to a specific line

```sql
INSERT INTO cues (text, file_path, line_number, status)
VALUES ('Refactor this function to return a Result', 'src/main.rs', 42, 'inbox');
```

### A cue attached to a line range

```sql
INSERT INTO cues (text, file_path, line_number, line_number_end, status)
VALUES ('Add tests covering this block', 'src/db/cue_ops.rs', 28, 62, 'inbox');
```

### A cue with an external source reference

```sql
INSERT INTO cues (text, file_path, line_number, status, source_label, source_ref)
VALUES ('Fix the crash on startup', '', 0, 'inbox', 'GitHub Issues', '#123');
```

> **Note:** Only `text`, `file_path`, `line_number`, and `status` are needed.
> All other columns have safe defaults (`NULL`, `0`, or `'inbox'`), so you can
> omit them.

---

## Recording activity (optional but recommended)

When Dirigent creates a cue through the UI, it also writes a row to the
`cue_activity_log` table so the cue has a history. You can mirror that:

```sql
INSERT INTO cue_activity_log (cue_id, timestamp, event)
VALUES (last_insert_rowid(), strftime('%Y-%m-%d %H:%M:%S', 'now', 'localtime'), 'Created');
```

The `cue_activity_log` table has these columns:

| Column      | Type    | Meaning                                         |
| ----------- | ------- | ----------------------------------------------- |
| `id`        | INTEGER | Primary key (autoincrement).                    |
| `cue_id`    | INTEGER | Foreign key into `cues(id)`.                    |
| `timestamp` | TEXT    | `YYYY-MM-DD HH:MM:SS` (local time).             |
| `event`     | TEXT    | Free-text event, e.g. `"Created"`.              |

This step is optional — a cue with no activity rows still works fine.

---

## Bulk-importing many cues

To import a batch from another system, wrap the inserts in a transaction so they
all land atomically:

```sql
BEGIN;

INSERT INTO cues (text, file_path, line_number, status)
VALUES ('First task',  'src/a.rs', 10, 'inbox');

INSERT INTO cues (text, file_path, line_number, status)
VALUES ('Second task', 'src/b.rs', 20, 'inbox');

INSERT INTO cues (text, file_path, line_number, status)
VALUES ('Third task',  '',          0, 'inbox');

COMMIT;
```

You can also pipe a generated `.sql` file into the database:

```sh
sqlite3 "<project-root>/.Dirigent/Dirigent.db" < my_cues.sql
```

Or import from CSV using the SQLite shell's `.import` command into a staging
table, then `INSERT … SELECT` into `cues`.

---

## Verifying your inserts

List the most recently added cues:

```sql
SELECT id, status, file_path, line_number, substr(text, 1, 60) AS preview
FROM cues
ORDER BY id DESC
LIMIT 10;
```

Then re-launch Dirigent (or reopen the project) and the new cues will appear in
the Cue Pool under the column matching their `status`.

---

## Gotchas

- **Attach to a real file.** `file_path` is repo-relative (e.g. `src/main.rs`).
  If the file or line does not exist, the cue still shows up but won't anchor to
  code.
- **`line_number` is 1-based.** `0` is reserved for global cues.
- **Don't quote integers.** `line_number`, the `_end`, and the boolean flags are
  INTEGER columns; pass numbers, not strings.
- **Escape single quotes** in `text` by doubling them: `'it''s broken'`.
- **`attached_images` must be valid JSON** — a JSON array string like
  `'["/path/one.png","/path/two.png"]'`, or `NULL` for none.
- **Foreign keys are on.** Any `cue_id` you write into `cue_activity_log`,
  `executions`, or `agent_runs` must reference an existing `cues.id`.
- **Don't set `id` manually** unless you know it's free — let AUTOINCREMENT
  handle it.
```
