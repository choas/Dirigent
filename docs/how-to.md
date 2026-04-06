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
- [Configure LSP for code intelligence](#configure-lsp-for-code-intelligence)
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

Built-in plays include: Documentation (Diataxis), Verify architecture, Verify last 5 commits, Create release, Security audit, Check dead code, Add tests, Fix all warnings, Commit changes, Pin dependency versions.

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
