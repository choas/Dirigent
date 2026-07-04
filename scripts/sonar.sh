#!/usr/bin/env bash
# SonarQube toolchain for Dirigent — one script, subcommand per step.
#
# Usage:
#   scripts/sonar.sh up               start SonarQube + Postgres containers
#   scripts/sonar.sh down             stop and remove the containers
#   scripts/sonar.sh setup            wait for ready, set admin password,
#                                     create project, mint token into .env
#   scripts/sonar.sh analyze          run sonar-scanner (extra args passed through)
#   scripts/sonar.sh report           generate sonar-report-*.md + sonar-fix-*.md
#   scripts/sonar.sh run [--keep]     full cycle: up → setup → analyze → report → down
#                                     (--keep skips the shutdown)
#
# Environment overrides:
#   CONTAINER_ENGINE       container CLI (default: podman if available, else docker)
#   SONAR_HOST_URL         server URL (default: http://localhost:9006)
#   SONAR_ADMIN_PASSWORD   admin password (default: Sonar-Admin-2026!)
#   SONAR_WAIT_ATTEMPTS    readiness poll attempts, 3s apart (default: 60)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

COMPOSE_FILE="${REPO_ROOT}/infra/docker-compose.sonarqube.yml"
COMPOSE_PROJECT="sonarqube-dirigent"
SONAR_URL="${SONAR_HOST_URL:-http://localhost:9006}"
PROJECT_KEY="dirigent"
TOKEN_NAME="dirigent-token"
# Community Build enforces a 12+ character password policy
ADMIN_PASS="${SONAR_ADMIN_PASSWORD:-Sonar-Admin-2026!}"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  GREEN=$'\033[1;32m'
  YELLOW=$'\033[1;33m'
  RESET=$'\033[0m'
else
  GREEN=""; YELLOW=""; RESET=""
fi

log() { printf '\n%s==> %s%s\n' "$GREEN" "$*" "$RESET"; }

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

engine() {
  if [ -n "${CONTAINER_ENGINE:-}" ]; then
    echo "${CONTAINER_ENGINE}"
  elif command -v podman >/dev/null 2>&1; then
    echo "podman"
  else
    echo "docker"
  fi
}

# .env wins over an inherited SONAR_TOKEN — shell profiles may export a
# stale token for a different SonarQube instance
load_token() {
  if [ -f .env ]; then
    ENV_TOKEN=$(grep "^SONAR_TOKEN=" .env | cut -d'=' -f2- || true)
    [ -n "${ENV_TOKEN}" ] && SONAR_TOKEN="${ENV_TOKEN}"
  fi
  if [ -z "${SONAR_TOKEN:-}" ]; then
    echo "ERROR: SONAR_TOKEN not set. Run '$0 setup' first."
    exit 1
  fi
}

cmd_up() {
  # SonarQube's embedded Elasticsearch needs vm.max_map_count >= 524288 on
  # Linux hosts; otherwise the container crash-loops without a clear error.
  if [ "$(uname -s)" = "Linux" ]; then
    MAX_MAP_COUNT=$(sysctl -n vm.max_map_count 2>/dev/null || echo 0)
    if [ "${MAX_MAP_COUNT}" -lt 524288 ]; then
      echo "${YELLOW}WARNING: vm.max_map_count is ${MAX_MAP_COUNT}, but SonarQube's embedded"
      echo "Elasticsearch needs at least 524288. Fix it with:"
      echo "  sudo sysctl -w vm.max_map_count=524288${RESET}"
    fi
  fi
  exec "$(engine)" compose -p "${COMPOSE_PROJECT}" -f "${COMPOSE_FILE}" up -d
}

cmd_down() {
  exec "$(engine)" compose -p "${COMPOSE_PROJECT}" -f "${COMPOSE_FILE}" down
}

