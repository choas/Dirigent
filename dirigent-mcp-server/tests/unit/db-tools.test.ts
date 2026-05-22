import { describe, it, expect, beforeAll, afterAll, beforeEach } from "vitest";
import Database from "better-sqlite3";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

let db: InstanceType<typeof Database>;
let tmpDir: string;
let dbPath: string;

function now(): string {
  return new Date().toISOString().replace("T", " ").substring(0, 19);
}

beforeAll(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "dirigent-mcp-unit-"));
  dbPath = path.join(tmpDir, "Dirigent.db");

  db = new Database(dbPath);
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");
  db.pragma("busy_timeout = 5000");

  db.exec(`
    CREATE TABLE IF NOT EXISTS cues (
      id              INTEGER PRIMARY KEY AUTOINCREMENT,
      text            TEXT NOT NULL DEFAULT '',
      file_path       TEXT NOT NULL DEFAULT '',
      line_number     INTEGER NOT NULL DEFAULT 0,
      line_number_end INTEGER,
      status          TEXT NOT NULL DEFAULT 'inbox',
      source_label    TEXT NOT NULL DEFAULT '',
      source_id       TEXT NOT NULL DEFAULT '',
      source_ref      TEXT NOT NULL DEFAULT '',
      attached_images TEXT NOT NULL DEFAULT '[]',
      tag             TEXT,
      plan_path       TEXT NOT NULL DEFAULT '',
      has_question    INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS executions (
      id          INTEGER PRIMARY KEY AUTOINCREMENT,
      cue_id      INTEGER NOT NULL REFERENCES cues(id),
      prompt      TEXT NOT NULL DEFAULT '',
      response    TEXT NOT NULL DEFAULT '',
      diff        TEXT NOT NULL DEFAULT '',
      log         TEXT NOT NULL DEFAULT '',
      status      TEXT NOT NULL DEFAULT 'pending',
      provider    TEXT NOT NULL DEFAULT 'Claude',
      cost_usd    REAL NOT NULL DEFAULT 0.0,
      duration_ms INTEGER NOT NULL DEFAULT 0,
      num_turns   INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS cue_activity_log (
      id        INTEGER PRIMARY KEY AUTOINCREMENT,
      cue_id    INTEGER NOT NULL REFERENCES cues(id),
      timestamp TEXT NOT NULL,
      event     TEXT NOT NULL
    );
  `);
});

afterAll(() => {
  db.close();
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

beforeEach(() => {
  db.exec("DELETE FROM cue_activity_log");
  db.exec("DELETE FROM executions");
  db.exec("DELETE FROM cues");
});

describe("Cue CRUD operations", () => {
  it("should insert a cue into inbox", () => {
    const result = db
      .prepare(
        "INSERT INTO cues (text, file_path, line_number, status, tag) VALUES (?, ?, ?, 'inbox', ?)"
      )
      .run("Fix auth bug", "src/auth.rs", 42, "bugfix");
    const id = Number(result.lastInsertRowid);
    expect(id).toBeGreaterThan(0);

    const cue = db.prepare("SELECT * FROM cues WHERE id = ?").get(id) as Record<string, unknown>;
    expect(cue.text).toBe("Fix auth bug");
    expect(cue.file_path).toBe("src/auth.rs");
    expect(cue.line_number).toBe(42);
    expect(cue.status).toBe("inbox");
    expect(cue.tag).toBe("bugfix");
  });

  it("should move a cue to a different status", () => {
    const result = db
      .prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')")
      .run("Test cue");
    const id = Number(result.lastInsertRowid);

    db.prepare("UPDATE cues SET status = ? WHERE id = ?").run("ready", id);
    const cue = db.prepare("SELECT status FROM cues WHERE id = ?").get(id) as { status: string };
    expect(cue.status).toBe("ready");
  });

  it("should update cue text and tag", () => {
    const result = db
      .prepare("INSERT INTO cues (text, tag, status) VALUES (?, ?, 'inbox')")
      .run("Original text", "old-tag");
    const id = Number(result.lastInsertRowid);

    db.prepare("UPDATE cues SET text = ?, tag = ? WHERE id = ?").run(
      "Updated text",
      "new-tag",
      id
    );
    const cue = db.prepare("SELECT text, tag FROM cues WHERE id = ?").get(id) as {
      text: string;
      tag: string;
    };
    expect(cue.text).toBe("Updated text");
    expect(cue.tag).toBe("new-tag");
  });

  it("should delete a cue and its related records", () => {
    const result = db
      .prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')")
      .run("To delete");
    const id = Number(result.lastInsertRowid);

    db.prepare(
      "INSERT INTO executions (cue_id, prompt, status) VALUES (?, 'test', 'completed')"
    ).run(id);
    db.prepare(
      "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?, ?, 'Created')"
    ).run(id, now());

    db.prepare("DELETE FROM cue_activity_log WHERE cue_id = ?").run(id);
    db.prepare("DELETE FROM executions WHERE cue_id = ?").run(id);
    db.prepare("DELETE FROM cues WHERE id = ?").run(id);

    const cue = db.prepare("SELECT id FROM cues WHERE id = ?").get(id);
    expect(cue).toBeUndefined();
    const execs = db
      .prepare("SELECT id FROM executions WHERE cue_id = ?")
      .all(id);
    expect(execs).toHaveLength(0);
    const activityLogs = db
      .prepare("SELECT id FROM cue_activity_log WHERE cue_id = ?")
      .all(id);
    expect(activityLogs).toHaveLength(0);
  });
});

describe("Archive done cues", () => {
  it("should archive all done cues", () => {
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'done')").run("Done 1");
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'done')").run("Done 2");
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')").run("Not done");

    const doneCues = db
      .prepare("SELECT id FROM cues WHERE status = 'done'")
      .all() as { id: number }[];
    expect(doneCues).toHaveLength(2);

    for (const { id } of doneCues) {
      db.prepare("UPDATE cues SET status = 'archived' WHERE id = ?").run(id);
    }

    const remaining = db
      .prepare("SELECT id FROM cues WHERE status = 'done'")
      .all();
    expect(remaining).toHaveLength(0);

    const archived = db
      .prepare("SELECT id FROM cues WHERE status = 'archived'")
      .all();
    expect(archived).toHaveLength(2);
  });
});

