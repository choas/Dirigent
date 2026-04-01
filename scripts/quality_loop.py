#!/usr/bin/env python3
"""
quality_loop.py -- Automated code quality improvement agent for Rust projects.

Run with --help for full usage instructions.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path


HELP_EPILOG = """
SETUP (step by step)
--------------------

1. Open a PR on GitHub
   Work on your branch as usual, push it, and open a Pull Request.
   Note the PR number from the GitHub URL (e.g. /pull/42 -> 42).

2. Create a .env file in the repo root (never commit this)
   Add it to .gitignore first:

       echo '.env' >> .gitignore

   Then create the file:

       SONAR_TOKEN=your_sonarqube_or_sonarcloud_token
       SONAR_URL=https://sonarcloud.io
       SONAR_PROJECT_KEY=your_org_your_repo
       GH_REPO=yourname/dirigent
       GH_PR_NUMBER=42

   GH_PR_NUMBER is the only value that changes per PR.
   Everything else stays constant across runs.

3. Authenticate the GitHub CLI (one-time)

       gh auth login

4. Make sure sonar-scanner is on your PATH (one-time)

       brew install sonar-scanner        # macOS
       # or download from sonarqube.org

5. Run the script from the repo root

       python scripts/quality_loop.py

   Or with overrides:

       python scripts/quality_loop.py --pr 43 --max-iterations 5 --dry-run

WHERE TO PUT THE SCRIPT
-----------------------
Option A (recommended): commit it to the repo as scripts/quality_loop.py
  - Lives alongside the code it operates on
  - Other contributors can use it
  - Not part of the code being reviewed (SonarQube/CodeRabbit ignore scripts/)

Option B: keep it outside the repo entirely
  - Run it from anywhere: python ~/tools/quality_loop.py
  - Set REPO_PATH env var or cd into the repo first

WHAT THE LOOP DOES
------------------
  1. cargo test             -- establishes a green baseline (exits if red)
  [ loop begins ]
  2. sonar-scanner          -- pushes analysis to SonarQube
  3. Fetch SonarQube issues -- via REST API
  4. cargo clippy           -- collects diagnostics as JSON
  5. cargo fmt --check      -- detects formatting drift
  6. Fetch CodeRabbit comments -- from the PR via GitHub API
  7. If all signals clear   -- loop exits cleanly
  8. Claude Code fixes all  -- single batched invocation per iteration
  9. cargo test             -- regression guard (exits if red)
 10. git commit + push      -- one commit per iteration
 11. Poll until CI done     -- waits for coderabbitai summary comment
 [ back to step 2 ]

EXIT CODES
----------
  0  All quality signals clean
  1  Tests failed (baseline or regression) -- manual intervention needed
  2  Max iterations reached without full resolution

ENV VARS (all can also be passed as CLI flags)
----------------------------------------------
  SONAR_TOKEN        SonarQube/SonarCloud user token (required)
  SONAR_URL          SonarQube host URL (required)
  SONAR_PROJECT_KEY  Project key in SonarQube (required)
  GH_REPO            GitHub repo as owner/repo (required)
  GH_PR_NUMBER       Pull Request number to operate on (required)
  MAX_ITERATIONS     Hard stop -- default 8
  POLL_INTERVAL      Seconds between CI/CodeRabbit polls -- default 30
  POLL_TIMEOUT       Max seconds to wait per push -- default 600
