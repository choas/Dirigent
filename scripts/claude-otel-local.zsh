#!/usr/bin/env zsh
# =============================================================================
# Dirigent + Claude Code  →  OpenTelemetry  →  Local Grafana (LGTM Stack)
# Loki · Grafana · Tempo · Prometheus · Alloy  —  everything on your machine
# Based on: https://braw.dev/blog/2026-03-28-monitor-claude-usage-with-grafana/
# =============================================================================

set -euo pipefail

# ─── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'

# ─── Helpers ──────────────────────────────────────────────────────────────────
step()    { echo "\n${BOLD}${BLUE}▶ $*${RESET}" }
ok()      { echo "  ${GREEN}✓${RESET} $*" }
warn()    { echo "  ${YELLOW}⚠${RESET}  $*" }
fail()    { echo "${RED}✗ $*${RESET}"; exit 1 }
info()    { echo "  ${CYAN}i${RESET} $*" }

confirm() {
  echo -n "  ${CYAN}?${RESET} $* [y/N] "
  read -r reply; [[ "${reply:l}" == "y" ]]
}

# ─── Banner ───────────────────────────────────────────────────────────────────
clear
echo "${BOLD}${BLUE}"
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  Dirigent + Claude Code  →  OpenTelemetry  →  Grafana      ║"
echo "║         Loki · Grafana · Tempo · Prometheus · Alloy         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "${RESET}"
echo "  Your data stays on your machine. No cloud accounts needed."
echo ""
echo "  This script will:"
echo "    1. Verify prerequisites  (Docker / Podman + Compose)"
echo "    2. Create the stack directory and all config files"
echo "    3. Generate the Grafana Alloy OTEL collector config"
echo "    4. Build a docker-compose.yml for the full LGTM stack"
echo "    5. Start all services"
echo "    6. Write and optionally persist the env vars (Claude Code + Dirigent)"
echo ""

confirm "Ready to continue?" || { echo "Aborted."; exit 0 }

# ═════════════════════════════════════════════════════════════════════════════
# STEP 1 — Prerequisites
# ═════════════════════════════════════════════════════════════════════════════
step "Step 1 / 6 — Checking prerequisites"

# Container runtime
CONTAINER_CMD=""
COMPOSE_CMD=""

if command -v podman &>/dev/null; then
  CONTAINER_CMD="podman"
  if command -v podman-compose &>/dev/null; then
    COMPOSE_CMD="podman-compose"
    ok "Podman + podman-compose found"
  else
    fail "Podman found but podman-compose is missing. Install with: pip3 install podman-compose"
  fi
elif command -v docker &>/dev/null; then
  CONTAINER_CMD="docker"
  if docker compose version &>/dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
    ok "Docker + Compose plugin found"
  elif command -v docker-compose &>/dev/null; then
    COMPOSE_CMD="docker-compose"
    ok "Docker + docker-compose (standalone) found"
  else
    fail "Docker found but Compose is missing. Install Docker Desktop or 'brew install docker-compose'"
  fi
else
  echo ""
  echo "  ${RED}No container runtime found.${RESET}"
  echo "  Install Docker Desktop:  https://www.docker.com/products/docker-desktop/"
  fail "Missing container runtime"
fi

if command -v claude &>/dev/null; then
  ok "Claude Code CLI found"
else
  warn "Claude Code CLI not found — env vars will still be written for later use."
fi

# Stack directory
BASE_DIR="${HOME}/.config/claude-otel-local"
mkdir -p "$BASE_DIR"
ok "Working directory: ${BASE_DIR}"

# ═════════════════════════════════════════════════════════════════════════════
# STEP 2 — Config files
# ═════════════════════════════════════════════════════════════════════════════
step "Step 2 / 6 — Writing service config files"

# ── Prometheus ────────────────────────────────────────────────────────────────
mkdir -p "${BASE_DIR}/prometheus"
cat > "${BASE_DIR}/prometheus/prometheus.yml" <<'PROM'
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: alloy
    static_configs:
      - targets: ['alloy:12345']

  - job_name: prometheus
    static_configs:
      - targets: ['localhost:9090']