cmd_setup() {
  echo "Waiting for SonarQube at ${SONAR_URL} ..."
  MAX_ATTEMPTS="${SONAR_WAIT_ATTEMPTS:-60}"  # ~3 minutes at 3s interval
  attempt=0
  until curl -sf "${SONAR_URL}/api/system/status" | grep -q '"status":"UP"'; do
    attempt=$((attempt + 1))
    if [ "${attempt}" -ge "${MAX_ATTEMPTS}" ]; then
      echo "ERROR: SonarQube did not become ready at ${SONAR_URL} after ${MAX_ATTEMPTS} attempts."
      echo "Is the container running? Start it with:"
      echo "  $0 up"
      echo "Then check logs with:"
      echo "  $(engine) logs dirigent_sonarqube"
      exit 1
    fi
    sleep 3
  done
  echo "SonarQube is up."

  if curl -sf -u "admin:admin" "${SONAR_URL}/api/authentication/validate" | grep -q '"valid":true'; then
    echo "Changing default admin password ..."
    curl -sf -u "admin:admin" -X POST \
      "${SONAR_URL}/api/users/change_password" \
      -d "login=admin&previousPassword=admin&password=${ADMIN_PASS}" >/dev/null
    echo "Admin password changed."
  elif curl -sf -u "admin:${ADMIN_PASS}" "${SONAR_URL}/api/authentication/validate" | grep -q '"valid":true'; then
    echo "Admin password already configured."
  else
    echo "ERROR: Cannot authenticate with admin:admin or admin:${ADMIN_PASS}"
    echo "If you changed the password manually, set SONAR_ADMIN_PASSWORD."
    exit 1
  fi

  AUTH="-u admin:${ADMIN_PASS}"

  if curl -sf ${AUTH} "${SONAR_URL}/api/projects/search?projects=${PROJECT_KEY}" | grep -q "\"key\":\"${PROJECT_KEY}\""; then
    echo "Project '${PROJECT_KEY}' already exists."
  else
    echo "Creating project '${PROJECT_KEY}' ..."
    curl -sf ${AUTH} -X POST \
      "${SONAR_URL}/api/projects/create" \
      -d "name=${PROJECT_KEY}&project=${PROJECT_KEY}" >/dev/null
    echo "Project created."
  fi

  echo "Revoking any existing '${TOKEN_NAME}' token ..."
  curl -sf ${AUTH} -X POST \
    "${SONAR_URL}/api/user_tokens/revoke" \
    -d "name=${TOKEN_NAME}" >/dev/null 2>&1 || true

  echo "Generating new token ..."
  TOKEN=$(curl -sf ${AUTH} -X POST \
    "${SONAR_URL}/api/user_tokens/generate" \
    -d "name=${TOKEN_NAME}" \
    | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

  if [ -z "${TOKEN}" ]; then
    echo "ERROR: Failed to generate token."
    exit 1
  fi

  ENV_FILE=".env"
  echo ""
  if [ -f "${ENV_FILE}" ]; then
    if grep -q "^SONAR_TOKEN=" "${ENV_FILE}"; then
      sed -i.bak "s|^SONAR_TOKEN=.*|SONAR_TOKEN=${TOKEN}|" "${ENV_FILE}"
      rm -f "${ENV_FILE}.bak"
      echo "${GREEN}==> SONAR_TOKEN updated in ${ENV_FILE}${RESET}"
    else
      echo "SONAR_TOKEN=${TOKEN}" >> "${ENV_FILE}"
      echo "${GREEN}==> SONAR_TOKEN added to ${ENV_FILE}${RESET}"
    fi
  else
    echo "SONAR_TOKEN=${TOKEN}" > "${ENV_FILE}"
    echo "${GREEN}==> Created ${ENV_FILE} with SONAR_TOKEN${RESET}"
  fi

  echo ""
  echo "${GREEN}Done!${RESET} Run analysis with: $0 analyze"
}

cmd_analyze() {
  load_token

  if ! command -v sonar-scanner >/dev/null 2>&1; then
    echo "ERROR: sonar-scanner not found. Install it with: brew install sonar-scanner"
    exit 1
  fi

  # The scanner's embedded git blame fails on worktrees (.git is a file, not a dir)
  SCM_FLAG=()
  if [ -f .git ]; then
    SCM_FLAG=(-Dsonar.scm.disabled=true)
  fi

  exec env -u SONAR_TOKEN -u SONAR_URL -u SONAR_PROJECT_KEY \
    sonar-scanner -Dsonar.token="${SONAR_TOKEN}" "${SCM_FLAG[@]+"${SCM_FLAG[@]}"}" "$@"
}

cmd_report() {
  load_token

  TIMESTAMP=$(date +%Y%m%d-%H%M%S)
  REPORT_FILE="sonar-report-${TIMESTAMP}.md"
  FIX_FILE="sonar-fix-${TIMESTAMP}.md"

  api() {
    curl -sf -u "admin:${ADMIN_PASS}" "${SONAR_URL}$1" \
      || curl -sf -u "${SONAR_TOKEN}:" "${SONAR_URL}$1"
  }

  echo "Fetching SonarQube data from ${SONAR_URL} ..."

  QUALITY_GATE=$(api "/api/qualitygates/project_status?projectKey=${PROJECT_KEY}")
  METRICS=$(api "/api/measures/component?component=${PROJECT_KEY}&metricKeys=bugs,vulnerabilities,code_smells,coverage,duplicated_lines_density,ncloc,sqale_index,sqale_rating,reliability_rating,security_rating,security_hotspots")

  PAGE=1
  PAGE_SIZE=500
  ALL_ISSUES="[]"
  while true; do
    RESPONSE=$(api "/api/issues/search?componentKeys=${PROJECT_KEY}&statuses=OPEN,CONFIRMED,REOPENED&ps=${PAGE_SIZE}&p=${PAGE}&additionalFields=comments")
    PAGE_ISSUES=$(echo "${RESPONSE}" | jq '.issues // []')
    TOTAL=$(echo "${RESPONSE}" | jq '.paging.total // 0')
    ALL_ISSUES=$(echo "${ALL_ISSUES}" "${PAGE_ISSUES}" | jq -s '.[0] + .[1]')
    FETCHED=$(echo "${ALL_ISSUES}" | jq 'length')
    if [ "${FETCHED}" -ge "${TOTAL}" ]; then
      break
    fi
    PAGE=$((PAGE + 1))
  done

  HOTSPOTS_RESPONSE=$(api "/api/hotspots/search?projectKey=${PROJECT_KEY}&status=TO_REVIEW&ps=500" 2>/dev/null || echo '{"hotspots":[]}')
  ALL_HOTSPOTS=$(echo "${HOTSPOTS_RESPONSE}" | jq '.hotspots // []')

  DUPLICATIONS_RESPONSE=$(api "/api/measures/component_tree?component=${PROJECT_KEY}&metricKeys=duplicated_lines_density,duplicated_blocks,duplicated_lines&qualifiers=FIL&metricSort=duplicated_lines_density&s=metric&asc=false&ps=50" 2>/dev/null || echo '{"components":[]}')
  ALL_DUPLICATIONS=$(echo "${DUPLICATIONS_RESPONSE}" | jq '[.components // [] | .[] | select(.measures[]? | select(.metric=="duplicated_lines_density") | .value != "0.0")]')

  ISSUE_COUNT=$(echo "${ALL_ISSUES}" | jq 'length')
  HOTSPOT_COUNT=$(echo "${ALL_HOTSPOTS}" | jq 'length')
  DUPLICATION_FILE_COUNT=$(echo "${ALL_DUPLICATIONS}" | jq 'length')
  echo "Found ${ISSUE_COUNT} issues, ${HOTSPOT_COUNT} security hotspots, and ${DUPLICATION_FILE_COUNT} files with duplications."

  if [ "${ISSUE_COUNT}" -eq 0 ] && [ "${HOTSPOT_COUNT}" -eq 0 ] && [ "${DUPLICATION_FILE_COUNT}" -eq 0 ]; then
    echo ""
    echo "  *   *   *       *   *   *       *   *   *  "
    echo "    *       *       *       *       *       * "
    echo "  *   *   *   *   *   *   *   *   *   *   *  "
    echo ""
    echo "       No issues -- congratulations!          "
    echo ""
    echo "  *   *   *   *   *   *   *   *   *   *   *  "
    echo "    *       *       *       *       *       * "
    echo "  *   *   *       *   *   *       *   *   *  "
    echo ""
    echo "  Quality Gate: $(echo "${QUALITY_GATE}" | jq -r '.projectStatus.status // "UNKNOWN"')"
    echo "  Your code is clean. No report generated."
    exit 0
  fi

  GATE_STATUS=$(echo "${QUALITY_GATE}" | jq -r '.projectStatus.status // "UNKNOWN"')

  rating_label() {
    case "$1" in
      1|1.0) echo "A" ;;
      2|2.0) echo "B" ;;
      3|3.0) echo "C" ;;
      4|4.0) echo "D" ;;
      5|5.0) echo "E" ;;
      *) echo "$1" ;;
    esac
  }

  get_metric() {
    echo "${METRICS}" | jq -r ".component.measures[]? | select(.metric==\"$1\") | .value // \"N/A\""
  }

  BUGS=$(get_metric "bugs")
  VULNS=$(get_metric "vulnerabilities")
  SMELLS=$(get_metric "code_smells")
  COVERAGE=$(get_metric "coverage")
  DUPLICATION=$(get_metric "duplicated_lines_density")
  NCLOC=$(get_metric "ncloc")
  TECH_DEBT=$(get_metric "sqale_index")
  MAINTAINABILITY=$(rating_label "$(get_metric 'sqale_rating')")
  RELIABILITY=$(rating_label "$(get_metric 'reliability_rating')")
  SECURITY=$(rating_label "$(get_metric 'security_rating')")
  HOTSPOTS_METRIC=$(get_metric "security_hotspots")

  # --- Generate sonar-report ---

  {
  cat <<'HEADER'
# SonarQube Analysis Report

> **Purpose**: This report captures the complete SonarQube analysis for the project.
> Use the companion `sonar-fix-*.md` file to have Claude Code fix these issues automatically.

HEADER

  cat <<EOF
**Project**: \`${PROJECT_KEY}\`
**Server**: ${SONAR_URL}
**Generated**: $(date '+%Y-%m-%d %H:%M:%S')
**Quality Gate**: ${GATE_STATUS}

## Project Metrics

| Metric | Value |
|---|---|
| Lines of Code | ${NCLOC} |
| Bugs | ${BUGS} |
| Vulnerabilities | ${VULNS} |
| Code Smells | ${SMELLS} |
| Security Hotspots | ${HOTSPOTS_METRIC:-${HOTSPOT_COUNT}} |
| Coverage | ${COVERAGE}% |
| Duplications | ${DUPLICATION}% |
| Technical Debt | ${TECH_DEBT} min |
| Maintainability Rating | ${MAINTAINABILITY} |
| Reliability Rating | ${RELIABILITY} |
| Security Rating | ${SECURITY} |

EOF

  echo "## Quality Gate Conditions"
  echo ""
  echo "${QUALITY_GATE}" | jq -r '
    .projectStatus.conditions[]? |
    "| \(.metricKey) | \(.status) | actual: \(.actualValue) | threshold: \(.errorThreshold // "-") |"
  ' | {
    echo "| Metric | Status | Actual | Threshold |"
    echo "|---|---|---|---|"
    cat
  }
  echo ""

  for SEVERITY in BLOCKER CRITICAL MAJOR MINOR INFO; do
    SEVERITY_ISSUES=$(echo "${ALL_ISSUES}" | jq "[.[] | select(.severity==\"${SEVERITY}\")]")
    COUNT=$(echo "${SEVERITY_ISSUES}" | jq 'length')
    if [ "${COUNT}" -eq 0 ]; then
      continue
    fi

    echo "## ${SEVERITY} Issues (${COUNT})"
    echo ""

    echo "${SEVERITY_ISSUES}" | jq -r '
      group_by(.type) | .[] |
      "### Type: \(.[0].type) (\(length))\n" +
      ([.[] |
        "#### \(.message | gsub("\n"; " "))\n" +
        "- **File**: `\(.component | sub(".*:"; ""))`" +
        (if .line then " (line \(.line))" else "" end) + "\n" +
        "- **Rule**: `\(.rule)`\n" +
        "- **Effort**: \(.effort // "N/A")\n" +
        "- **Tags**: \((.tags // []) | join(", "))\n"
      ] | join("\n"))
    '
  done

  if [ "${HOTSPOT_COUNT}" -gt 0 ]; then
    echo "## Security Hotspots (${HOTSPOT_COUNT})"
    echo ""
    echo "${ALL_HOTSPOTS}" | jq -r '
      .[] |
      "### \(.message)\n" +
      "- **File**: `\(.component | sub(".*:"; ""))`" +
      (if .line then " (line \(.line))" else "" end) + "\n" +
      "- **Rule**: `\(.securityCategory)` / `\(.vulnerabilityProbability)`\n" +
      "- **Status**: \(.status)\n"
    '
  fi

  if [ "${DUPLICATION_FILE_COUNT}" -gt 0 ]; then
    echo "## Duplications (${DUPLICATION_FILE_COUNT} files)"
    echo ""
    echo "| File | Duplication % | Duplicated Lines | Duplicated Blocks |"
    echo "|---|---|---|---|"
    echo "${ALL_DUPLICATIONS}" | jq -r '
      .[] |
      (.key | sub(".*:"; "")) as $file |
      ((.measures // []) | map({(.metric): .value}) | add // {}) as $m |
      "| `\($file)` | \($m.duplicated_lines_density // "N/A")% | \($m.duplicated_lines // "N/A") | \($m.duplicated_blocks // "N/A") |"
    '
    echo ""
  fi

  echo "## All Issues by File"
  echo ""
  echo "${ALL_ISSUES}" | jq -r '
    [.[] | .component_short = (.component | sub(".*:"; ""))] |
    group_by(.component_short) | .[] |
    "### `\(.[0].component_short)`\n" +
    ([.[] |
      "- [\(.severity)] \(.message | gsub("\n"; " "))" +
      (if .line then " (line \(.line))" else "" end) +
      " — `\(.rule)`"
    ] | join("\n")) + "\n"
  '

  } > "${REPORT_FILE}"

  echo "Created ${REPORT_FILE}"

  # --- Generate sonar-fix ---

  {
  cat <<PROMPT_HEADER
# SonarQube Fix Instructions

> **How to use**: Open Claude Code and pass this file as a prompt.
>
> \`\`\`
> claude "take a look at the @${FIX_FILE} document and fix all changes (see also @${REPORT_FILE} as reference)"
> \`\`\`

## Task

You are an expert code quality engineer. Fix every SonarQube issue listed below.
Follow these rules strictly:

### Workflow — one commit per file group

Process each **file group** (the \`#### File:\` blocks below) as a unit.
For each file group:

1. **Read** the file and understand the surrounding code context
2. **Fix** all issues listed for that file — don't just suppress warnings
3. **Verify** the fix compiles (\`cargo check\` for Rust, \`npx tsc --noEmit\` in \`dirigent-mcp-server\` for TypeScript)
4. **Commit** with a message in this format:
   \`\`\`
   fix(<scope>): <short description> (SonarQube <rule-id>[, <rule-id>...])

   <one-line explanation of what the rules require and why the old code violated them>
   \`\`\`
5. **Update this document**: mark each fixed issue's checkbox as \`[x]\` and append \` — **FIXED**: <what you changed>\`

If a file group contains issues with **different rules** that require incompatible changes (e.g. a complexity refactor vs. a deprecation fix), split them into separate commits — but still update this document after each one.

### Progress tracking

- This document serves as a living checklist. Update it after every commit.
- If you resume from a partially-completed run, skip any items already marked \`[x]\`.
- At the end, every checkbox should be checked.

### Principles

- **One commit per file group**: All same-rule issues in one file = one commit. Different rules in one file may be separate commits if the changes don't overlap.
- **Smallest safe change**: Don't refactor surrounding code — fix exactly what SonarQube flagged
- **Preserve behavior**: Fixes must not change runtime behavior unless the issue IS a bug
- **No suppression comments**: Don't add \`// NOSONAR\` or \`#[allow(...)]\` — actually fix the problem
- **Order by severity**: Fix BLOCKER and CRITICAL issues first, then MAJOR, MINOR, INFO
- **Explain trade-offs**: If a fix has a non-obvious trade-off, mention it in the commit body

### Scope mapping for commit messages

Use the deepest meaningful module or directory name as scope:
- \`src/app/...\` → \`app\`
- \`src/<module>.rs\` → \`<module>\` (e.g. \`src/git.rs\` → \`git\`)
- \`claude_pty/...\` → \`claude_pty\`
- \`dirigent-mcp-server/...\` → \`mcp-server\`
- Root config files → \`config\`

PROMPT_HEADER

  echo "---"
  echo ""
  echo "## Issues to Fix"
  echo ""

  for SEVERITY in BLOCKER CRITICAL MAJOR MINOR INFO; do
    SEVERITY_ISSUES=$(echo "${ALL_ISSUES}" | jq "[.[] | select(.severity==\"${SEVERITY}\")]")
    COUNT=$(echo "${SEVERITY_ISSUES}" | jq 'length')
    if [ "${COUNT}" -eq 0 ]; then
      continue
    fi

    echo "### ${SEVERITY} (${COUNT} issues)"
    echo ""

    echo "${SEVERITY_ISSUES}" | jq -r '
      [.[] | .component_short = (.component | sub(".*:"; ""))] |
      group_by(.component_short) | .[] |
      "#### File: `\(.[0].component_short)`\n" +
      (group_by(.rule) | [.[] |
        ([.[] |
          "- [ ] **\(.rule)**: \(.message | gsub("\n"; " "))" +
          (if .line then " *(line \(.line))*" else "" end) +
          (if .effort then " — effort: \(.effort)" else "" end)
        ] | join("\n"))
      ] | join("\n")) + "\n"
    '
  done

  if [ "${HOTSPOT_COUNT}" -gt 0 ]; then
    echo "### Security Hotspots (${HOTSPOT_COUNT})"
    echo ""
    echo "${ALL_HOTSPOTS}" | jq -r '
      group_by(.component) | .[] |
      "#### File: `\(.[0].component | sub(".*:"; ""))`\n" +
      (.[] |
        "- [ ] **\(.securityCategory)** (\(.vulnerabilityProbability) risk): \(.message)" +
        (if .line then " *(line \(.line))*" else "" end)
      ) + "\n"
    '
  fi

  if [ "${DUPLICATION_FILE_COUNT}" -gt 0 ]; then
    echo "### Duplications (${DUPLICATION_FILE_COUNT} files)"
    echo ""
    echo "Extract shared logic into reusable functions/modules to reduce duplication."
    echo ""
    echo "${ALL_DUPLICATIONS}" | jq -r '
      .[] |
      (.key | sub(".*:"; "")) as $file |
      ((.measures // []) | map({(.metric): .value}) | add // {}) as $m |
      "#### File: `\($file)`\n" +
      "- [ ] **duplicated_lines_density**: \($m.duplicated_lines_density // "N/A")% duplication (\($m.duplicated_lines // "?") lines, \($m.duplicated_blocks // "?") blocks) — extract duplicated code into shared utilities\n"
    '
  fi

  cat <<'PROMPT_FOOTER'
---

## Final Steps

After all issues are fixed:

1. Run `cargo fmt` and commit any formatting changes
2. Run `cargo clippy --workspace` and verify no new warnings
3. Run `cargo build` to verify compilation
4. Run `cargo test --workspace` to verify tests pass
5. If fmt or build required additional changes, commit them:
   ```
   style: format code after SonarQube fixes
   ```
6. Verify every checkbox in this document is marked `[x]`
7. Add a `## Summary` section at the bottom of this document with:
   - Total issues fixed
   - Number of commits created
   - Any issues that could not be fixed and why
PROMPT_FOOTER

  } > "${FIX_FILE}"

  echo "Created ${FIX_FILE}"
  echo ""
  echo "Next steps:"
  echo "  1. Review: cat ${REPORT_FILE}"
  echo "  2. Fix all issues:"
  echo "     claude \"take a look at the @${FIX_FILE} document and fix all changes (see also @${REPORT_FILE} as reference)\""
}

