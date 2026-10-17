#!/usr/bin/env bash
# Open the Strata diff UI on this repo.
#
# Default: compare the latest commit (HEAD) against the uncommitted working tree.
#
# Usage:
#   scripts/strata.sh                        HEAD vs working tree
#   scripts/strata.sh --from main            main vs working tree
#   scripts/strata.sh --from HEAD~3 --to HEAD
#   scripts/strata.sh --provenance           attribute the diff to Claude Code transcripts
#   scripts/strata.sh --no-cache             skip the incremental cache in .strata/
#   scripts/strata.sh --build                cargo build --release in the Strata checkout first
#   scripts/strata.sh -- --threshold 0.9     everything after -- goes to strata-ui verbatim
#
# Environment overrides:
#   STRATA_DIR   Strata checkout (default: $HOME/prj/strata_diff_tool)
#   STRATA_UI    path to the strata-ui binary (default: $STRATA_DIR/target/release/strata-ui,
#                falling back to strata-ui on PATH)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STRATA_DIR="${STRATA_DIR:-$HOME/prj/strata_diff_tool}"
FROM="HEAD"
TO=""
CACHE=1
BUILD=0
PROVENANCE=""
PASSTHROUGH=()

while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="${2:?--from needs a revision}"; shift 2 ;;
    --to) TO="${2:?--to needs a revision}"; shift 2 ;;
    --provenance)
      # Bare flag auto-discovers transcripts; an optional path may follow.
      if [ $# -ge 2 ] && [ "${2#-}" = "$2" ]; then
        PROVENANCE="$2"; shift 2
      else
        PROVENANCE="auto"; shift
      fi
      ;;
    --no-cache) CACHE=0; shift ;;
    --build) BUILD=1; shift ;;
    -h|--help) sed -n '2,/^[^#]/p' "$0" | sed -n 's/^# \{0,1\}//p'; exit 0 ;;
    --) shift; PASSTHROUGH+=("$@"); break ;;
    *) echo "Unknown option: $1 (use -- to pass args straight to strata-ui)" >&2; exit 2 ;;
  esac
done

if [ "$BUILD" -eq 1 ]; then
  echo "==> Building strata-ui in $STRATA_DIR"
  cargo build --release --manifest-path "$STRATA_DIR/Cargo.toml" --bin strata-ui
fi

STRATA_UI="${STRATA_UI:-$STRATA_DIR/target/release/strata-ui}"
if [ ! -x "$STRATA_UI" ]; then
  if command -v strata-ui >/dev/null 2>&1; then
    STRATA_UI="$(command -v strata-ui)"
  else
    echo "strata-ui not found at $STRATA_UI." >&2
    echo "Set STRATA_DIR/STRATA_UI, or run: scripts/strata.sh --build" >&2
    exit 1
  fi
fi

ARGS=(--git "$FROM" --repo "$REPO_ROOT")
[ -n "$TO" ] && ARGS+=(--to "$TO")
[ "$CACHE" -eq 1 ] && ARGS+=(--cache)
if [ "$PROVENANCE" = "auto" ]; then
  ARGS+=(--provenance)
elif [ -n "$PROVENANCE" ]; then
  ARGS+=(--provenance "$PROVENANCE")
fi
ARGS+=("${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}")

echo "==> $STRATA_UI ${ARGS[*]}"
exec "$STRATA_UI" "${ARGS[@]}"
