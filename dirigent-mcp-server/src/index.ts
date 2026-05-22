#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import Database from "better-sqlite3";
import { z } from "zod";

const DB_PATH = process.argv[2] || process.env.DIRIGENT_DB;
if (!DB_PATH) {
  process.stderr.write(
    "Usage: dirigent-mcp <path-to-Dirigent.db>\n" +
      "   or: DIRIGENT_DB=<path> dirigent-mcp\n"
  );
  process.exit(1);
}

let db: InstanceType<typeof Database>;
try {
  db = new Database(DB_PATH, { readonly: false });
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");
  db.pragma("busy_timeout = 5000");
} catch (err) {
  const msg = err instanceof Error ? err.message : String(err);
  process.stderr.write(`Failed to open database "${DB_PATH}": ${msg}\n`);
  process.exit(1);
}

const REQUIRED_TABLES = ["cues", "executions", "cue_activity_log"] as const;
const existing = db
  .prepare(
    "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN (?, ?, ?)"
  )
  .all(...REQUIRED_TABLES)
  .map((r) => (r as { name: string }).name);
const missing = REQUIRED_TABLES.filter((t) => !existing.includes(t));
if (missing.length > 0) {
  process.stderr.write(
    `Database "${DB_PATH}" is missing required tables: ${missing.join(", ")}\n`
  );
  process.exit(1);
}

const VALID_STATUSES = [
  "inbox",
  "ready",
  "review",
  "done",
  "archived",
  "backlog",
] as const;

type CueStatus = (typeof VALID_STATUSES)[number];

function now(): string {
  return new Date().toISOString().replace("T", " ").substring(0, 19);
}

function logActivity(cueId: number | bigint, event: string): void {
  db.prepare(
    "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?, ?, ?)"
  ).run(cueId, now(), event);
}

const server = new McpServer({
  name: "dirigent",
  version: "0.1.0",
});

// ---------------------------------------------------------------------------
// Cue Management
// ---------------------------------------------------------------------------

server.tool(
  "dirigent_list_cues",
  "List Dirigent cues, optionally filtered by status or tag",
  {
    status: z
      .enum(VALID_STATUSES)
      .optional()
      .describe("Filter by cue status"),
    tag: z.string().optional().describe("Filter by tag"),
    limit: z
      .number()
      .int()
      .positive()
      .max(1000)
      .default(50)
      .describe("Max results to return (up to 1000)"),
  },
  async ({ status, tag, limit }) => {
    let sql =
      "SELECT id, text, file_path, line_number, line_number_end, status, tag FROM cues WHERE 1=1";
    const params: unknown[] = [];
    if (status) {
      sql += " AND status = ?";
      params.push(status);
    }
    if (tag) {
      sql += " AND tag = ?";
      params.push(tag);
    }
    sql += " ORDER BY id DESC LIMIT ?";
    params.push(limit);
    const rows = db.prepare(sql).all(...params);
    return { content: [{ type: "text", text: JSON.stringify(rows, null, 2) }] };
  }
);

server.tool(
  "dirigent_get_cue",
  "Get full details of a single Dirigent cue",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
  },
  async ({ cue_id }) => {
    const cue = db
      .prepare("SELECT * FROM cues WHERE id = ?")
      .get(cue_id) as Record<string, unknown> | undefined;
    if (!cue)
      return {
        content: [{ type: "text", text: `Cue #${cue_id} not found` }],
        isError: true,
      };
    return { content: [{ type: "text", text: JSON.stringify(cue, null, 2) }] };
  }
);