# Wait until the server-side Compute Engine has processed the last scanner
# upload — the scanner returns before the analysis is visible via the API,
# so reporting right away could read the previous analysis.
wait_for_ce_task() {
  TASK_FILE=".scannerwork/report-task.txt"
  CE_TASK_ID=""
  if [ -f "${TASK_FILE}" ]; then
    CE_TASK_ID=$(grep '^ceTaskId=' "${TASK_FILE}" | cut -d'=' -f2- || true)
  fi
  if [ -z "${CE_TASK_ID}" ]; then
    echo "WARNING: no ceTaskId found in ${TASK_FILE}; falling back to a fixed 15s wait."
    sleep 15
    return
  fi

  echo "Waiting for SonarQube to process analysis (task ${CE_TASK_ID}) ..."
  MAX_ATTEMPTS="${SONAR_WAIT_ATTEMPTS:-60}"  # ~3 minutes at 3s interval
  attempt=0
  while true; do
    STATUS=$(curl -sf -u "admin:${ADMIN_PASS}" "${SONAR_URL}/api/ce/task?id=${CE_TASK_ID}" \
      | grep -o '"status":"[^"]*"' | head -1 | cut -d'"' -f4 || true)
    case "${STATUS}" in
      SUCCESS)
        echo "Analysis processed."
        return 0
        ;;
      FAILED|CANCELED)
        echo "ERROR: server-side analysis processing ended with status ${STATUS}."
        echo "Check the background tasks at ${SONAR_URL}/admin/background_tasks"
        exit 1
        ;;
    esac
    attempt=$((attempt + 1))
    if [ "${attempt}" -ge "${MAX_ATTEMPTS}" ]; then
      echo "ERROR: analysis was not processed after ${MAX_ATTEMPTS} attempts."
      exit 1
    fi
    sleep 3
  done
}