"""


# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

@dataclass
class Config:
    sonar_token: str
    sonar_url: str
    sonar_project_key: str
    gh_repo: str
    gh_pr_number: str
    max_iterations: int = 8
    poll_interval: int = 30
    poll_timeout: int = 600
    dry_run: bool = False


def load_dotenv(path: Path = Path(".env")) -> None:
    """Load a .env file into os.environ (no external dependency)."""
    if not path.exists():
        return
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, _, value = line.partition("=")
            key = key.strip()
            value = value.strip().strip('"').strip("'")
            if key and key not in os.environ:
                os.environ[key] = value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="quality_loop.py",
        description="Automated code quality improvement agent for Rust projects.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=HELP_EPILOG,
    )
    parser.add_argument("--pr", metavar="NUMBER",
        help="PR number to operate on (overrides GH_PR_NUMBER in .env)")
    parser.add_argument("--repo", metavar="OWNER/REPO",
        help="GitHub repo (overrides GH_REPO in .env)")
    parser.add_argument("--max-iterations", type=int, metavar="N",
        help="Hard stop after N iterations (default: 8)")
    parser.add_argument("--poll-interval", type=int, metavar="SECONDS",
        help="Seconds between CI/CodeRabbit polls (default: 30)")
    parser.add_argument("--poll-timeout", type=int, metavar="SECONDS",
        help="Max wait time per push before continuing (default: 600)")
    parser.add_argument("--dry-run", action="store_true",
        help="Collect and print all signals, but do not invoke Claude Code or push")
    parser.add_argument("--env-file", metavar="PATH", default=".env",
        help="Path to .env file (default: .env in current directory)")
    return parser.parse_args()


def build_config(args: argparse.Namespace) -> Config:
    load_dotenv(Path(args.env_file))

    # CLI flags take priority over env vars
    if args.pr:
        os.environ["GH_PR_NUMBER"] = args.pr
    if args.repo:
        os.environ["GH_REPO"] = args.repo

    for flag, env_key in [
        (args.max_iterations, "MAX_ITERATIONS"),
        (args.poll_interval, "POLL_INTERVAL"),
        (args.poll_timeout, "POLL_TIMEOUT"),
    ]:
        if flag is not None:
            if flag <= 0:
                print(f"[error] --{env_key.lower().replace('_', '-')} must be a positive integer, got {flag}")
                sys.exit(1)
            os.environ[env_key] = str(flag)

    required = ["SONAR_TOKEN", "SONAR_URL", "SONAR_PROJECT_KEY", "GH_REPO", "GH_PR_NUMBER"]
    missing = [k for k in required if not os.environ.get(k)]
    if missing:
        print(f"[error] Missing required configuration: {', '.join(missing)}")
        print(f"        Add them to {args.env_file} or pass as CLI flags.")
        print("        Run with --help for full setup instructions.")
        sys.exit(1)

    return Config(
        sonar_token=os.environ["SONAR_TOKEN"],
        sonar_url=os.environ["SONAR_URL"].rstrip("/"),
        sonar_project_key=os.environ["SONAR_PROJECT_KEY"],
        gh_repo=os.environ["GH_REPO"],
        gh_pr_number=os.environ["GH_PR_NUMBER"],
        max_iterations=int(os.environ.get("MAX_ITERATIONS", 8)),
        poll_interval=int(os.environ.get("POLL_INTERVAL", 30)),
        poll_timeout=int(os.environ.get("POLL_TIMEOUT", 600)),
        dry_run=args.dry_run,
    )


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_SENSITIVE_PREFIXES = ("-Dsonar.token=", "-Dsonar.login=", "--token=")
_MAX_ARG_DISPLAY_LEN = 120


def _redact_cmd(cmd: list[str]) -> list[str]:
    """Return a copy of cmd with sensitive-looking values masked and long args truncated."""
    redacted = []
    for arg in cmd:
        for prefix in _SENSITIVE_PREFIXES:
            if arg.startswith(prefix):
                arg = f"{prefix}***"
                break
        else:
            # Truncate very long arguments (e.g. inlined prompts) to avoid
            # flooding logs with multi-KB strings.
            if len(arg) > _MAX_ARG_DISPLAY_LEN:
                arg = f"{arg[:_MAX_ARG_DISPLAY_LEN]}... ({len(arg)} chars)"
        redacted.append(arg)
    return redacted


def run(cmd: list[str], check=True, capture=False) -> subprocess.CompletedProcess:
    print(f"  $ {' '.join(_redact_cmd(cmd))}")
    try:
        return subprocess.run(
            cmd,
            check=check,
            capture_output=capture,
            text=True,
        )
    except FileNotFoundError:
        print(f"  [error] Command not found: {cmd[0]}")
        return subprocess.CompletedProcess(cmd, returncode=127, stdout="", stderr="")
    except subprocess.CalledProcessError as e:
        print(f"  [error] Command failed (exit {e.returncode}): {' '.join(_redact_cmd(cmd))}")
        return subprocess.CompletedProcess(cmd, returncode=e.returncode,
                                           stdout=e.stdout or "", stderr=e.stderr or "")


def banner(msg: str) -> None:
    width = 60
    print("\n" + "=" * width)
    print(f"  {msg}")
    print("=" * width)


def gh(*args, capture=True) -> str:
    """Run a gh CLI command and return stdout. Returns empty string on failure."""
    result = run(["gh", *args], check=False, capture=capture)
    if result.returncode != 0:
        print(f"  [warn] gh command failed (exit {result.returncode})")
        return ""
    return result.stdout.strip()


# ---------------------------------------------------------------------------
# Cargo
# ---------------------------------------------------------------------------

def cargo_test() -> bool:
    """Returns True if all tests pass."""
    result = run(["cargo", "test"], check=False)
    return result.returncode == 0


def cargo_clippy_issues() -> list[str]:
    """Return clippy diagnostics as a list of message strings."""
    result = run(
        ["cargo", "clippy", "--message-format=json", "--", "-D", "warnings"],
        check=False,
        capture=True,
    )
    issues = []
    for line in result.stdout.splitlines():
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if obj.get("reason") == "compiler-message":
            msg = obj.get("message", {})
            if msg.get("level") in ("error", "warning"):
                rendered = msg.get("rendered", "")
                if rendered:
                    issues.append(rendered.strip())
    return issues


def cargo_fmt_check() -> list[str]:
    """Return list of files with formatting issues."""
    result = run(
        ["cargo", "fmt", "--check", "--message-format=json"],
        check=False,
        capture=True,
    )
    if result.returncode == 0:
        return []
    # fmt --check just exits non-zero, output is human-readable
    return [result.stdout.strip()] if result.stdout.strip() else ["Formatting issues found (run cargo fmt)"]


# ---------------------------------------------------------------------------
# SonarQube
# ---------------------------------------------------------------------------

def sonar_scan(cfg: Config) -> bool:
    """Run sonar-scanner. Returns True if scan succeeded."""
    result = run([
        "sonar-scanner",
        f"-Dsonar.token={cfg.sonar_token}",
        f"-Dsonar.host.url={cfg.sonar_url}",
        f"-Dsonar.projectKey={cfg.sonar_project_key}",
    ])
    if result.returncode != 0:
        print("  [warn] SonarQube scan failed. Continuing without SonarQube issues.")
        return False
    return True


def sonar_issues(cfg: Config) -> list[dict]:
    """Fetch open issues from SonarQube API."""
    import urllib.request
    import urllib.parse
    import base64

    params = urllib.parse.urlencode({
        "componentKeys": cfg.sonar_project_key,
        "resolved": "false",
        "ps": 100,
    })
    url = f"{cfg.sonar_url}/api/issues/search?{params}"
    req = urllib.request.Request(url)
    token_b64 = base64.b64encode(f"{cfg.sonar_token}:".encode()).decode()
    req.add_header("Authorization", f"Basic {token_b64}")

    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            data = json.loads(resp.read())
            return data.get("issues", [])
    except Exception as e:
        print(f"  [warn] SonarQube API error: {e}")
        return []


def format_sonar_issues(issues: list[dict]) -> list[str]:
    formatted = []
    for issue in issues:
        component = issue.get("component", "").split(":")[-1]
        line = issue.get("line", "?")
        severity = issue.get("severity", "?")
        message = issue.get("message", "")
        rule = issue.get("rule", "")
        formatted.append(f"[{severity}] {component}:{line} -- {message} ({rule})")
    return formatted


# ---------------------------------------------------------------------------
# GitHub / CodeRabbit
# ---------------------------------------------------------------------------

def pr_checks_passing(cfg: Config) -> bool:
    """Return True if all PR status checks are green."""
    output = gh(
        "pr", "view", cfg.gh_pr_number,
        "--repo", cfg.gh_repo,
        "--json", "statusCheckRollup",
    )
    try:
        data = json.loads(output)
        checks = data.get("statusCheckRollup", [])
        if not checks:
            return True  # No checks configured -- treat as passing
        return all(
            c.get("conclusion") in ("SUCCESS", "SKIPPED")
            for c in checks
            # A None conclusion means the check is still in progress;
            # skip those so we don't treat pending checks as failures,
            # but also require at least one check to have completed.
            if c.get("conclusion") is not None
        ) and any(c.get("conclusion") is not None for c in checks)
    except json.JSONDecodeError:
        return False


def coderabbit_comments(cfg: Config, since: str = "") -> list[str]:
    """Return unresolved review comments left by coderabbitai[bot].
    If `since` is set (ISO 8601 timestamp), only return comments created after that time."""
    args = [
        "api",
        f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/comments",
        "--paginate",
    ]
    if since:
        args.extend(["-f", f"since={since}"])
    output = gh(*args)
    try:
        comments = json.loads(output)
    except json.JSONDecodeError:
        return []

    unresolved = []
    for c in comments:
        user = c.get("user", {}).get("login", "")
        if "coderabbitai" not in user:
            continue
        # Skip if the comment thread was resolved (no direct API field --
        # heuristic: if body contains actionable suggestion keywords)
        body = c.get("body", "")
        if any(kw in body.lower() for kw in ("suggestion", "consider", "should", "nitpick", "issue")):
            path = c.get("path", "")
            line = c.get("line") or c.get("original_line") or "?"
            unresolved.append(f"{path}:{line} -- {body[:300]}")

    return unresolved


def coderabbit_summary_posted(cfg: Config) -> bool:
    """Return True if CodeRabbit has posted its summary review for this push."""
    output = gh(
        "api",
        f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/reviews",
    )
    try:
        reviews = json.loads(output)
    except json.JSONDecodeError:
        return False

    return any(
        "coderabbitai" in r.get("user", {}).get("login", "")
        for r in reviews
    )


def wait_for_coderabbit(cfg: Config) -> None:
    """Block until CI passes and CodeRabbit has reviewed the latest push."""
    banner(f"Waiting for CI + CodeRabbit (timeout {cfg.poll_timeout}s)")
    deadline = time.time() + cfg.poll_timeout
    ci_passed = False
    cr_done = False

    while time.time() < deadline:
        if not ci_passed:
            ci_passed = pr_checks_passing(cfg)
            print(f"  CI checks passing: {ci_passed}")

        if ci_passed and not cr_done:
            cr_done = coderabbit_summary_posted(cfg)
            print(f"  CodeRabbit summary posted: {cr_done}")

        if ci_passed and cr_done:
            print("  All systems go.")
            return

        print(f"  Sleeping {cfg.poll_interval}s ...")
        time.sleep(cfg.poll_interval)

    print("[warn] Timed out waiting for CI/CodeRabbit. Continuing anyway.")


# ---------------------------------------------------------------------------
# Claude Code
# ---------------------------------------------------------------------------

def _process_claude_stream(proc) -> None:
    """Parse and display Claude stream-json events from stdout."""
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        _print_stream_event(event)


def claude_fix(prompt: str, iteration: int = 0) -> bool:
    """Invoke Claude Code with stream-json output to show live progress.
    Returns True if Claude exited successfully."""
    # Log prompt to /tmp for debugging (owner-only permissions since the
    # prompt may embed issue details or tokens from surrounding context).
    ts = time.strftime("%Y%m%d_%H%M%S")
    prompt_file = f"/tmp/quality_loop_prompt_{ts}_iter{iteration}.txt"
    pf = Path(prompt_file)
    pf.write_text(prompt)
    pf.chmod(0o600)
    print(f"  Prompt saved to {prompt_file}")
    print(f"  $ claude -p '<prompt ({len(prompt)} chars)>' --output-format stream-json --verbose --dangerously-skip-permissions")
    start = time.time()
    try:
        proc = subprocess.Popen(
            ["claude", "-p", prompt, "--output-format", "stream-json", "--verbose", "--dangerously-skip-permissions"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except FileNotFoundError:
        print("  [error] Command not found: claude")
        return False

    _process_claude_stream(proc)
    stderr_output = proc.stderr.read()
    proc.wait()
    elapsed = time.time() - start
    print(f"  Claude Code finished in {elapsed:.0f}s (exit {proc.returncode})")
    if proc.returncode != 0 and stderr_output:
        print(f"  [stderr] {stderr_output.strip()}")
    return proc.returncode == 0


def _extract_tool_detail(inp: dict) -> str:
    """Build a short detail string from a tool_use input dict."""
    if "command" in inp:
        first_line = inp["command"].split("\n", 1)[0]
        return f" $ {first_line}"
    if "file_path" in inp:
        return f" {inp['file_path']}"
    if "pattern" in inp:
        return f' "{inp["pattern"]}"'
    return ""


def _print_result_summary(event: dict) -> None:
    """Print cost/duration summary from a Claude result event."""
    cost = event.get("cost_usd")
    duration = event.get("duration_ms")
    if cost is None and duration is None:
        return
    parts = []
    if duration is not None:
        parts.append(f"{duration / 1000:.0f}s")
    if cost is not None:
        parts.append(f"${cost:.4f}")
    print(f"  [{', '.join(parts)}]")


def _print_stream_event(event: dict) -> None:
    """Print a human-readable summary of a Claude stream-json event."""
    etype = event.get("type", "")
    if etype == "assistant":
        for block in event.get("message", {}).get("content", []):
            btype = block.get("type", "")
            if btype == "tool_use":
                name = block.get("name", "?")
                detail = _extract_tool_detail(block.get("input", {}))
                print(f"  \u2192 {name}{detail}")
    elif etype == "result":
        _print_result_summary(event)


def build_fix_prompt(
    sonar: list[str],
    clippy: list[str],
    fmt: list[str],
    coderabbit: list[str],
) -> str:
    parts = [
        "You are a code quality agent for a Rust project.",
        "Fix ALL of the following issues. Do not change any functionality --",
        "only improve code quality. After each fix, ensure the code still compiles.",
        "",
    ]
    if sonar:
        parts.append("== SonarQube issues ==")
        parts.extend(sonar)
        parts.append("")
    if clippy:
        parts.append("== Clippy diagnostics ==")
        parts.extend(clippy)
        parts.append("")
    if fmt:
        parts.append("== Formatting issues ==")
        parts.extend(fmt)
        parts.append("")
    if coderabbit:
        parts.append("== CodeRabbit review comments ==")
        parts.extend(coderabbit)
        parts.append("")
    parts.append(
        "Fix all issues above. Run `cargo fmt` and `cargo clippy` after editing "
        "to verify no new issues were introduced. Do not modify tests."
    )
    return "\n".join(parts)


# ---------------------------------------------------------------------------
# Git
# ---------------------------------------------------------------------------

def current_sha() -> str:
    result = run(["git", "rev-parse", "HEAD"], check=False, capture=True)
    return result.stdout.strip()


def _get_dirty_files() -> set[str]:
    """Return the set of tracked file paths with uncommitted changes."""
    result = run(["git", "diff", "--name-only", "HEAD"], check=False, capture=True)
    return set(result.stdout.strip().splitlines()) if result.stdout.strip() else set()


def git_commit_push(iteration: int, files_to_stage: list[str] | None = None) -> bool:
    """Commit and push changes. Returns True if push succeeded.
    If `files_to_stage` is given, only those files are staged (avoids
    accidentally committing pre-existing local edits or secrets)."""
    if files_to_stage:
        result = run(["git", "add", "--"] + files_to_stage, check=False)
    else:
        result = run(["git", "add", "-u"], check=False)
    if result.returncode != 0:
        print("  [warn] git add failed.")
        return False
    result = run(
        ["git", "diff", "--cached", "--quiet"],
        check=False,
    )
    if result.returncode == 0:
        print("  Nothing to commit -- skipping push.")
        return False
    result = run(["git", "commit", "-m", f"chore: quality loop iteration {iteration}"], check=False)
    if result.returncode != 0:
        print("  [warn] git commit failed.")
        return False
    result = run(["git", "push"], check=False)
    if result.returncode != 0:
        print("  [warn] git push failed. Changes are committed locally.")
        return False
    return True


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def collect_signals(cfg: Config, cr_since: str = "") -> tuple[list[str], list[str], list[str], list[str]]:
    """Collect all quality signals and return (sonar, clippy, fmt, coderabbit).
    `cr_since` filters CodeRabbit comments to only those created after this ISO 8601 timestamp."""
    print("\n[1/4] Running SonarQube scan ...")
    scan_ok = sonar_scan(cfg)
    s_issues = format_sonar_issues(sonar_issues(cfg)) if scan_ok else []
    print(f"  SonarQube issues: {len(s_issues)}")

    print("\n[2/4] Running cargo clippy ...")
    c_issues = cargo_clippy_issues()
    print(f"  Clippy issues: {len(c_issues)}")

    print("\n[3/4] Running cargo fmt --check ...")
    f_issues = cargo_fmt_check()
    print(f"  Fmt issues: {len(f_issues)}")

    print("\n[4/4] Fetching CodeRabbit comments ...")
    cr_issues = coderabbit_comments(cfg, since=cr_since)
    print(f"  CodeRabbit comments: {len(cr_issues)}")

    return s_issues, c_issues, f_issues, cr_issues


def _apply_fixes(prompt: str, iteration: int) -> None:
    """Invoke Claude Code and verify tests still pass. Exits on test failure."""
    print("  Invoking Claude Code ...")
    success = claude_fix(prompt, iteration)
    if not success:
        print("[warn] Claude Code exited with an error. Continuing anyway.")

    print("\n  Running cargo test (regression guard) ...")
    if not cargo_test():
        print("[error] Tests broke after Claude's changes. Stopping.")
        print("  Check the diff, fix manually, and re-run.")
        sys.exit(1)
    print("  Tests still green.")


def _commit_and_wait(cfg: Config, iteration: int, files_to_stage: list[str] | None = None) -> None:
    """Commit, push, and wait for CI/CodeRabbit if the push succeeded."""
    sha_before = current_sha()
    pushed = git_commit_push(iteration, files_to_stage=files_to_stage)
    sha_after = current_sha()

    if sha_before == sha_after:
        print("  No changes committed — Claude may not have produced a fix.")
        return

    if pushed:
        wait_for_coderabbit(cfg)
    else:
        print("  Skipping CI/CodeRabbit wait (push did not succeed).")


def _run_iteration(cfg: Config, iteration: int, cr_since: str) -> str:
    """Run one quality loop iteration. Returns updated cr_since timestamp,
    or empty string to signal that all issues are resolved."""
    from datetime import datetime, timezone

    s_issues, c_issues, f_issues, cr_issues = collect_signals(cfg, cr_since=cr_since)
    total = len(s_issues) + len(c_issues) + len(f_issues) + len(cr_issues)

    if total == 0:
        return ""

    print(f"\n  Total issues: {total}.")
    prompt = build_fix_prompt(s_issues, c_issues, f_issues, cr_issues)

    if cfg.dry_run:
        print("\n[dry-run] Would invoke Claude Code with this prompt:\n")
        print(prompt)
        print("\n[dry-run] Stopping after first iteration.")
        sys.exit(0)

    # Snapshot dirty files before Claude runs so we can scope staging
    # to only files that are *newly* changed, avoiding the risk of
    # committing pre-existing local edits or mis-ignored secrets.
    dirty_before = _get_dirty_files()

    _apply_fixes(prompt, iteration)

    dirty_after = _get_dirty_files()
    new_files = sorted(dirty_after - dirty_before)

    new_cr_since = datetime.now(timezone.utc).isoformat()
    _commit_and_wait(cfg, iteration, files_to_stage=new_files or None)

    return new_cr_since


def main() -> None:
    args = parse_args()
    cfg = build_config(args)

    if cfg.dry_run:
        print("[dry-run] No Claude Code invocations or git pushes will happen.")

    banner("Baseline: cargo test")
    if not cargo_test():
        print("[error] Tests failed before the loop even started. Fix them first.")
        sys.exit(1)
    print("  Baseline tests green.")

    cr_since = ""

    for iteration in range(1, cfg.max_iterations + 1):
        banner(f"Iteration {iteration}/{cfg.max_iterations}")
        cr_since = _run_iteration(cfg, iteration, cr_since)
        if not cr_since:
            banner("All signals clean -- loop complete!")
            sys.exit(0)

    print(f"\n[warn] Reached max iterations ({cfg.max_iterations}). Stopping.")
    print("  Some issues may remain. Check the PR manually.")
    sys.exit(2)


if __name__ == "__main__":
    main()
