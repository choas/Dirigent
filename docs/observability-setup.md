# Observability Setup — OpenTelemetry + Grafana

This guide walks you through setting up local observability for both
**Claude Code** and **Dirigent** using a Grafana LGTM stack
(Loki + Grafana + Tempo + Prometheus), with Grafana Alloy as the
OpenTelemetry collector.

All data stays on your machine. No cloud accounts needed.

---

## Architecture

```
┌──────────────┐   OTLP/gRPC :4317   ┌──────────────┐
│  Claude Code │ ─────────────────────▶│              │──▶ Loki   (logs)
└──────────────┘                      │  Grafana     │──▶ Tempo  (traces)
                   OTLP/HTTP :4318    │  Alloy       │──▶ Prometheus (metrics)
┌──────────────┐ ─────────────────────▶│  (collector) │
│   Dirigent   │                      └──────────────┘
└──────────────┘                             │
                                      ┌──────────────┐
                                      │   Grafana    │  ← dashboards
                                      │  :3001       │
                                      └──────────────┘
```

**Claude Code** sends traces, logs, and metrics via OTLP/gRPC (`:4317`).
**Dirigent** sends structured OTLP log events via HTTP (`:4318`).

---

## Prerequisites

| Requirement | Check | Install |
|---|---|---|
| Docker Desktop (or Podman + podman-compose) | `docker compose version` | [docker.com/products/docker-desktop](https://www.docker.com/products/docker-desktop/) |
| Claude Code CLI (optional, for Claude telemetry) | `claude --version` | `npm install -g @anthropic-ai/claude-code` |
| ~1 GB disk for container images | — | first `docker compose pull` will download them |

---

## Quick Start (automated)

The setup script handles everything — config files, Docker Compose, env vars:

```bash
cd /path/to/Dirigent
zsh scripts/claude-otel-local.zsh
```

The script is interactive and will:

1. Verify Docker/Podman is installed
2. Create config files in `~/.config/claude-otel-local/`
3. Generate the Grafana Alloy collector config
4. Write a `docker-compose.yml` for the full LGTM stack
5. Start all services
6. Write env vars and optionally add them to `~/.zshrc`

After completion, open **http://localhost:3001** to view Grafana.

---

## Manual Setup (step by step)

If you prefer to understand each piece, here's what the script does.

### 1. Create the stack directory

```bash
mkdir -p ~/.config/claude-otel-local
cd ~/.config/claude-otel-local
```

### 2. Config files

The script creates config for each service under subdirectories:

| File | Purpose |
|---|---|
| `prometheus/prometheus.yml` | Scrape config for Alloy and self-monitoring |
| `loki/loki.yml` | Loki log storage (filesystem backend, in-memory ring) |
| `tempo/tempo.yml` | Tempo trace storage (local filesystem, OTLP receiver) |
| `alloy.alloy` | Grafana Alloy — OTLP receiver that fans out to Loki/Tempo/Prometheus |
| `grafana/provisioning/datasources/datasources.yml` | Auto-provisions Loki, Tempo, Prometheus datasources |
| `grafana/provisioning/dashboards/dashboards.yml` | Auto-provisions dashboard directory |
| `grafana/dashboards/claude-code.json` | Pre-built Claude Code Usage dashboard |
| `grafana/dashboards/dirigent.json` | Pre-built Dirigent Usage dashboard |

### 3. Start the stack

```bash
cd ~/.config/claude-otel-local
docker compose pull
docker compose up -d
```

Wait for all services to be healthy:

```bash
# Check readiness
curl -sf http://localhost:3100/ready   # Loki
curl -sf http://localhost:3200/ready   # Tempo
curl -sf http://localhost:9090/-/ready # Prometheus
curl -sf http://localhost:12345/       # Alloy UI
curl -sf http://localhost:3001/api/health  # Grafana
```

### 4. Set environment variables

Source the generated env file before running Claude Code or Dirigent:

```bash
source ~/.config/claude-otel-local/claude-otel.env
```

Or set the variables manually:

```bash
# ── Claude Code ──────────────────────────────────────────────
export BETA_TRACING_ENDPOINT="http://localhost:4318"
export CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1
export ENABLE_BETA_TRACING_DETAILED=1
export CLAUDE_CODE_ENABLE_TELEMETRY=1
export CLAUDE_CODE_OTEL_FLUSH_TIMEOUT_MS=1000
export OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4317"
export OTEL_EXPORTER_OTLP_PROTOCOL="grpc"
export OTEL_LOGS_EXPORT_INTERVAL=1000
export OTEL_METRIC_EXPORT_INTERVAL=1000
export OTEL_LOGS_EXPORTER="otlp"
export OTEL_METRICS_EXPORTER="otlp"
export OTEL_TRACES_EXPORTER="otlp"

# ── Dirigent ─────────────────────────────────────────────────
export DIRIGENT_OTEL_ENDPOINT="http://localhost:4318"
```

To persist across terminal sessions, add to `~/.zshrc`:

```bash
echo 'source ~/.config/claude-otel-local/claude-otel.env' >> ~/.zshrc
```

#### Optional: extended logging

These capture tool content, tool details, and user prompts (more verbose, still local):

```bash
export OTEL_LOG_TOOL_CONTENT=1
export OTEL_LOG_TOOL_DETAILS=1
export OTEL_LOG_USER_PROMPTS=1
```

---

## Running with telemetry

### Claude Code

```bash
source ~/.config/claude-otel-local/claude-otel.env
claude
```

### Dirigent

```bash
source ~/.config/claude-otel-local/claude-otel.env
Dirigent /path/to/project
```

Or set `DIRIGENT_OTEL_ENDPOINT` per-project in your project's `.env` file.

---

## Grafana Dashboards

Open **http://localhost:3001** (no login required — anonymous admin is enabled).

### Claude Code Usage dashboard

Navigate to **Dashboards → Claude Code → Claude Code Usage**

| Panel | What it shows |
|---|---|
| Total Cost (USD) | Cumulative spend across all sessions |
| Input / Output / Cache Read Tokens | Token counters |
| Cost over time | Per-session cost broken down by `session_id` |
| Token usage over time | Input, output, and cache_read tokens over time |
| Log stream | Raw log events from Claude Code |

### Dirigent Usage dashboard

Navigate to **Dashboards → Claude Code → Dirigent Usage**

| Panel | What it shows |
|---|---|
| Total Cost (USD) | Cumulative execution cost |
| Input / Output Tokens | Token counters from executions |
| Total Executions | Count of completed executions |
| Cost over time by project | Per-project cost breakdown |
| Token usage over time | Input and output token series |
| Execution duration (ms) | Bar chart of execution durations |
| Executions by provider | Grouped by provider (e.g. claude, opencode) |
| Agent runs by kind | Agent execution counts by kind |
| Failures & rate limits | Error and rate-limit event counts |
| Log stream | Raw Dirigent log events |

---

## Exploring data in Grafana

### Using Explore (ad-hoc queries)

Go to **Explore** (compass icon in left sidebar), select the **Loki** datasource.

**Claude Code events:**
```logql
{service_name="claude-code"}
```

**Dirigent events:**
```logql
{service_name="dirigent"}
```

**Filter by event type:**
```logql
{service_name="dirigent"} |= "execution.completed"
```

**Extract and aggregate fields:**
```logql
sum_over_time(
  {service_name="dirigent"}
    |= "execution.completed"
    | json cost_usd="attributes.cost_usd"
    | unwrap cost_usd
  [1h]
)
```

### Dirigent event types

| Event | Attributes |
|---|---|
| `app.started` | `project` |
| `execution.started` | `project`, `cue_id`, `provider`, `model` |
| `execution.completed` | `project`, `cue_id`, `provider`, `cost_usd`, `duration_ms`, `num_turns`, `input_tokens`, `output_tokens`, `has_diff` |
| `execution.failed` | `project`, `cue_id`, `provider`, `error` |
| `execution.rate_limited` | `project`, `cue_id`, `message` |
| `cue.status_changed` | `project`, `cue_id`, `from_status`, `to_status` |
| `agent.completed` | `project`, `agent_kind`, `status`, `duration_ms`, `cue_id` (optional) |
| `git.commit` | `project`, `files_changed` |

All events also include a `session_id` attribute (unique per Dirigent app launch).

---

## Service URLs

| Service | URL | Purpose |
|---|---|---|
| Grafana | http://localhost:3001 | Dashboards and Explore |
| Prometheus | http://localhost:9090 | Metrics storage and PromQL |
| Alloy UI | http://localhost:12345 | Collector pipeline status |
| Loki | http://localhost:3100 | Log storage (API only) |
| Tempo | http://localhost:3200 | Trace storage (API only) |

---

## Common Operations

### Check stack status

```bash
cd ~/.config/claude-otel-local
docker compose ps
```

### View collector logs

```bash
cd ~/.config/claude-otel-local
docker compose logs -f alloy
```

### Stop the stack (preserves data)

```bash
cd ~/.config/claude-otel-local
docker compose stop
```

### Restart the stack

```bash
cd ~/.config/claude-otel-local
docker compose up -d
```

### Remove everything (including data)

```bash
cd ~/.config/claude-otel-local
docker compose down -v
```

---

## Troubleshooting

### No data appearing in Grafana

1. **Check the stack is running:** `docker compose ps` — all 5 containers should be `Up`
2. **Check env vars are set:** `echo $DIRIGENT_OTEL_ENDPOINT` should print `http://localhost:4318`
3. **Check Alloy is receiving data:** Open http://localhost:12345 and look at the pipeline graph
4. **Check Loki directly:** `curl -s 'http://localhost:3100/loki/api/v1/query?query={service_name="dirigent"}&limit=5'` should return recent log entries
5. **Check Alloy logs:** `docker compose logs alloy` — look for connection errors

### Port conflicts

If ports 3001, 3100, 4317, 4318, 9090, or 12345 are already in use, edit `docker-compose.yml` and change the host-side port mappings. Update the env file to match.

### Dirigent not sending events

- `DIRIGENT_OTEL_ENDPOINT` must be set **before** launching Dirigent (it's read once at startup via `telemetry::init()`)
- The endpoint should be `http://localhost:4318` (HTTP, not gRPC)
- Events are sent on background threads — they won't block the UI but require the collector to be reachable

### Claude Code not sending events

- All `OTEL_*` and `CLAUDE_CODE_*` env vars must be set **before** running `claude`
- Claude Code uses gRPC on port 4317 by default
- Check `OTEL_EXPORTER_OTLP_ENDPOINT` is set to `http://localhost:4317`
