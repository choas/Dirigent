import { describe, it, expect, beforeAll, afterAll } from "vitest";
import Database from "better-sqlite3";
import { execFileSync } from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

function isClaudeAvailable(): boolean {
  try {
    execFileSync(process.env.CLAUDE_CLI_PATH ?? "claude", ["--version"], {
      stdio: "ignore",
      timeout: 5_000,
    });
    return true;
  } catch {
    return false;
  }
}

const claudeAvailable = isClaudeAvailable();

const TEST_CUE_TEXT =
  "[integration-test] MCP self-referential running cues query";

describe.skipIf(!claudeAvailable)("Dirigent MCP Integration", { timeout: 60_000 }, () => {
  let dbPath: string;
  let tmpDir: string;
  let testCueId: number;

  beforeAll(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "dirigent-mcp-test-"));
    dbPath = path.join(tmpDir, "Dirigent.db");

    const db = new Database(dbPath);
    db.pragma("journal_mode = WAL");
    db.pragma("foreign_keys = ON");

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

    const result = db
      .prepare(
        `INSERT INTO cues (text, file_path, line_number, status, tag)
       VALUES (?, '', 0, 'ready', 'integration-test')`
      )
      .run(TEST_CUE_TEXT);
    testCueId = Number(result.lastInsertRowid);

    db.prepare(
      `INSERT INTO executions (cue_id, prompt, status, provider)
       VALUES (?, ?, 'running', 'Claude')`
    ).run(testCueId, "Integration test execution");

    const now = new Date().toISOString().replace("T", " ").substring(0, 19);
    db.prepare(
      `INSERT INTO cue_activity_log (cue_id, timestamp, event)
       VALUES (?, ?, 'Created for integration test')`
    ).run(testCueId, now);

    db.prepare(
      "INSERT INTO cues (text, status) VALUES (?, 'inbox')"
    ).run("Some inbox cue");
    db.prepare(
      "INSERT INTO cues (text, status) VALUES (?, 'done')"
    ).run("Some done cue");
    db.prepare(
      "INSERT INTO cues (text, status) VALUES (?, 'ready')"
    ).run("Ready but no execution");

    db.close();
  });

  afterAll(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("should list running cues via MCP and find the test itself", () => {
    const prompt = [
      "Use the Dirigent MCP tools to list all running cues.",
      'A "running" cue is one in ready status that has an execution with status running.',
      'Call dirigent_list_cues with status "ready", then for each result check',
      "if it has a running execution via dirigent_get_execution.",
      "Return the results as a JSON array of {id, text, tag} objects.",
      "Only include cues that have a running execution.",
    ].join(" ");

    const mcpServerSrc = path.resolve(__dirname, "../../src/index.ts");
    const mcpConfig = {
      mcpServers: {
        dirigent: {
          type: "stdio",
          command: "npx",
          args: ["tsx", mcpServerSrc, dbPath],
        },
      },
    };
    const mcpConfigPath = path.join(tmpDir, "mcp-config.json");
    fs.writeFileSync(mcpConfigPath, JSON.stringify(mcpConfig));

    const claudeCli = process.env.CLAUDE_CLI_PATH ?? "claude";
    const result = execFileSync(claudeCli, [
      "-p", prompt,
      "--mcp-config", mcpConfigPath,
      "--output-format", "json",
    ], {
      encoding: "utf-8",
      timeout: 55_000,
      env: { ...process.env },
    });

    const response = JSON.parse(result);
    const responseText: string = response.result ?? response.text ?? result;

    const jsonMatch = responseText.match(/\[[\s\S]*?\]/);
    expect(
      jsonMatch,
      "Claude should return a JSON array of running cues"
    ).toBeTruthy();

    const runningCues = JSON.parse(jsonMatch![0]) as Array<{
      id: number;
      text: string;
      tag: string | null;
    }>;

    const selfCue = runningCues.find((c) => c.id === testCueId);
    expect(
      selfCue,
      `Cue #${testCueId} (the test itself) should be in running cues`
    ).toBeDefined();
    expect(selfCue!.text).toBe(TEST_CUE_TEXT);
    expect(selfCue!.tag).toBe("integration-test");

    const falsePositive = runningCues.find(
      (c) => c.text === "Ready but no execution"
    );
    expect(
      falsePositive,
      "Cues without a running execution should be excluded"
    ).toBeUndefined();
  });

  it("should return the test cue when asking Claude about Dirigent running cues in natural language", () => {
    const prompt =
      "What Dirigent cues are currently running? Give me their IDs, text, and tags as JSON.";

    const mcpServerSrc = path.resolve(__dirname, "../../src/index.ts");
    const mcpConfig = {
      mcpServers: {
        dirigent: {
          type: "stdio",
          command: "npx",
          args: ["tsx", mcpServerSrc, dbPath],
        },
      },
    };
    const mcpConfigPath = path.join(tmpDir, "mcp-config.json");
    fs.writeFileSync(mcpConfigPath, JSON.stringify(mcpConfig));

    const claudeCli = process.env.CLAUDE_CLI_PATH ?? "claude";
    const result = execFileSync(claudeCli, [
      "-p", prompt,
      "--mcp-config", mcpConfigPath,
      "--output-format", "json",
    ], {
      encoding: "utf-8",
      timeout: 55_000,
      env: { ...process.env },
    });

    const response = JSON.parse(result);
    const responseText: string = response.result ?? response.text ?? result;

    expect(responseText).toContain(TEST_CUE_TEXT);
    expect(responseText).toContain("integration-test");
  });
});