PROM
ok "Prometheus config written"

# ── Loki ──────────────────────────────────────────────────────────────────────
mkdir -p "${BASE_DIR}/loki"
cat > "${BASE_DIR}/loki/loki.yml" <<'LOKI'
auth_enabled: false

server:
  http_listen_port: 3100
  grpc_listen_port: 9096

common:
  instance_addr: 127.0.0.1
  path_prefix: /tmp/loki
  storage:
    filesystem:
      chunks_directory: /tmp/loki/chunks
      rules_directory: /tmp/loki/rules
  replication_factor: 1
  ring:
    kvstore:
      store: inmemory

query_range:
  results_cache:
    cache:
      embedded_cache:
        enabled: true
        max_size_mb: 100

schema_config:
  configs:
    - from: 2024-01-01
      store: tsdb
      object_store: filesystem
      schema: v13
      index:
        prefix: index_
        period: 24h

ruler:
  alertmanager_url: http://localhost:9093

limits_config:
  allow_structured_metadata: true
  volume_enabled: true
LOKI
ok "Loki config written"

# ── Tempo ─────────────────────────────────────────────────────────────────────
mkdir -p "${BASE_DIR}/tempo"
cat > "${BASE_DIR}/tempo/tempo.yml" <<'TEMPO'
server:
  http_listen_port: 3200

distributor:
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: 0.0.0.0:4317
        http:
          endpoint: 0.0.0.0:4318

storage:
  trace:
    backend: local
    local:
      path: /tmp/tempo/blocks
    wal:
      path: /tmp/tempo/wal

metrics_generator:
  registry:
    external_labels:
      source: tempo
  storage:
    path: /tmp/tempo/generator/wal

overrides:
  defaults:
    metrics_generator:
      processors: [service-graphs, span-metrics]
TEMPO
ok "Tempo config written"

# ── Grafana datasource provisioning ───────────────────────────────────────────
mkdir -p "${BASE_DIR}/grafana/provisioning/datasources"
cat > "${BASE_DIR}/grafana/provisioning/datasources/datasources.yml" <<'DS'
apiVersion: 1

datasources:
  - name: Loki
    type: loki
    uid: loki
    access: proxy
    url: http://loki:3100
    isDefault: true
    jsonData:
      derivedFields:
        - datasourceUid: tempo
          matcherRegex: '"trace_id":"(\w+)"'
          name: TraceID
          url: "$${__value.raw}"

  - name: Tempo
    type: tempo
    uid: tempo
    access: proxy
    url: http://tempo:3200
    jsonData:
      serviceMap:
        datasourceUid: prometheus
      lokiSearch:
        datasourceUid: loki

  - name: Prometheus
    type: prometheus
    uid: prometheus
    access: proxy
    url: http://prometheus:9090
DS
ok "Grafana datasources provisioned"

# ── Grafana dashboard provisioning ────────────────────────────────────────────
mkdir -p "${BASE_DIR}/grafana/provisioning/dashboards"
cat > "${BASE_DIR}/grafana/provisioning/dashboards/dashboards.yml" <<'DBP'
apiVersion: 1

providers:
  - name: claude-otel
    orgId: 1
    folder: Claude Code
    type: file
    disableDeletion: false
    updateIntervalSeconds: 30
    options:
      path: /var/lib/grafana/dashboards
DBP

mkdir -p "${BASE_DIR}/grafana/dashboards"

