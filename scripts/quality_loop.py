#!/usr/bin/env python3
"""
quality_loop.py -- Automated quality loop for the Dirigent repository.

What it does, per round:

  1. Baseline        cargo test must be green (Claude repairs and commits if not)
  2. SonarQube       starts the local server if needed (scripts/sonar.sh up + setup),
                     runs the analysis (scripts/sonar.sh analyze), fetches findings
  3. Fix findings    Claude Code fixes them and commits per file group:
                       fix(<scope>): <description> (SonarQube <rule-id>)
  4. Bot comments    fetches unprocessed CodeRabbit and Codex comments from the PR;
                     Claude implements each suggestion and commits it:
                       fix(<scope>): <description> (CodeRabbit)
  5. Push            pushes all commits of the round
  6. Poll            polls the PR for NEW bot comments -- processed comments are
                     remembered in /tmp and ignored. New comments start the next
                     round; a quiet round after the bots reviewed ends the loop.

If the script started the SonarQube server it stops it on exit
(--keep-sonar leaves it running). Duplication findings are not part of the
loop -- use `scripts/sonar.sh report` for those.

Usage:
    python3 scripts/quality_loop.py                 # auto-detects the PR
    python3 scripts/quality_loop.py --pr 42
    python3 scripts/quality_loop.py --dry-run       # print findings + prompts only
    python3 scripts/quality_loop.py --skip-sonar    # bot comments only

Requirements: gh (authenticated), sonar-scanner, podman/docker, claude.

EXIT CODES
  0  clean -- no findings, no new comments
  1  tests failing and Claude could not repair them, or a Claude Code
     invocation failed (unprocessed items stay unseen for the next run)
  2  max rounds reached with issues remaining
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from base64 import b64encode
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

REPO_ROOT = Path(__file__).resolve().parent.parent
SONAR_SH = "scripts/sonar.sh"
SONAR_URL = "http://localhost:9006"
SONAR_PROJECT_KEY = "dirigent"

# login substring -> label used in commit messages
BOT_LABELS = {"coderabbit": "CodeRabbit", "codex": "Codex"}

TEST_CMD = ["cargo", "test", "--workspace"]

SCOPE_MAPPING = """\
Scope mapping for commit messages (deepest meaningful module wins):
- src/app/...            -> app
- src/<module>.rs        -> <module>   (e.g. src/git.rs -> git)
- claude_pty/...         -> claude_pty
- dirigent-mcp-server/.. -> mcp-server
- root config files      -> config"""

VERIFY_INSTRUCTIONS = """\
Verify every change before committing: `cargo check` for Rust,
`npx tsc --noEmit` inside dirigent-mcp-server for TypeScript.
Run `cargo fmt` before each commit that touches Rust files."""


@dataclass
class Config:
    gh_repo: str
    gh_pr_number: str
    max_rounds: int = 5
    poll_interval: int = 60
    poll_timeout: int = 1800
    initial_wait: int = 60
    skip_sonar: bool = False
    keep_sonar: bool = False
    dry_run: bool = False
    sonar_token: str = ""


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def banner(msg: str) -> None:
    print("\n" + "=" * 60)
    print(f"  {msg}")
    print("=" * 60)


def run(cmd: list[str], check=False, capture=False, timeout: Optional[int] = None) -> subprocess.CompletedProcess:
    print(f"  $ {' '.join(cmd)}")
    try:
        return subprocess.run(cmd, check=check, capture_output=capture, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        print(f"  [error] Command timed out after {timeout}s")
        return subprocess.CompletedProcess(cmd, returncode=124, stdout="", stderr="")
    except FileNotFoundError:
        print(f"  [error] Command not found: {cmd[0]}")
        return subprocess.CompletedProcess(cmd, returncode=127, stdout="", stderr="")


def gh(cfg: Config, *args) -> str:
    """Run a gh CLI command. Returns stdout, or empty string on failure."""
    result = run(["gh", *args, "-R", cfg.gh_repo] if args[0] != "api" else ["gh", *args],
                 capture=True)
    if result.returncode != 0:
        print(f"  [warn] gh failed (exit {result.returncode}): {(result.stderr or '').strip()[:200]}")
        return ""
    return result.stdout.strip()


def read_env_token() -> str:
    """Read SONAR_TOKEN from .env -- .env wins over the (possibly stale) environment."""
    env_file = REPO_ROOT / ".env"
    if env_file.exists():
        for line in env_file.read_text().splitlines():
            if line.startswith("SONAR_TOKEN="):
                return line.partition("=")[2].strip()
    return os.environ.get("SONAR_TOKEN", "")


def head_sha() -> str:
    return run(["git", "rev-parse", "HEAD"], capture=True).stdout.strip()


def working_tree_dirty() -> bool:
    out = run(["git", "status", "--porcelain"], capture=True).stdout.strip()
    return bool(out)


# ---------------------------------------------------------------------------
# SonarQube (local server via scripts/sonar.sh)
# ---------------------------------------------------------------------------

def _sonar_api(cfg: Config, path: str, params: dict) -> Optional[dict]:
    url = f"{SONAR_URL}{path}?{urllib.parse.urlencode(params)}"
    req = urllib.request.Request(url)
    req.add_header("Authorization", "Basic " + b64encode(f"{cfg.sonar_token}:".encode()).decode())
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read())
    except Exception as e:
        print(f"  [warn] SonarQube API error ({path}): {e}")
        return None


def sonar_server_up() -> bool:
    try:
        with urllib.request.urlopen(f"{SONAR_URL}/api/system/status", timeout=5) as resp:
            return json.loads(resp.read()).get("status") == "UP"
    except Exception:
        return False


def ensure_sonar(cfg: Config) -> bool:
    """Make sure the SonarQube server is running and a token exists in .env.
    Returns True if this script started the server (caller stops it on exit)."""
    started = False
    if sonar_server_up():
        print("  SonarQube already running -- will leave it running afterwards.")
    else:
        print("  SonarQube not running -- starting it.")
        if run(["bash", SONAR_SH, "up"]).returncode != 0:
            print("  [error] Could not start SonarQube.")
            sys.exit(1)
        started = True
    if run(["bash", SONAR_SH, "setup"]).returncode != 0:
        print("  [error] SonarQube setup failed.")
        sys.exit(1)
    cfg.sonar_token = read_env_token()
    if not cfg.sonar_token:
        print("  [error] No SONAR_TOKEN in .env after setup.")
        sys.exit(1)
    return started


def sonar_analyze(cfg: Config) -> bool:
    """Run the scanner and wait until the server has processed the report."""
    if run(["bash", SONAR_SH, "analyze"], timeout=600).returncode != 0:
        print("  [warn] Analysis failed -- continuing without SonarQube findings.")
        return False
    deadline = time.time() + 180
    while time.time() < deadline:
        data = _sonar_api(cfg, "/api/ce/component", {"component": SONAR_PROJECT_KEY})
        if data is not None and not data.get("queue"):
            current = data.get("current") or {}
            if current.get("status") == "FAILED":
                print("  [warn] Server-side report processing failed.")
                return False
            if current.get("status") == "SUCCESS":
                return True
        time.sleep(3)
    print("  [warn] Timed out waiting for report processing -- using previous results.")
    return True


def sonar_findings(cfg: Config) -> list[dict]:
    """Fetch open issues and TO_REVIEW hotspots. Each finding:
    {key, file, line, severity, rule, message}."""
    findings = []
    page = 1
    while True:
        data = _sonar_api(cfg, "/api/issues/search", {
            "componentKeys": SONAR_PROJECT_KEY,
            "statuses": "OPEN,CONFIRMED,REOPENED",
            "ps": 500, "p": page,
        })
        if not data:
            break
        for issue in data.get("issues", []):
            findings.append({
                "key": issue.get("key", ""),
                "file": issue.get("component", "").split(":")[-1],
                "line": issue.get("line", "?"),
                "severity": issue.get("severity", "?"),
                "rule": issue.get("rule", ""),
                "message": issue.get("message", ""),
            })
        if page * 500 >= data.get("paging", {}).get("total", 0):
            break
        page += 1

    data = _sonar_api(cfg, "/api/hotspots/search", {
        "projectKey": SONAR_PROJECT_KEY, "status": "TO_REVIEW", "ps": 500,
    })
    for h in (data or {}).get("hotspots", []):
        findings.append({
            "key": h.get("key", ""),
            "file": h.get("component", "").split(":")[-1],
            "line": h.get("line", "?"),
            "severity": f"HOTSPOT/{h.get('vulnerabilityProbability', '?')}",
            "rule": h.get("securityCategory", "hotspot"),
            "message": h.get("message", ""),
        })
    return findings


# ---------------------------------------------------------------------------
# Seen-state (/tmp) -- "ignore the old comments"
# ---------------------------------------------------------------------------

def _state_path(cfg: Config, kind: str) -> Path:
    return Path(f"/tmp/quality_loop_{cfg.gh_repo.replace('/', '_')}_{cfg.gh_pr_number}_{kind}.txt")


def _load_seen(cfg: Config, kind: str) -> set[str]:
    p = _state_path(cfg, kind)
    return set(p.read_text().split()) if p.exists() else set()


def _save_seen(cfg: Config, kind: str, seen: set[str]) -> None:
    _state_path(cfg, kind).write_text("\n".join(sorted(seen)) + "\n")


def filter_unseen(cfg: Config, kind: str, items: list[dict], id_of) -> list[dict]:
    """Return items not seen before. Items are only persisted as seen via
    mark_processed after they were actually handled -- recording them here
    would make a failed run silently skip them on the next attempt."""
    seen = _load_seen(cfg, kind)
    return [i for i in items if id_of(i) not in seen]


def mark_processed(cfg: Config, kind: str, items: list[dict], id_of) -> None:
    """Persist items as seen. Call only after successful processing."""
    if cfg.dry_run or not items:
        return
    seen = _load_seen(cfg, kind)
    seen.update(id_of(i) for i in items)
    _save_seen(cfg, kind, seen)


# ---------------------------------------------------------------------------
# PR bot comments (CodeRabbit, Codex)
# ---------------------------------------------------------------------------

def _bot_label(login: str) -> Optional[str]:
    login = login.lower()
    for pattern, label in BOT_LABELS.items():
        if pattern in login:
            return label
    return None


def fetch_bot_comments(cfg: Config) -> list[dict]:
    """Line comments and review bodies from CodeRabbit/Codex.
    Each: {id, label, location, body}."""
    comments = []

    output = gh(cfg, "api", f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/comments", "--paginate")
    for c in json.loads(output) if output else []:
        label = _bot_label(c.get("user", {}).get("login", ""))
        body = (c.get("body") or "").strip()
        if not label or not body:
            continue
        line = c.get("line") or c.get("original_line") or "?"
        comments.append({
            "id": f"comment-{c.get('id')}",
            "label": label,
            "location": f"{c.get('path', '?')}:{line}",
            "body": body[:2000],
        })

    output = gh(cfg, "api", f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/reviews", "--paginate")
    for r in json.loads(output) if output else []:
        label = _bot_label(r.get("user", {}).get("login", ""))
        body = (r.get("body") or "").strip()
        if not label or not body:
            continue
        # Boilerplate bodies carry no content -- the real findings are line comments
        if body.startswith("Actionable comments posted:") or "About Codex in GitHub" in body:
            continue
        comments.append({
            "id": f"review-{r.get('id')}",
            "label": label,
            "location": "review body",
            "body": body[:2000],
        })

    return comments


def bots_reviewed_since(cfg: Config, since_iso: str) -> bool:
    """True if any bot submitted a review (or line comment) after the given time."""
    output = gh(cfg, "api", f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/reviews", "--paginate")
    for r in json.loads(output) if output else []:
        if _bot_label(r.get("user", {}).get("login", "")) and r.get("submitted_at", "") > since_iso:
            return True
    output = gh(cfg, "api", f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/comments", "--paginate")
    for c in json.loads(output) if output else []:
        if _bot_label(c.get("user", {}).get("login", "")) and c.get("created_at", "") > since_iso:
            return True
    return False


# ---------------------------------------------------------------------------
# Claude Code
# ---------------------------------------------------------------------------

def claude_fix(prompt: str, tag: str) -> bool:
    """Invoke Claude Code, streaming progress. Returns True on exit 0."""
    with tempfile.NamedTemporaryFile(
        mode="w", prefix=f"quality_loop_{tag}_", suffix=".txt", delete=False
    ) as fd:
        fd.write(prompt)
        print(f"  Prompt saved to {fd.name}")
    print(f"  $ claude -p '<prompt ({len(prompt)} chars)>' --output-format stream-json ...")
    start = time.time()
    try:
        proc = subprocess.Popen(
            ["claude", "-p", prompt, "--output-format", "stream-json",
             "--verbose", "--dangerously-skip-permissions"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
    except FileNotFoundError:
        print("  [error] Command not found: claude")
        return False

    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "assistant":
            for block in event.get("message", {}).get("content", []):
                if block.get("type") == "tool_use":
                    inp = block.get("input", {})
                    detail = inp.get("command", "").split("\n", 1)[0] or inp.get("file_path", "")
                    print(f"  -> {block.get('name', '?')} {detail[:100]}")

    stderr_output = proc.stderr.read()
    proc.wait()
    print(f"  Claude Code finished in {time.time() - start:.0f}s (exit {proc.returncode})")
    if proc.returncode != 0 and stderr_output:
        print(f"  [stderr] {stderr_output.strip()[:500]}")
    return proc.returncode == 0


def build_sonar_prompt(findings: list[dict]) -> str:
    by_file: dict[str, list[dict]] = {}
    for f in findings:
        by_file.setdefault(f["file"], []).append(f)

    parts = [
        "You are an expert code quality engineer working in the Dirigent repository",
        "(Rust workspace + a TypeScript MCP server). Fix every SonarQube finding below.",
        "",
        "Workflow -- one commit per file group (the `File:` blocks below):",
        "1. Read the file and understand the surrounding code.",
        "2. Fix ALL findings listed for that file. Never suppress instead of fixing:",
        "   no `#[allow(...)]`, no `// NOSONAR`, no lint-disable comments.",
        "3. " + VERIFY_INSTRUCTIONS.replace("\n", "\n   "),
        "4. Commit ONLY the files you touched, message format:",
        "   fix(<scope>): <short description> (SonarQube <rule-id>[, <rule-id>...])",
        "",
        SCOPE_MAPPING,
        "",
        "Principles: smallest safe change; preserve behavior; don't refactor",
        "surrounding code; don't modify tests except when a finding is inside one.",
        "If a finding cannot be fixed without changing behavior, skip it and say",
        "why in your final summary.",
        "NEVER run `git push`.",
        "",
        "== SonarQube findings ==",
    ]
    for file, group in sorted(by_file.items()):
        parts.append(f"\nFile: {file}")
        for f in group:
            parts.append(f"- [{f['severity']}] line {f['line']}: {f['message']} ({f['rule']})")
    return "\n".join(parts)


def build_comments_prompt(cfg: Config, comments: list[dict]) -> str:
    parts = [
        f"You are addressing automated review feedback on PR #{cfg.gh_pr_number}",
        f"({cfg.gh_repo}) in the Dirigent repository (Rust workspace + a TypeScript",
        "MCP server). The comments below come from the CodeRabbit and Codex bots.",
        "",
        "For each comment:",
        "1. Judge it first -- if it is outdated, wrong, or would change behavior",
        "   in an unwanted way, skip it and say why in your final summary.",
        "2. Otherwise implement the suggestion.",
        "3. " + VERIFY_INSTRUCTIONS.replace("\n", "\n   "),
        "4. Commit each implemented suggestion SEPARATELY, message format:",
        "   fix(<scope>): <short description> (CodeRabbit)   or   (Codex)",
        "",
        SCOPE_MAPPING,
        "",
        "NEVER run `git push`.",
        "",
        "== Review comments ==",
    ]
    for i, c in enumerate(comments, 1):
        parts.append(f"\n--- Comment {i}/{len(comments)} [{c['label']}] {c['location']} ---")
        parts.append(c["body"])
    return "\n".join(parts)


def build_repair_prompt(test_output: str) -> str:
    if len(test_output) > 8000:
        test_output = "... (truncated) ...\n" + test_output[-8000:]
    return "\n".join([
        "You are working in the Dirigent Rust repository. The build or tests are",
        "failing after recent quality fixes. Repair them with the smallest possible",
        "change -- do not change functionality beyond what the errors require.",
        VERIFY_INSTRUCTIONS,
        "Commit the repair with message: fix: repair build/tests after quality fixes",
        "NEVER run `git push`.",
        "",
        "== Build/test errors ==",
        test_output,
    ])


# ---------------------------------------------------------------------------
# Tests / regression guard
# ---------------------------------------------------------------------------

def run_tests() -> tuple[bool, str]:
    result = run(TEST_CMD, capture=True, timeout=1800)
    output = ((result.stdout or "") + "\n" + (result.stderr or "")).strip()
    if result.returncode != 0:
        print(f"  Tests FAILED (exit {result.returncode}). Last lines:")
        for line in output.splitlines()[-25:]:
            print(f"    {line}")
    else:
        print("  Tests passed.")
    return result.returncode == 0, output


def regression_guard(cfg: Config) -> None:
    """Run tests; if red, let Claude repair (and commit) up to 3 times."""
    for attempt in range(4):
        print("\n  Running tests (regression guard) ...")
        passed, output = run_tests()
        if passed:
            return
        if attempt == 3:
            print("[error] Tests still failing after 3 repair attempts. Fix manually.")
            sys.exit(1)
        print(f"  [warn] Asking Claude Code to repair (attempt {attempt + 1}/3) ...")
        claude_fix(build_repair_prompt(output), f"repair{attempt + 1}")


# ---------------------------------------------------------------------------
# Git
# ---------------------------------------------------------------------------

def commit_leftovers() -> None:
    """Commit anything Claude edited but forgot to commit (fmt drift etc.)."""
    if not working_tree_dirty():
        return
    print("  Working tree has uncommitted changes -- committing leftovers.")
    run(["git", "add", "-A"])
    result = run(["git", "commit", "-m", "style: format and leftover fixes from quality loop"])
    if result.returncode != 0:
        print("  [warn] Leftover commit failed -- changes remain uncommitted.")


def push(cfg: Config) -> bool:
    result = run(["git", "push"])
    if result.returncode != 0:
        print("  [warn] git push failed -- commits remain local.")
        return False
    return True


# ---------------------------------------------------------------------------
# Polling
# ---------------------------------------------------------------------------

def poll_for_new_comments(cfg: Config, pushed_at: str) -> list[dict]:
    """Poll the PR after a push. Returns new bot comments (start the next round),
    or [] once the bots reviewed and a quiet round confirms nothing new."""
    banner(f"Polling for new bot comments (initial wait {cfg.initial_wait}s, "
           f"timeout {cfg.poll_timeout}s)")
    time.sleep(cfg.initial_wait)

    deadline = time.time() + cfg.poll_timeout
    quiet_needed = 2
    quiet = 0
    while time.time() < deadline:
        new = filter_unseen(cfg, "comments", fetch_bot_comments(cfg), lambda c: c["id"])
        if new:
            print(f"  {len(new)} new bot comment(s) found.")
            return new
        if bots_reviewed_since(cfg, pushed_at):
            quiet += 1
            print(f"  Bots reviewed this push, nothing new (quiet {quiet}/{quiet_needed}).")
            if quiet >= quiet_needed:
                return []
        else:
            print("  No bot activity for this push yet.")
        print(f"  Sleeping {cfg.poll_interval}s ...")
        time.sleep(cfg.poll_interval)

    print("  [warn] Poll timeout reached -- treating as no new comments.")
    return []


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="quality_loop.py",
        description="SonarQube + CodeRabbit/Codex quality loop for Dirigent.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("Usage:")[1] if "Usage:" in (__doc__ or "") else None,
    )
    parser.add_argument("--pr", metavar="N", help="PR number (default: auto-detect via gh)")
    parser.add_argument("--repo", metavar="OWNER/REPO", help="GitHub repo (default: auto-detect)")
    parser.add_argument("--max-rounds", type=int, default=5, metavar="N",
                        help="max fix/push/poll rounds (default: 5)")
    parser.add_argument("--poll-interval", type=int, default=60, metavar="SECONDS",
                        help="seconds between comment polls (default: 60)")
    parser.add_argument("--poll-timeout", type=int, default=1800, metavar="SECONDS",
                        help="max seconds to poll per push (default: 1800)")
    parser.add_argument("--skip-sonar", action="store_true",
                        help="skip SonarQube entirely (bot comments only)")
    parser.add_argument("--keep-sonar", action="store_true",
                        help="leave the SonarQube server running even if this script started it")
    parser.add_argument("--dry-run", action="store_true",
                        help="print findings and prompts; no Claude, no commits, no push")
    return parser.parse_args()


def build_config(args: argparse.Namespace) -> Config:
    repo = args.repo
    if not repo:
        result = run(["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"],
                     capture=True, timeout=15)
        repo = result.stdout.strip()
    if not repo:
        print("[error] Could not detect GitHub repo. Pass --repo owner/repo.")
        sys.exit(1)

    pr = args.pr
    if not pr:
        result = run(["gh", "pr", "view", "--json", "number", "--jq", ".number"],
                     capture=True, timeout=15)
        pr = result.stdout.strip()
        if pr:
            print(f"[info] Auto-detected PR #{pr}")
    if not pr or not pr.isdigit():
        print("[error] No PR found for the current branch. Open one or pass --pr.")
        sys.exit(1)

    return Config(
        gh_repo=repo, gh_pr_number=pr,
        max_rounds=args.max_rounds,
        poll_interval=args.poll_interval,
        poll_timeout=args.poll_timeout,
        skip_sonar=args.skip_sonar,
        keep_sonar=args.keep_sonar,
        dry_run=args.dry_run,
    )


def run_round(cfg: Config, round_no: int, pending_comments: list[dict]) -> tuple[bool, list[dict]]:
    """One round: sonar fixes + comment fixes + push + poll.
    Returns (work_was_done, comments_for_next_round)."""
    sha_at_start = head_sha()

    # --- SonarQube findings ---
    if not cfg.skip_sonar:
        banner(f"Round {round_no}: SonarQube analysis")
        if sonar_analyze(cfg):
            findings = filter_unseen(cfg, "sonar", sonar_findings(cfg), lambda f: f["key"])
            print(f"  New findings: {len(findings)}")
            if findings:
                prompt = build_sonar_prompt(findings)
                if cfg.dry_run:
                    print("\n[dry-run] SonarQube prompt:\n")
                    print(prompt)
                else:
                    banner(f"Round {round_no}: fixing {len(findings)} SonarQube finding(s)")
                    if not claude_fix(prompt, f"round{round_no}_sonar"):
                        print("[error] Claude Code failed -- findings stay unprocessed"
                              " for the next run.")
                        sys.exit(1)
                    regression_guard(cfg)
                    mark_processed(cfg, "sonar", findings, lambda f: f["key"])

    # --- Bot comments ---
    if pending_comments:
        prompt = build_comments_prompt(cfg, pending_comments)
        if cfg.dry_run:
            print("\n[dry-run] Bot comments prompt:\n")
            print(prompt)
        else:
            banner(f"Round {round_no}: implementing {len(pending_comments)} bot suggestion(s)")
            if not claude_fix(prompt, f"round{round_no}_comments"):
                print("[error] Claude Code failed -- comments stay unprocessed"
                      " for the next run.")
                sys.exit(1)
            regression_guard(cfg)
            mark_processed(cfg, "comments", pending_comments, lambda c: c["id"])

    if cfg.dry_run:
        print("\n[dry-run] Stopping after the first round.")
        sys.exit(0)

    commit_leftovers()

    # --- Push + poll ---
    if head_sha() == sha_at_start:
        print("\n  No commits this round -- nothing to push.")
        return False, []

    banner(f"Round {round_no}: pushing")
    pushed_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if not push(cfg):
        return True, []
    return True, poll_for_new_comments(cfg, pushed_at)


def main() -> None:
    args = parse_args()
    os.chdir(REPO_ROOT)
    cfg = build_config(args)
    print(f"\n  Repo: {cfg.gh_repo}   PR: #{cfg.gh_pr_number}")
    if cfg.dry_run:
        print("  [dry-run] No Claude invocations, commits, or pushes will happen.")

    if working_tree_dirty() and not cfg.dry_run:
        print("[error] Working tree is dirty. Commit or stash your changes first --")
        print("        the loop needs clean state to attribute commits correctly.")
        sys.exit(1)

    # --- Baseline ---
    if cfg.dry_run:
        print("\n  [dry-run] Skipping baseline tests.")
    else:
        banner("Baseline: cargo test")
        regression_guard(cfg)

    # --- SonarQube server ---
    sonar_started = False
    if not cfg.skip_sonar:
        banner("SonarQube server")
        sonar_started = ensure_sonar(cfg)

    try:
        # Initial comment sweep: everything unprocessed counts, old processed
        # comments are remembered in /tmp and ignored.
        print("\n  Fetching unprocessed bot comments ...")
        pending = filter_unseen(cfg, "comments", fetch_bot_comments(cfg), lambda c: c["id"])
        print(f"  Unprocessed bot comments: {len(pending)}")

        for round_no in range(1, cfg.max_rounds + 1):
            worked, pending = run_round(cfg, round_no, pending)
            if pending:
                continue  # new bot comments -> next round
            banner("Quality loop complete -- "
                   + ("pushed, bots quiet." if worked else "nothing left to do."))
            sys.exit(0)

        print(f"\n[warn] Reached max rounds ({cfg.max_rounds}) with comments still pending.")
        sys.exit(2)
    finally:
        if sonar_started and not cfg.keep_sonar:
            banner("Stopping SonarQube (this script started it)")
            run(["bash", SONAR_SH, "down"])
        elif sonar_started:
            print("\n  Leaving SonarQube running (--keep-sonar).")


if __name__ == "__main__":
    main()