cmd_run() {
  KEEP_RUNNING=false
  for arg in "$@"; do
    case "$arg" in
      --keep) KEEP_RUNNING=true ;;
    esac
  done

  cleanup() {
    local exit_code=$?
    if [ "$KEEP_RUNNING" = false ]; then
      log "Stopping SonarQube"
      "$0" down 2>/dev/null || true
    else
      log "Keeping SonarQube running (--keep)"
    fi
    exit "$exit_code"
  }
  trap cleanup EXIT

  log "Starting SonarQube"
  "$0" up

  log "Setting up SonarQube (waiting for ready, creating project, minting token)"
  cmd_setup

  log "Running analysis"
  "$0" analyze

  log "Waiting for server-side processing"
  wait_for_ce_task

  log "Generating report and LLM fix instructions"
  cmd_report
}

COMMAND="${1:-}"
[ $# -gt 0 ] && shift

case "${COMMAND}" in
  up)      cmd_up "$@" ;;
  down)    cmd_down "$@" ;;
  setup)   cmd_setup "$@" ;;
  analyze) cmd_analyze "$@" ;;
  report)  cmd_report "$@" ;;
  run)     cmd_run "$@" ;;
  -h|--help|help|"") usage; [ -n "${COMMAND}" ] || exit 1 ;;
  *)
    echo "ERROR: unknown command '${COMMAND}'"
    echo ""
    usage
    exit 1
    ;;
esac
