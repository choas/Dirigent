# How to Use the Dirigent MCP Server

The Dirigent MCP server (`dirigent-mcp`) exposes the Dirigent SQLite database as a set of MCP tools. This lets Claude Code (or any MCP-capable client) read and manage cues, query execution history, and inspect pool status — all without opening the Dirigent GUI.

## Prerequisites

- **Node.js** v18+ (the server is a TypeScript/Node project)
- **Dirigent** must have been run at least once on the target repository so that a `.Dirigent/Dirigent.db` file exists

## Installation

```bash
cd dirigent-mcp-server
npm install
npm run build
npm link          # makes `dirigent-mcp` available globally
```

Verify the installation:

```bash
dirigent-mcp --help
# Usage: dirigent-mcp <path-to-Dirigent.db>
```

## Running Standalone

```bash
dirigent-mcp /path/to/your/project/.Dirigent/Dirigent.db
```

The server communicates over **stdio** using the MCP protocol. It opens the database in read-write mode with WAL journaling, so Dirigent GUI and the MCP server can access the same DB concurrently.

You can also set the path via environment variable:

```bash
export DIRIGENT_DB=/path/to/your/project/.Dirigent/Dirigent.db
dirigent-mcp
```

## Configuring for Claude Code

### Option A: Claude Code `--mcp-config` (standalone)

Create or edit a file like `.Dirigent/mcp-config.json`:

```json
{
  "mcpServers": {
    "dirigent": {
      "type": "stdio",
      "command": "dirigent-mcp",
      "args": ["/path/to/your/project/.Dirigent/Dirigent.db"]
    }
  }
}
```

Then launch Claude Code with:

```bash
claude --mcp-config .Dirigent/mcp-config.json
```

### Option B: Automatic injection from Dirigent GUI

When you run a cue through the Dirigent GUI using the Claude provider, the MCP server is **automatically injected** if:

1. The prompt text contains the word **"Dirigent"** (case-insensitive)
2. The `dirigent-mcp` binary is found on `PATH` (or a custom path is set in Settings)
3. A `Dirigent.db` exists at `.Dirigent/Dirigent.db` in the project root (or a custom DB path is set in Settings)

When all three conditions are met, Dirigent writes a temporary `mcp-config.json` to `.Dirigent/` and passes `--mcp-config` to the Claude CLI automatically.

**Settings fields** (in Dirigent GUI > Settings):

| Field | Description | Default |
|---|---|---|
| `dirigent_mcp_server_path` | Path to `dirigent-mcp` binary | `""` (uses `dirigent-mcp` from PATH) |
| `dirigent_mcp_db_path` | Path to Dirigent.db | `""` (uses `.Dirigent/Dirigent.db` in project root) |

### Option C: Claude Code project settings

Add the MCP server to your Claude Code project settings at `.claude/settings.json`:

```json
{
  "mcpServers": {
    "dirigent": {
      "type": "stdio",
      "command": "dirigent-mcp",
      "args": ["/path/to/your/project/.Dirigent/Dirigent.db"]
    }
  }
}
```

## Available Tools

Once connected, the following MCP tools are available:

### Cue Management

| Tool | Description | Key Parameters |
|---|---|---|
| `dirigent_list_cues` | List cues, optionally filtered | `status`, `tag`, `limit` |
| `dirigent_get_cue` | Get full details of a single cue | `cue_id` |
| `dirigent_add_cue` | Create a new cue in Inbox | `text`, `file_path`, `line_number`, `tag` |
| `dirigent_move_cue` | Move a cue to a different status column | `cue_id`, `new_status` |
| `dirigent_update_cue` | Edit a cue's text or tag | `cue_id`, `text`, `tag` |
| `dirigent_delete_cue` | Delete a cue and its history | `cue_id` |
| `dirigent_archive_done` | Archive all cues in Done status | *(none)* |

**Valid statuses:** `inbox`, `ready`, `review`, `done`, `archived`, `backlog`

### Query & Reporting

| Tool | Description | Key Parameters |
|---|---|---|
| `dirigent_pool_summary` | Cue counts per status column | *(none)* |
| `dirigent_recent_activity` | Recent activity log entries | `limit` |
| `dirigent_cue_history` | Activity log for a specific cue | `cue_id` |
| `dirigent_get_execution` | Latest execution for a cue | `cue_id` |
| `dirigent_execution_stats` | Aggregate cost/duration/turns stats | *(none)* |

## Example Usage in Claude Code

Once the MCP server is configured, you can ask Claude to interact with cues directly:

```
> Show me all cues in the ready column
  → Claude calls dirigent_list_cues with status="ready"

> Add a cue: "Refactor the auth module for clarity"
  → Claude calls dirigent_add_cue with text="Refactor the auth module for clarity"

> Move cue #12 to done
  → Claude calls dirigent_move_cue with cue_id=12, new_status="done"

> What's the pool status?
  → Claude calls dirigent_pool_summary

> How much have executions cost so far?
  → Claude calls dirigent_execution_stats
```

## Development

```bash
cd dirigent-mcp-server

# Run in development mode (no build step)
tsx src/index.ts /path/to/Dirigent.db

# Run tests
npm test                   # unit tests
npm run test:integration   # integration tests

# Rebuild after changes
npm run build
```

## Troubleshooting

**`dirigent-mcp` not found:** Run `npm link` inside `dirigent-mcp-server/` to install the binary globally.

**Database missing required tables:** Ensure Dirigent has been run at least once on the project to create the schema (tables: `cues`, `executions`, `cue_activity_log`).

**MCP not injected from Dirigent GUI:** Check that your prompt contains the word "Dirigent" and that the binary is on PATH. Look at the Dirigent log output for warnings like `"Dirigent MCP server binary not found"`.