# ── Claude Code dashboard (Loki-based, mirrors blog post) ─────────────────────
cat > "${BASE_DIR}/grafana/dashboards/claude-code.json" <<'DASH'
{
  "title": "Claude Code Usage",
  "uid": "claude-code-usage",
  "tags": ["claude", "opentelemetry"],
  "timezone": "browser",
  "refresh": "10s",
  "schemaVersion": 38,
  "panels": [
    {
      "id": 1,
      "title": "Total Cost (USD) — all time",
      "type": "stat",
      "gridPos": { "x": 0, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] }, "colorMode": "background" },
      "fieldConfig": { "defaults": { "unit": "currencyUSD", "decimals": 4,
        "thresholds": { "mode": "absolute",
          "steps": [{"color":"green","value":null},{"color":"yellow","value":1},{"color":"red","value":5}]
        }
      }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json cost_usd=\"attributes.cost_usd\" | unwrap cost_usd [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 2,
      "title": "Input Tokens — all time",
      "type": "stat",
      "gridPos": { "x": 6, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json input_tokens=\"attributes.input_tokens\" | unwrap input_tokens [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 3,
      "title": "Output Tokens — all time",
      "type": "stat",
      "gridPos": { "x": 12, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json output_tokens=\"attributes.output_tokens\" | unwrap output_tokens [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 4,
      "title": "Cache Read Tokens — all time",
      "type": "stat",
      "gridPos": { "x": 18, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json cache_read_tokens=\"attributes.cache_read_tokens\" | unwrap cache_read_tokens [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 5,
      "title": "Cost over time (USD)",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 4, "w": 24, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "currencyUSD", "custom": { "fillOpacity": 15 }}},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum by (session_id) (sum_over_time({service_name=\"claude-code\"} | json cost_usd=\"attributes.cost_usd\", session_id=\"attributes.session_id\" | unwrap cost_usd [1m]))" }]
    },
    {
      "id": 6,
      "title": "Token usage over time",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 12, "w": 24, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "short", "custom": { "fillOpacity": 10 }}},
      "targets": [
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "A", "legendFormat": "input",
          "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json input_tokens=\"attributes.input_tokens\" | unwrap input_tokens [1m]))" },
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "B", "legendFormat": "output",
          "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json output_tokens=\"attributes.output_tokens\" | unwrap output_tokens [1m]))" },
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "C", "legendFormat": "cache_read",
          "expr": "sum(sum_over_time({service_name=\"claude-code\"} | json cache_read_tokens=\"attributes.cache_read_tokens\" | unwrap cache_read_tokens [1m]))" }
      ]
    },
    {
      "id": 7,
      "title": "Log stream",
      "type": "logs",
      "gridPos": { "x": 0, "y": 20, "w": 24, "h": 10 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "showTime": true, "sortOrder": "Descending" },
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "{service_name=\"claude-code\"}" }]
    }
  ],
  "time": { "from": "now-1h", "to": "now" }
}
DASH
ok "Claude Code dashboard written"

