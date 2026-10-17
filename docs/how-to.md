# How-To Guides

Task-oriented recipes for common Dirigent workflows. Each section is self-contained.

---

## Table of Contents

- [Use cue commands for specialized tasks](#use-cue-commands-for-specialized-tasks)
- [Run a playbook prompt](#run-a-playbook-prompt)
- [Send follow-up feedback (reply workflow)](#send-follow-up-feedback-reply-workflow)
- [Attach images to a cue](#attach-images-to-a-cue)
- [Import cues from external sources](#import-cues-from-external-sources)
- [Import cues from a Markdown file](#import-cues-from-a-markdown-file)
- [Set up post-run agents](#set-up-post-run-agents)
- [Create a GitHub pull request](#create-a-github-pull-request)
- [Import PR review findings as cues](#import-pr-review-findings-as-cues)
- [Use workflow planning for multiple cues](#use-workflow-planning-for-multiple-cues)
- [Work with Git worktrees](#work-with-git-worktrees)
- [Switch the AI backend to OpenCode](#switch-the-ai-backend-to-opencode)
- [Run an ACP agent (Goose, Gemini CLI, …)](How_to_ACP.md)
- [Configure LSP for code intelligence](#configure-lsp-for-code-intelligence)
- [Split a large cue into smaller ones](#split-a-large-cue-into-smaller-ones)
- [Read history in the graph view](#read-history-in-the-graph-view)
- [Use Jujutsu (jj) instead of Git](#use-jujutsu-jj-instead-of-git)
- [Browse a remote host over SSH](#browse-a-remote-host-over-ssh)
- [Create a custom theme](#create-a-custom-theme)
- [Let an agent manage cues via the MCP server](#let-an-agent-manage-cues-via-the-mcp-server)
- [Set up observability](#set-up-observability)
- [Build a macOS .app bundle](#build-a-macos-app-bundle)

---

## Use cue commands for specialized tasks

Prefix your cue text with a command in brackets to apply a specialized prompt template.

1. Create a cue (select lines or use the prompt field)
2. Start the text with a command tag, e.g.: `[test] Add unit tests for the parse_config function`
3. Run the cue

Available commands: `[plan]`, `[test]`, `[refactor]`, `[review]`, `[fix]`, `[docs]`, `[explain]`, `[optimize]`.

The `[plan]` command runs Claude in plan-only mode (no code changes). The `[review]` and `[explain]` commands ask for analysis without modifications. All others produce code changes.

You can customize commands or add your own in **Settings > Commands**.

## Run a playbook prompt

Playbook prompts are predefined tasks you can run without writing a prompt from scratch.

1. Open the cue pool
2. Click the **Playbook** button (book icon) at the top
3. Select a play, e.g. "Security audit" or "Add tests"
4. If the play has template variables (e.g. version number), fill them in the dialog
5. The play creates a global cue and optionally runs it immediately

Built-in plays include: Documentation (Diataxis), Verify architecture, Git Pull, Create release, Security audit, Check dead code, Add tests, Fix all warnings, Commit changes, Zero day test, Pin dependency versions.

## Send follow-up feedback (reply workflow)

After reviewing a diff, you can refine it iteratively without creating a new cue.

1. Open the diff review for a cue in Review status
2. Type feedback in the **Reply** field at the bottom, e.g.: `The validation is good but also check for empty strings`
3. Click **Send** — Claude receives your feedback along with the conversation history
4. A new diff is generated; review it again

You can reply multiple times until the result is satisfactory, then accept or reject.

## Attach images to a cue

You can attach screenshots or diagrams for Claude to reference.

**Method 1 — Drag and drop:**
- Drag an image file onto the Dirigent window; it attaches to the currently selected cue

**Method 2 — Attachment button:**
1. Click the attachment icon on a cue card
2. Select an image file from the file dialog

Supported formats: PNG, JPEG, GIF, BMP, WebP, ICO.

## Import cues from external sources

Dirigent can pull tasks from external tools and create cues automatically.

1. Open **Settings > Sources**
2. Add a source — supported types:
   - **GitHub Issues** — requires a `gh` CLI or personal access token
   - **Slack** — channel messages as cues
   - **SonarQube** — code quality findings
   - **Notion** — database entries
   - **Trello** — card imports
   - **Asana** — task imports
   - **MCP** — Model Context Protocol servers
   - **Custom** — any shell command that outputs JSON
3. Configure the connection details (URL, token, project, etc.)
4. Sources are polled in the background; new items appear in the Inbox
5. Deduplication prevents the same item from being imported twice

## Import cues from a Markdown file

Batch-create cues from a Markdown document where each heading becomes a cue.

1. In the cue pool, click the import button
2. Select a `.md` file — defaults to browsing from the project folder
3. Each heading (`#`, `##`, etc.) becomes a separate cue title, with the body text as the cue content
4. Upsert logic avoids duplicates if you re-import the same file

## Set up post-run agents

Agents run automatically after Claude finishes (or on other triggers) to format, lint, build, or test.

1. Open **Settings > Agents**
2. Define agents with:
   - **Name** — e.g. "Format", "Lint", "Build", "Test"
   - **Command** — the shell command to run, e.g. `cargo fmt`, `npm run lint`
   - **Trigger** — when to run: `AfterRun`, `AfterCommit`, `AfterAgent` (chaining), `OnFileChange`, or `Manual`
3. Click **Initialize** to populate agents with language-specific defaults for your project
4. After a cue runs, triggered agents execute automatically; results appear in the agent log viewer
5. Cargo diagnostic output is parsed and displayed with file/line references

Use `agent_shell_init` in settings to prepend shell initialization (e.g. `source ~/.zshrc`) if agents can't find tools when launched from the GUI.

## Create a GitHub pull request

Create a PR directly from Dirigent after committing changes.

1. Commit your changes (via the Done column or Git integration)
2. Open the **Create PR** dialog from the Git menu or cue pool
3. Fill in:
   - **Title** — short PR title
   - **Base branch** — the branch to merge into (defaults to main)
   - **Description** — PR body text
   - **Draft** — toggle to create as a draft PR
4. Click **Create** — Dirigent uses the `gh` CLI to create the PR

Requires the GitHub CLI (`gh`) to be installed and authenticated.

## Import PR review findings as cues

Turn automated review comments (e.g. from CodeRabbit or Qodo) into actionable cues.

1. Open the **Import PR** dialog from the Git menu
2. Select a PR number
3. Dirigent fetches review findings and displays them in a filter dialog
4. Check the findings you want to import — each becomes a cue with the file path, line number, and review text
5. Imported cues land in Inbox, ready to run

HTML tags and non-actionable summary headers are automatically stripped.

## Use workflow planning for multiple cues

When you have several cues in Inbox, Dirigent can analyze dependencies and plan execution order.

1. Place multiple cues in the **Inbox** column
2. Click the **Plan Workflow** button
3. Claude analyzes the cues and produces an execution plan:
   - Independent cues are grouped for parallel execution
   - Dependent cues are ordered sequentially
4. A visual workflow graph overlay shows the plan with step-by-step progress
5. Execute the workflow — cues run in the planned order

## Work with Git worktrees

Worktrees let you work on multiple branches simultaneously without switching.

1. Open the **Worktree Manager** from the Git menu
2. Create a new worktree — specify a branch name and path
3. Each worktree gets its own `.Dirigent/` directory with a separate database
4. Switch between worktrees from the repository bar
5. When you remove a worktree, its database is archived under `.Dirigent/archives/`

## Switch the AI backend to OpenCode

Dirigent supports OpenCode as an alternative to Claude Code, enabling models from OpenAI, Google, and Anthropic.

1. Open **Settings > General**
2. Change **CLI Provider** from Claude to OpenCode
3. Configure the OpenCode model (e.g. `openai/o3`, `google/gemini-2.5-pro`)
4. Set the CLI path if `opencode` is not on your PATH
5. Click **Test Connection** to check that OpenCode starts and answers. This works for every provider.

The rest of the workflow (cues, diffs, review) works the same regardless of provider.

## Configure LSP for code intelligence

LSP provides diagnostics, hover info, go-to-definition, and find-references.

1. Open **Settings > LSP**
2. Enable the **LSP** toggle
3. Dirigent ships with presets for 13 languages: Rust, TypeScript, Python, Go, Java, C#, C/C++, Ruby, Swift, Kotlin, Elixir, Zig, Lua
4. Each preset specifies the language server binary (e.g. `rust-analyzer`, `typescript-language-server`)
5. Ensure the language server is installed on your system
6. Add custom server configurations if needed

Once enabled, diagnostics appear as inline markers in the code viewer, and Cmd+click on a symbol triggers go-to-definition.

## Split a large cue into smaller ones

A cue that describes several unrelated changes runs badly: the agent has to
hold all of them at once, and the resulting diff is hard to review.

1. Open the cue's overflow menu (`⋯`) in the Inbox or Backlog column
2. Choose **✂ Split**
3. Dirigent sends the cue text to the configured AI backend as a read-only
   analysis prompt and proposes a set of sub-cues
4. Review the proposal; accepted sub-cues are created in the same column and
   the original is archived

Split is only offered for Inbox and Backlog cues — once a cue is Ready or
running, splitting it would orphan the run.

## Read history in the graph view

The sidebar's Git log shows a compact lane graph. For a full-window view:

1. Click the **📊** button in the Git log section header
2. Commits load 100 at a time; scrolling near the bottom loads more
3. Hover a row to highlight that branch's lineage through the graph
4. Click a commit to open its diff in the review pane
5. **⇄** in the header opens *Analyze changes over…*, which groups the diff
   between two refs into reviewable change sets

## Use Jujutsu (jj) instead of Git

Dirigent drives either backend through the same UI.

1. Dirigent switches automatically when the project has a `.jj` directory
2. Otherwise set **Settings > Version control > VCS backend** to `Jj`; on a
   git-only repository Dirigent offers to run `jj git init --colocate`
3. The Git menu becomes the jj menu: commit describes the working copy and
   starts a new change, and branches become bookmarks
4. Per-cue workspaces replace worktrees — see
   [Work with Git worktrees](#work-with-git-worktrees) for the equivalent flow

A longer walkthrough lives in [jj-demo-walkthrough.md](jj-demo-walkthrough.md).

## Browse a remote host over SSH

1. Open **Settings > SSH Servers** and add a host (name, host, username)
2. Use **Test connection** to verify SSH *and* SFTP reachability
3. Pick the server from the **SSH** menu to replace the file tree with the
   remote host's files
4. Files open read-only in the viewer; cues you create on them stay local

## Create a custom theme

1. Open **Settings > General**
2. Click **+** next to the theme dropdown — the new theme starts as a copy of
   the current one
3. Adjust the colors; **✎** re-opens an existing custom theme for editing
4. Custom themes are repo-scoped and apply without a trust prompt, since they
   cannot cause anything to execute

## Let an agent manage cues via the MCP server

Dirigent exposes its cue database over MCP, so a run can list, add and move
cues while it works.

1. Mention "dirigent" anywhere in the cue or prompt text — that is the trigger
2. Dirigent writes an MCP config into `.Dirigent/` and passes it to the run
3. Override the server binary or database in **Settings > Dirigent MCP server**
   if they are not where Dirigent expects them

Full tool list: [How_to_Dirigent_MCP.md](How_to_Dirigent_MCP.md).

## Set up observability

See the dedicated guide: [Observability Setup](observability-setup.md).

In short: set `DIRIGENT_OTEL_ENDPOINT=http://localhost:4318` before launching Dirigent, and run the Grafana LGTM stack via Docker. Dirigent emits structured events for executions, cue transitions, agent runs, and git commits.

## Build a macOS .app bundle

A Makefile provides targets for the full macOS distribution workflow:

```bash
make build                    # cargo build --release
make bundle                   # Create .app bundle with icon and Info.plist
make sign IDENTITY="Developer ID Application: Your Name (TEAMID)"
make dmg                      # Create DMG (requires create-dmg)
make notarize APPLE_ID=you@example.com TEAM_ID=TEAMID
```

Pushing a `v*` tag to GitHub also triggers automated build, sign, notarize, and release via GitHub Actions.
