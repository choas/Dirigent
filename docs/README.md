# Documentation

An index of all Dirigent documentation. The core guides follow the [Diátaxis](https://diataxis.fr/) framework (tutorial, how-to, reference, explanation); the remaining pages cover specific integrations and workflows.

## Core guides

| Document | Type | Description |
| --- | --- | --- |
| [Getting Started with Dirigent](tutorial.md) | Tutorial | Install Dirigent, open a project, create a cue, run it with Claude Code, and review the result. |
| [How-To Guides](how-to.md) | How-to | Task-oriented recipes for common Dirigent workflows, each section self-contained. |
| [Reference](reference.md) | Reference | Complete reference for settings, keyboard shortcuts, cue commands, playbook, CLI usage, environment variables, and per-project data. |
| [Explanation](explanation.md) | Explanation | Understanding Dirigent's architecture, design decisions, and core concepts. |

## Integrations & workflows

| Document | Description |
| --- | --- |
| [Git Workflows](git-workflows.md) | AI-assisted commit splitting, hunk-level staging, cross-branch review, and worktree tooling. |
| [jj Demo Walkthrough](jj-demo-walkthrough.md) | Build a Node.js CLI app from scratch using Dirigent and jj; doubles as an integration test. |
| [How to Use the Dirigent MCP Server](How_to_Dirigent_MCP.md) | Expose the Dirigent SQLite database as MCP tools for Claude Code or any MCP-capable client. |
| [OpenCode Integration](How_to_OpenCode.md) | Configure and use OpenCode as an alternative CLI backend to Claude Code. |
| [How to use SonarQube with Dirigent](How_to_use_SonarQube.md) | Pull SonarQube issues, security hotspots, and duplication data in as cues. |
| [Observability Setup](observability-setup.md) | Local OpenTelemetry + Grafana LGTM observability for Claude Code and Dirigent. |
| [Claude PTY recording and replay](claude-pty-recording.md) | Optional JSON-lines PTY recording via `ClaudeCodeBuilder::record_to(path)`. |

## Related docs (repository root)

| Document | Description |
| --- | --- |
| [README](../README.md) | Project overview, features, and installation. |
| [CHANGELOG](../CHANGELOG.md) | Release history and notable changes. |
| [How to add cues via SQLite to Dirigent](../How_to_add_cues_via_SQLite_to_Dirigent.md) | Insert cues directly into the Dirigent SQLite database. |
| [How to jj](../How_to_jj.md) | Using Jujutsu (jj) with Dirigent. |