# ── Dirigent dashboard (OTLP-based, execution metrics + cue lifecycle) ────────
cat > "${BASE_DIR}/grafana/dashboards/dirigent.json" <<'DIRDASH'
{
  "title": "Dirigent Usage",
  "uid": "dirigent-usage",
  "tags": ["dirigent", "opentelemetry"],
  "timezone": "browser",
  "refresh": "10s",
  "schemaVersion": 38,
  "panels": [
    {
      "id": 1,
      "title": "Total Cost (USD)",
      "type": "stat",
      "gridPos": { "x": 0, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] }, "colorMode": "background" },
      "fieldConfig": { "defaults": { "unit": "currencyUSD", "decimals": 4,
        "thresholds": { "mode": "absolute",
          "steps": [{"color":"green","value":null},{"color":"yellow","value":1},{"color":"red","value":5}]
        }
      }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json cost_usd=\"attributes.cost_usd\" | unwrap cost_usd [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 2,
      "title": "Input Tokens",
      "type": "stat",
      "gridPos": { "x": 6, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json input_tokens=\"attributes.input_tokens\" | unwrap input_tokens [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 3,
      "title": "Output Tokens",
      "type": "stat",
      "gridPos": { "x": 12, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum(sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json output_tokens=\"attributes.output_tokens\" | unwrap output_tokens [$__auto]))",
        "queryType": "instant" }]
    },
    {
      "id": 4,
      "title": "Total Executions",
      "type": "stat",
      "gridPos": { "x": 18, "y": 0, "w": 6, "h": 4 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "reduceOptions": { "calcs": ["lastNotNull"] } },
      "fieldConfig": { "defaults": { "unit": "short" }},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "count_over_time({service_name=\"dirigent\"} |= \"execution.completed\" [$__auto])",
        "queryType": "instant" }]
    },
    {
      "id": 5,
      "title": "Cost over time (USD) by project",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 4, "w": 24, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "currencyUSD", "custom": { "fillOpacity": 15 }}},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum by (project) (sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json cost_usd=\"attributes.cost_usd\", project=\"attributes.project\" | unwrap cost_usd [1m]))" }]
    },
    {
      "id": 6,
      "title": "Token usage over time",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 12, "w": 24, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "short", "custom": { "fillOpacity": 10 }}},
      "targets": [
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "A", "legendFormat": "input",
          "expr": "sum(sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json input_tokens=\"attributes.input_tokens\" | unwrap input_tokens [1m]))" },
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "B", "legendFormat": "output",
          "expr": "sum(sum_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json output_tokens=\"attributes.output_tokens\" | unwrap output_tokens [1m]))" }
      ]
    },
    {
      "id": 7,
      "title": "Execution duration (ms)",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 20, "w": 12, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "ms", "custom": { "fillOpacity": 10, "drawStyle": "bars" }}},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "avg_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json duration_ms=\"attributes.duration_ms\" | unwrap duration_ms [5m])" }]
    },
    {
      "id": 8,
      "title": "Executions by provider",
      "type": "timeseries",
      "gridPos": { "x": 12, "y": 20, "w": 12, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "short", "custom": { "fillOpacity": 15, "drawStyle": "bars" }}},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum by (provider) (count_over_time({service_name=\"dirigent\"} |= \"execution.completed\" | json provider=\"attributes.provider\" [5m]))" }]
    },
    {
      "id": 9,
      "title": "Agent runs by kind",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 28, "w": 12, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "short", "custom": { "fillOpacity": 15, "drawStyle": "bars" }}},
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "sum by (agent_kind) (count_over_time({service_name=\"dirigent\"} |= \"agent.completed\" | json agent_kind=\"attributes.agent_kind\" [5m]))" }]
    },
    {
      "id": 10,
      "title": "Failures & rate limits",
      "type": "timeseries",
      "gridPos": { "x": 12, "y": 28, "w": 12, "h": 8 },
      "datasource": { "type": "loki", "uid": "loki" },
      "fieldConfig": { "defaults": { "unit": "short", "custom": { "fillOpacity": 15, "drawStyle": "bars" }}},
      "targets": [
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "A", "legendFormat": "failed",
          "expr": "count_over_time({service_name=\"dirigent\"} |= \"execution.failed\" [5m])" },
        { "datasource": { "type": "loki", "uid": "loki" }, "refId": "B", "legendFormat": "rate_limited",
          "expr": "count_over_time({service_name=\"dirigent\"} |= \"execution.rate_limited\" [5m])" }
      ]
    },
    {
      "id": 11,
      "title": "Log stream",
      "type": "logs",
      "gridPos": { "x": 0, "y": 36, "w": 24, "h": 10 },
      "datasource": { "type": "loki", "uid": "loki" },
      "options": { "showTime": true, "sortOrder": "Descending" },
      "targets": [{ "datasource": { "type": "loki", "uid": "loki" }, "refId": "A",
        "expr": "{service_name=\"dirigent\"}" }]
    }
  ],
  "time": { "from": "now-1h", "to": "now" }
}
DIRDASH
ok "Dirigent dashboard written"

# ── Alloy config (local — no cloud auth) ──────────────────────────────────────
step "Step 3 / 6 — Writing Grafana Alloy config (local targets)"

cat > "${BASE_DIR}/alloy.alloy" <<'ALLOY'
// Grafana Alloy — local LGTM stack
// OTLP listeners forward to Loki, Tempo and Prometheus