describe("Pool summary", () => {
  it("should return correct counts per status", () => {
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')").run("A");
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')").run("B");
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'ready')").run("C");
    db.prepare("INSERT INTO cues (text, status) VALUES (?, 'done')").run("D");

    const rows = db
      .prepare(
        "SELECT status, COUNT(*) as count FROM cues GROUP BY status ORDER BY status"
      )
      .all() as { status: string; count: number }[];

    const map = Object.fromEntries(rows.map((r) => [r.status, r.count]));
    expect(map.inbox).toBe(2);
    expect(map.ready).toBe(1);
    expect(map.done).toBe(1);
  });
});

describe("Execution queries", () => {
  it("should return the latest execution for a cue", () => {
    const cueResult = db
      .prepare("INSERT INTO cues (text, status) VALUES (?, 'ready')")
      .run("Test");
    const cueId = Number(cueResult.lastInsertRowid);

    db.prepare(
      "INSERT INTO executions (cue_id, prompt, status, provider) VALUES (?, 'first', 'completed', 'Claude')"
    ).run(cueId);
    db.prepare(
      "INSERT INTO executions (cue_id, prompt, status, provider) VALUES (?, 'second', 'running', 'Claude')"
    ).run(cueId);

    const exec = db
      .prepare(
        "SELECT id, cue_id, status, provider FROM executions WHERE cue_id = ? ORDER BY id DESC LIMIT 1"
      )
      .get(cueId) as { status: string };
    expect(exec.status).toBe("running");
  });

  it("should return aggregate execution stats", () => {
    const cueResult = db
      .prepare("INSERT INTO cues (text, status) VALUES (?, 'done')")
      .run("Test");
    const cueId = Number(cueResult.lastInsertRowid);

    db.prepare(
      "INSERT INTO executions (cue_id, prompt, status, cost_usd, duration_ms, num_turns) VALUES (?, 'p1', 'completed', 0.05, 10000, 3)"
    ).run(cueId);
    db.prepare(
      "INSERT INTO executions (cue_id, prompt, status, cost_usd, duration_ms, num_turns) VALUES (?, 'p2', 'completed', 0.10, 20000, 5)"
    ).run(cueId);

    const stats = db
      .prepare(
        `SELECT
         COUNT(*) as total_executions,
         SUM(cost_usd) as total_cost_usd,
         SUM(duration_ms) as total_duration_ms,
         SUM(num_turns) as total_turns
       FROM executions WHERE status = 'completed'`
      )
      .get() as Record<string, number>;

    expect(stats.total_executions).toBe(2);
    expect(stats.total_cost_usd).toBeCloseTo(0.15);
    expect(stats.total_duration_ms).toBe(30000);
    expect(stats.total_turns).toBe(8);
  });
});

describe("Activity log", () => {
  it("should record and retrieve activity", () => {
    const cueResult = db
      .prepare("INSERT INTO cues (text, status) VALUES (?, 'inbox')")
      .run("Test");
    const cueId = Number(cueResult.lastInsertRowid);

    db.prepare(
      "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?, ?, ?)"
    ).run(cueId, now(), "Created via MCP");
    db.prepare(
      "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?, ?, ?)"
    ).run(cueId, now(), "Moved to Ready via MCP");

    const logs = db
      .prepare(
        "SELECT event FROM cue_activity_log WHERE cue_id = ? ORDER BY id"
      )
      .all(cueId) as { event: string }[];
    expect(logs).toHaveLength(2);
    expect(logs[0].event).toBe("Created via MCP");
    expect(logs[1].event).toBe("Moved to Ready via MCP");
  });
});