server.tool(
  "dirigent_add_cue",
  "Add a new cue to the Dirigent Inbox",
  {
    text: z.string().min(1).describe("Cue description / task body"),
    file_path: z
      .string()
      .default("")
      .describe("Associated file path (empty for global)"),
    line_number: z
      .number()
      .int()
      .nonnegative()
      .default(0)
      .describe("Start line (0 for global)"),
    line_number_end: z
      .number()
      .int()
      .positive()
      .optional()
      .describe("End line for ranges"),
    tag: z.string().optional().describe("Grouping tag"),
  },
  async ({ text, file_path, line_number, line_number_end, tag }) => {
    const validEnd =
      line_number_end != null && line_number_end >= line_number
        ? line_number_end
        : null;
    const result = db
      .prepare(
        `INSERT INTO cues (text, file_path, line_number, line_number_end, status, tag)
       VALUES (?, ?, ?, ?, 'inbox', ?)`
      )
      .run(text, file_path, line_number, validEnd, tag ?? null);
    const id = result.lastInsertRowid;
    logActivity(id, "Created via MCP");
    return {
      content: [{ type: "text", text: `Created cue #${id} in Inbox` }],
    };
  }
);

server.tool(
  "dirigent_move_cue",
  "Move a Dirigent cue to a different status column",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
    new_status: z
      .enum(VALID_STATUSES)
      .describe("Target status column"),
  },
  async ({ cue_id, new_status }) => {
    const cue = db
      .prepare("SELECT id, status FROM cues WHERE id = ?")
      .get(cue_id) as { id: number; status: string } | undefined;
    if (!cue)
      return {
        content: [{ type: "text", text: `Cue #${cue_id} not found` }],
        isError: true,
      };
    db.prepare("UPDATE cues SET status = ? WHERE id = ?").run(
      new_status,
      cue_id
    );
    const label = new_status.charAt(0).toUpperCase() + new_status.slice(1);
    logActivity(cue_id, `Moved to ${label} via MCP`);
    return {
      content: [{ type: "text", text: `Moved cue #${cue_id} to ${label}` }],
    };
  }
);

server.tool(
  "dirigent_update_cue",
  "Edit a cue's text or tag",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
    text: z.string().optional().describe("New cue text"),
    tag: z.string().optional().describe("New tag value"),
  },
  async ({ cue_id, text, tag }) => {
    const cue = db
      .prepare("SELECT id FROM cues WHERE id = ?")
      .get(cue_id) as { id: number } | undefined;
    if (!cue)
      return {
        content: [{ type: "text", text: `Cue #${cue_id} not found` }],
        isError: true,
      };
    const updates: string[] = [];
    const params: unknown[] = [];
    if (text !== undefined) {
      updates.push("text = ?");
      params.push(text);
    }
    if (tag !== undefined) {
      updates.push("tag = ?");
      params.push(tag);
    }
    if (updates.length === 0)
      return {
        content: [
          { type: "text", text: "No fields to update — provide text or tag" },
        ],
        isError: true,
      };
    params.push(cue_id);
    db.prepare(`UPDATE cues SET ${updates.join(", ")} WHERE id = ?`).run(
      ...params
    );
    logActivity(cue_id, "Updated via MCP");
    return {
      content: [{ type: "text", text: `Updated cue #${cue_id}` }],
    };
  }
);

server.tool(
  "dirigent_delete_cue",
  "Delete a cue and its associated executions and activity log",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
  },
  async ({ cue_id }) => {
    const cue = db
      .prepare("SELECT id FROM cues WHERE id = ?")
      .get(cue_id) as { id: number } | undefined;
    if (!cue)
      return {
        content: [{ type: "text", text: `Cue #${cue_id} not found` }],
        isError: true,
      };
    const deleteCue = db.transaction((id: number) => {
      db.prepare("DELETE FROM cue_activity_log WHERE cue_id = ?").run(id);
      db.prepare("DELETE FROM executions WHERE cue_id = ?").run(id);
      db.prepare("DELETE FROM cues WHERE id = ?").run(id);
    });
    deleteCue(cue_id);
    return {
      content: [{ type: "text", text: `Deleted cue #${cue_id}` }],
    };
  }
);