// ── OTLP receiver (Claude Code sends here) ───────────────────────────────────
otelcol.receiver.otlp "default" {
  grpc { endpoint = "0.0.0.0:4317" }
  http { endpoint = "0.0.0.0:4318" }

  output {
    metrics = [otelcol.processor.batch.default.input]
    logs    = [otelcol.processor.batch.default.input]
    traces  = [otelcol.processor.batch.default.input]
  }
}

// ── Batch processor ───────────────────────────────────────────────────────────
otelcol.processor.batch "default" {
  output {
    metrics = [otelcol.exporter.prometheus.default.input]
    logs    = [otelcol.exporter.loki.default.input]
    traces  = [otelcol.exporter.otlp.tempo.input]
  }
}

// ── Exporters ─────────────────────────────────────────────────────────────────
otelcol.exporter.loki "default" {
  forward_to = [loki.write.default.receiver]
}

loki.write "default" {
  endpoint {
    url = "http://loki:3100/loki/api/v1/push"
  }
}

otelcol.exporter.prometheus "default" {
  forward_to = [prometheus.remote_write.default.receiver]
}

prometheus.remote_write "default" {
  endpoint {
    url = "http://prometheus:9090/api/v1/write"
  }
}

otelcol.exporter.otlp "tempo" {
  client {
    endpoint = "tempo:4317"
    tls { insecure = true }
  }
}
ALLOY
ok "Alloy config written"

# ═════════════════════════════════════════════════════════════════════════════
# STEP 4 — docker-compose.yml
# ═════════════════════════════════════════════════════════════════════════════
step "Step 4 / 6 — Generating docker-compose.yml"

cat > "${BASE_DIR}/docker-compose.yml" <<COMPOSE
name: claude-otel

services:

  # ── Loki — log storage ──────────────────────────────────────────────────────
  loki:
    image: grafana/loki:latest
    container_name: claude-otel-loki
    restart: unless-stopped
    ports:
      - "3100:3100"
    volumes:
      - ./loki/loki.yml:/etc/loki/local-config.yaml:ro
      - loki-data:/tmp/loki
    command: -config.file=/etc/loki/local-config.yaml

  # ── Tempo — trace storage ───────────────────────────────────────────────────
  tempo:
    image: grafana/tempo:2.7.2
    container_name: claude-otel-tempo
    restart: unless-stopped
    ports:
      - "3200:3200"
      - "4320:4317"    # OTLP gRPC (internal, not exposed to Claude Code)
    volumes:
      - ./tempo/tempo.yml:/etc/tempo.yml:ro
      - tempo-data:/tmp/tempo
    command: -config.file=/etc/tempo.yml

  # ── Prometheus — metrics storage ────────────────────────────────────────────
  prometheus:
    image: prom/prometheus:latest
    container_name: claude-otel-prometheus
    restart: unless-stopped
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus/prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - prometheus-data:/prometheus
    command:
      - --config.file=/etc/prometheus/prometheus.yml
      - --web.enable-remote-write-receiver
      - --enable-feature=exemplar-storage

  # ── Grafana Alloy — OTEL collector ──────────────────────────────────────────
  alloy:
    image: grafana/alloy:latest
    container_name: claude-otel-alloy
    restart: unless-stopped
    ports:
      - "12345:12345"  # Alloy UI
      - "4317:4317"    # OTLP gRPC  ← Claude Code sends here
      - "4318:4318"    # OTLP HTTP  ← Claude Code sends here
    volumes:
      - ./alloy.alloy:/etc/alloy/config.alloy:ro
      - alloy-data:/var/lib/alloy/data
    command:
      - run
      - --server.http.listen-addr=0.0.0.0:12345
      - --storage.path=/var/lib/alloy/data
      - /etc/alloy/config.alloy
    depends_on:
      - loki
      - tempo
      - prometheus

  # ── Grafana — dashboards ─────────────────────────────────────────────────────
  grafana:
    image: grafana/grafana:latest
    container_name: claude-otel-grafana
    restart: unless-stopped
    ports:
      - "3001:3000"
    environment:
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: "Admin"
      GF_AUTH_DISABLE_LOGIN_FORM: "true"
      GF_FEATURE_TOGGLES_ENABLE: "traceqlEditor"
    volumes:
      - ./grafana/provisioning:/etc/grafana/provisioning:ro
      - ./grafana/dashboards:/var/lib/grafana/dashboards:ro
      - grafana-data:/var/lib/grafana
    depends_on:
      - loki
      - tempo
      - prometheus

volumes:
  loki-data:
  tempo-data:
  prometheus-data:
  grafana-data:
  alloy-data:
COMPOSE
ok "docker-compose.yml written"

# ═════════════════════════════════════════════════════════════════════════════
# STEP 5 — Start the stack
# ═════════════════════════════════════════════════════════════════════════════
step "Step 5 / 6 — Starting the local LGTM stack"
info "This will pull ~1 GB of images on first run — grab a coffee."
echo ""

cd "$BASE_DIR"
if [[ "$CONTAINER_CMD" == "docker" ]]; then
  ${=COMPOSE_CMD} pull --quiet
else
  ${=COMPOSE_CMD} pull
fi
${=COMPOSE_CMD} up -d || warn "Some containers may have failed to start — checking individually…"

echo ""
info "Waiting for services to become healthy…"

wait_healthy() {
  local name="$1" url="$2" label="$3"
  local attempts=0
  while ! curl -sf "$url" &>/dev/null; do
    sleep 2
    attempts=$((attempts + 1))
    [[ $attempts -ge 30 ]] && { warn "${label} did not start in time — check: ${COMPOSE_CMD} logs ${name}"; return; }
  done
  ok "${label} is up"
}

wait_healthy "claude-otel-loki"       "http://localhost:3100/ready"   "Loki"
wait_healthy "claude-otel-tempo"      "http://localhost:3200/ready"   "Tempo"
wait_healthy "claude-otel-prometheus" "http://localhost:9090/-/ready" "Prometheus"
wait_healthy "claude-otel-alloy"      "http://localhost:12345/"       "Alloy"
wait_healthy "claude-otel-grafana"    "http://localhost:3001/api/health" "Grafana"

# ═════════════════════════════════════════════════════════════════════════════
# STEP 6 — Environment variables (Claude Code + Dirigent)
# ═════════════════════════════════════════════════════════════════════════════
step "Step 6 / 6 — Writing OTEL environment variables (Claude Code + Dirigent)"

echo ""
echo "  Would you like to also log tool content, tool details, and user prompts?"
warn "  These will be stored in your local Loki — still private, but more verbose."

EXTRA_LOGGING=false
confirm "Enable extended prompt/tool logging?" && EXTRA_LOGGING=true

ENV_FILE="${BASE_DIR}/claude-otel.env"

cat > "$ENV_FILE" <<ENV
# ── Claude Code + Dirigent  →  OpenTelemetry  →  Local LGTM stack ────────────
# Source before running claude or Dirigent:   source ${ENV_FILE}

# ── Claude Code ──────────────────────────────────────────────────────────────

# Undocumented beta tracing
export BETA_TRACING_ENDPOINT="http://localhost:4318"
export CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1
export ENABLE_BETA_TRACING_DETAILED=1

# Core telemetry
export CLAUDE_CODE_ENABLE_TELEMETRY=1
export CLAUDE_CODE_OTEL_FLUSH_TIMEOUT_MS=1000
export OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4317"
export OTEL_EXPORTER_OTLP_PROTOCOL="grpc"

# Near-real-time export intervals
export OTEL_LOGS_EXPORT_INTERVAL=1000
export OTEL_METRIC_EXPORT_INTERVAL=1000

# OTLP exporters
export OTEL_LOGS_EXPORTER="otlp"
export OTEL_METRICS_EXPORTER="otlp"
export OTEL_TRACES_EXPORTER="otlp"