server.tool(
  "dirigent_archive_done",
  "Archive all cues currently in Done status",
  {},
  async () => {
    const doneCues = db
      .prepare("SELECT id FROM cues WHERE status = 'done'")
      .all() as { id: number }[];
    if (doneCues.length === 0)
      return {
        content: [{ type: "text", text: "No cues in Done to archive" }],
      };
    const archiveAll = db.transaction((cues: { id: number }[]) => {
      const stmt = db.prepare("UPDATE cues SET status = 'archived' WHERE id = ?");
      for (const { id } of cues) {
        stmt.run(id);
        logActivity(id, "Archived via MCP");
      }
    });
    archiveAll(doneCues);
    return {
      content: [
        {
          type: "text",
          text: `Archived ${doneCues.length} cue${doneCues.length === 1 ? "" : "s"}`,
        },
      ],
    };
  }
);

// ---------------------------------------------------------------------------
// Query / Reporting
// ---------------------------------------------------------------------------

server.tool(
  "dirigent_pool_summary",
  "Get a summary of cue counts per status column",
  {},
  async () => {
    const rows = db
      .prepare(
        "SELECT status, COUNT(*) as count FROM cues GROUP BY status ORDER BY status"
      )
      .all();
    return { content: [{ type: "text", text: JSON.stringify(rows, null, 2) }] };
  }
);

server.tool(
  "dirigent_recent_activity",
  "Get recent activity log entries across all cues",
  {
    limit: z
      .number()
      .int()
      .positive()
      .max(1000)
      .default(20)
      .describe("Max entries to return (up to 1000)"),
  },
  async ({ limit }) => {
    const rows = db
      .prepare(
        `SELECT a.id, a.cue_id, a.timestamp, a.event, c.text as cue_text
       FROM cue_activity_log a
       LEFT JOIN cues c ON c.id = a.cue_id
       ORDER BY a.id DESC LIMIT ?`
      )
      .all(limit);
    return { content: [{ type: "text", text: JSON.stringify(rows, null, 2) }] };
  }
);

server.tool(
  "dirigent_cue_history",
  "Get the activity log for a specific cue",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
  },
  async ({ cue_id }) => {
    const rows = db
      .prepare(
        "SELECT id, timestamp, event FROM cue_activity_log WHERE cue_id = ? ORDER BY id"
      )
      .all(cue_id);
    return { content: [{ type: "text", text: JSON.stringify(rows, null, 2) }] };
  }
);

// ---------------------------------------------------------------------------
// Execution History (read-only)
// ---------------------------------------------------------------------------

server.tool(
  "dirigent_get_execution",
  "Get the latest execution for a Dirigent cue",
  {
    cue_id: z.number().int().positive().describe("Cue ID"),
  },
  async ({ cue_id }) => {
    const exec = db
      .prepare(
        `SELECT id, cue_id, status, provider, cost_usd, duration_ms, num_turns
       FROM executions WHERE cue_id = ? ORDER BY id DESC LIMIT 1`
      )
      .get(cue_id) as Record<string, unknown> | undefined;
    if (!exec)
      return {
        content: [
          { type: "text", text: `No executions found for cue #${cue_id}` },
        ],
      };
    return { content: [{ type: "text", text: JSON.stringify(exec, null, 2) }] };
  }
);

server.tool(
  "dirigent_execution_stats",
  "Get aggregate execution statistics (cost, duration, turns)",
  {},
  async () => {
    const stats = db
      .prepare(
        `SELECT
         COUNT(*) as total_executions,
         SUM(cost_usd) as total_cost_usd,
         AVG(cost_usd) as avg_cost_usd,
         SUM(duration_ms) as total_duration_ms,
         AVG(duration_ms) as avg_duration_ms,
         SUM(num_turns) as total_turns,
         AVG(num_turns) as avg_turns
       FROM executions WHERE status = 'completed'`
      )
      .get();
    return {
      content: [{ type: "text", text: JSON.stringify(stats, null, 2) }],
    };
  }
);

// ---------------------------------------------------------------------------
// Start server
// ---------------------------------------------------------------------------

const transport = new StdioServerTransport();
await server.connect(transport);