# ── Dirigent ─────────────────────────────────────────────────────────────────
# When set, Dirigent emits OTLP logs (execution metrics, cue lifecycle,
# agent runs, git commits) to the Alloy collector via HTTP.
export DIRIGENT_OTEL_ENDPOINT="http://localhost:4318"
ENV

if [[ "$EXTRA_LOGGING" == "true" ]]; then
  cat >> "$ENV_FILE" <<EXTRA

# Extended logging — stored locally in Loki
export OTEL_LOG_TOOL_CONTENT=1
export OTEL_LOG_TOOL_DETAILS=1
export OTEL_LOG_USER_PROMPTS=1
EXTRA
  ok "Extended logging enabled"
fi

ok "Env file: ${ENV_FILE}"

# ── Persist to ~/.zshrc ───────────────────────────────────────────────────────
echo ""
if confirm "Add 'source ${ENV_FILE}' to ~/.zshrc?"; then
  MARKER="# claude-otel-local"
  if grep -q "$MARKER" "${HOME}/.zshrc" 2>/dev/null; then
    warn "Entry already in ~/.zshrc — skipping."
  else
    { echo ""; echo "${MARKER}"; echo "source \"${ENV_FILE}\""; } >> "${HOME}/.zshrc"
    ok "Added to ~/.zshrc — open a new terminal or run: source ~/.zshrc"
  fi
else
  ok "Skipped. Activate manually: ${CYAN}source ${ENV_FILE}${RESET}"
fi

# ═════════════════════════════════════════════════════════════════════════════
# Summary
# ═════════════════════════════════════════════════════════════════════════════
echo ""
echo "${BOLD}${GREEN}════════════════════════════════════════════════════════════════${RESET}"
echo "${BOLD}${GREEN}  Stack running — everything is local.${RESET}"
echo "${BOLD}${GREEN}════════════════════════════════════════════════════════════════${RESET}"
echo ""
echo "  ${BOLD}Service URLs${RESET}"
echo "    Grafana       →  ${CYAN}http://localhost:3001${RESET}   (no login required)"
echo "    Prometheus    →  ${CYAN}http://localhost:9090${RESET}"
echo "    Alloy UI      →  ${CYAN}http://localhost:12345${RESET}"
echo ""
echo "  ${BOLD}OTEL endpoints (Claude Code + Dirigent → Alloy)${RESET}"
echo "    gRPC          →  localhost:4317  (Claude Code)"
echo "    HTTP          →  localhost:4318  (Dirigent + Claude Code)"
echo ""
echo "  ${BOLD}Stack directory${RESET}  ${BASE_DIR}"
echo ""
echo "  ${BOLD}Compose commands${RESET}  (run from ${BASE_DIR})"
echo "    ${CYAN}${COMPOSE_CMD} ps${RESET}               — status"
echo "    ${CYAN}${COMPOSE_CMD} logs -f alloy${RESET}    — Alloy logs"
echo "    ${CYAN}${COMPOSE_CMD} stop${RESET}             — pause all services"
echo "    ${CYAN}${COMPOSE_CMD} down -v${RESET}          — remove stack + data"
echo ""
echo "  ${BOLD}Run Claude Code with telemetry${RESET}"
echo "    ${CYAN}source ${ENV_FILE} && claude${RESET}"
echo ""
echo "  ${BOLD}Run Dirigent with telemetry${RESET}"
echo "    ${CYAN}source ${ENV_FILE} && Dirigent /path/to/project${RESET}"
echo "    (or set DIRIGENT_OTEL_ENDPOINT in .Dirigent/.env per-project)"
echo ""
echo "  ${BOLD}Grafana dashboards${RESET}"
echo "    Open http://localhost:3001"
echo "    Navigate to Dashboards  →  Claude Code  →  Claude Code Usage"
echo "    Navigate to Dashboards  →  Claude Code  →  Dirigent Usage"
echo ""
echo "  ${BOLD}Explore logs directly (Grafana Explore)${RESET}"
echo '    {service_name="claude-code"}    — Claude Code events'
echo '    {service_name="dirigent"}       — Dirigent events'
echo ""
