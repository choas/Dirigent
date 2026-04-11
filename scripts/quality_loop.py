#!/usr/bin/env python3
"""
quality_loop.py -- Automated code quality improvement agent for any language.

Run with --help for full usage instructions.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


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
       GH_REPO=yourname/yourrepo
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
       python scripts/quality_loop.py --lang python
       python scripts/quality_loop.py --lang javascript

   Or with custom tool commands:

       python scripts/quality_loop.py \\
           --test-cmd "pytest -x" \\
           --lint-cmd "ruff check --output-format json ." \\
           --fmt-check-cmd "ruff format --check ." \\
           --fmt-fix-cmd "ruff format ."

   Or with a config file:

       python scripts/quality_loop.py --config .quality_loop.json

CONFIG FILE (.quality_loop.json)
--------------------------------
Place a JSON file in the repo root to define all tool commands:

    {
        "lang": "python",
        "test_cmd": "pytest -x",
        "lint_cmd": "ruff check --output-format json .",
        "fmt_check_cmd": "ruff format --check .",
        "fmt_fix_cmd": "ruff format ."
    }

Any field can be null to skip that signal. CLI flags override config file values.

SUPPORTED LANGUAGES (built-in presets)
--------------------------------------
  rust        cargo test / cargo clippy / cargo fmt
  python      pytest / ruff check (or flake8, pylint) / ruff format (or black)
  javascript  npm test / eslint / prettier --check
  typescript  npm test / eslint / prettier --check
  go          go test ./... / go vet + staticcheck / gofmt -l
  java        mvn test / mvn checkstyle:check / (none)
  ruby        bundle exec rspec / rubocop --format json / rubocop -a
  swift       swift test / swiftlint / swiftformat --lint

Use --lang to select a preset, then override individual commands with
--test-cmd, --lint-cmd, --fmt-check-cmd, --fmt-fix-cmd as needed.

WHAT THE LOOP DOES
------------------
  1. test command          -- establishes a green baseline (exits if red)
  [ loop begins ]
  2. sonar-scanner          -- pushes analysis to SonarQube
  3. Fetch SonarQube issues -- via REST API
  4. Fetch security hotspots -- via /api/hotspots/search
  5. Fetch duplication metrics -- via /api/measures/component
  6. lint command           -- collects diagnostics
  7. fmt check command      -- detects formatting drift
  8. Fetch CodeRabbit comments -- from the PR via GitHub API
  9. If all signals clear   -- loop exits cleanly
 10. Claude Code fixes all  -- single batched invocation per iteration
 11. fmt fix command        -- auto-fix formatting
 12. test command           -- regression guard (exits if red)
 13. git commit + push      -- one commit per iteration
 14. Poll until CI done     -- waits for coderabbitai summary comment
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
  POLL_TIMEOUT       Max seconds to wait per push -- default 900
"""


# ---------------------------------------------------------------------------
# Language presets
# ---------------------------------------------------------------------------

@dataclass
class LangPreset:
    """Commands for a specific language. None means skip that signal."""
    name: str
    test_cmd: Optional[str]
    lint_cmd: Optional[str]
    fmt_check_cmd: Optional[str]
    fmt_fix_cmd: Optional[str]
    # The verify commands Claude should run after editing (used in the prompt)
    verify_instructions: str


PRESETS: dict[str, LangPreset] = {
    "rust": LangPreset(
        name="Rust",
        test_cmd="cargo test",
        lint_cmd="cargo clippy --message-format=json -- -D warnings",
        fmt_check_cmd="cargo fmt --check",
        fmt_fix_cmd="cargo fmt",
        verify_instructions="Run `cargo fmt` and `cargo clippy` after editing to verify no new issues were introduced.",
    ),
    "python": LangPreset(
        name="Python",
        test_cmd="pytest -x",
        lint_cmd="ruff check --output-format json .",
        fmt_check_cmd="ruff format --check .",
        fmt_fix_cmd="ruff format .",
        verify_instructions="Run `ruff format .` and `ruff check .` after editing to verify no new issues were introduced.",
    ),
    "javascript": LangPreset(
        name="JavaScript",
        test_cmd="npm test",
        lint_cmd="npx eslint . --format json",
        fmt_check_cmd="npx prettier --check .",
        fmt_fix_cmd="npx prettier --write .",
        verify_instructions="Run `npx prettier --write .` and `npx eslint .` after editing to verify no new issues were introduced.",
    ),
    "typescript": LangPreset(
        name="TypeScript",
        test_cmd="npm test",
        lint_cmd="npx eslint . --format json",
        fmt_check_cmd="npx prettier --check .",
        fmt_fix_cmd="npx prettier --write .",
        verify_instructions="Run `npx prettier --write .` and `npx eslint .` after editing to verify no new issues were introduced.",
    ),
    "go": LangPreset(
        name="Go",
        test_cmd="go test ./...",
        lint_cmd="staticcheck ./...",
        fmt_check_cmd="gofmt -l .",
        fmt_fix_cmd="gofmt -w .",
        verify_instructions="Run `gofmt -w .` and `go vet ./...` after editing to verify no new issues were introduced.",
    ),
    "java": LangPreset(
        name="Java",
        test_cmd="mvn test -q",
        lint_cmd="mvn checkstyle:check -q",
        fmt_check_cmd=None,
        fmt_fix_cmd=None,
        verify_instructions="Run `mvn checkstyle:check` after editing to verify no new issues were introduced.",
    ),
    "ruby": LangPreset(
        name="Ruby",
        test_cmd="bundle exec rspec",
        lint_cmd="bundle exec rubocop --format json",
        fmt_check_cmd="bundle exec rubocop --format json",
        fmt_fix_cmd="bundle exec rubocop -a",
        verify_instructions="Run `bundle exec rubocop -a` after editing to verify no new issues were introduced.",
    ),
    "swift": LangPreset(
        name="Swift",
        test_cmd="swift test",
        lint_cmd="swiftlint --reporter json",
        fmt_check_cmd="swiftformat --lint .",
        fmt_fix_cmd="swiftformat .",
        verify_instructions="Run `swiftformat .` and `swiftlint` after editing to verify no new issues were introduced.",
    ),
}


# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

@dataclass
class ToolCommands:
    """Shell commands for test, lint, and format. None means skip."""
    test_cmd: Optional[str] = None
    lint_cmd: Optional[str] = None
    fmt_check_cmd: Optional[str] = None
    fmt_fix_cmd: Optional[str] = None
    verify_instructions: str = "Verify no new issues were introduced after editing."
    lang_name: str = "unknown"


@dataclass
class Config:
    sonar_token: str
    sonar_url: str
    sonar_project_key: str
    gh_repo: str
    gh_pr_number: str
    tools: ToolCommands
    max_iterations: int = 8
    start_iteration: int = 1
    poll_interval: int = 30
    poll_timeout: int = 900
    skip_sonar_first: bool = False
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


def _load_config_file(path: str) -> dict:
    """Load a JSON config file. Returns empty dict if not found."""
    p = Path(path)
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text())
    except (json.JSONDecodeError, OSError) as e:
        print(f"[warn] Failed to load config file {path}: {e}")
        return {}


def _cmd_available(cmd: str) -> bool:
    """Check if the first word of a command is available on PATH."""
    binary = cmd.split()[0]
    # Handle npx/bunx wrappers -- the tool itself is always available via them
    if binary in ("npx", "bunx"):
        return shutil.which(binary) is not None
    # Handle "bundle exec X" -- check for bundle
    if binary == "bundle":
        return shutil.which("bundle") is not None
    return shutil.which(binary) is not None


def _check_tools(tools: ToolCommands) -> None:
    """Check that all configured tool commands are available. Warn and disable missing ones."""
    for attr, label in [
        ("test_cmd", "Test"),
        ("lint_cmd", "Lint"),
        ("fmt_check_cmd", "Format check"),
        ("fmt_fix_cmd", "Format fix"),
    ]:
        cmd = getattr(tools, attr)
        if cmd and not _cmd_available(cmd):
            binary = cmd.split()[0]
            print(f"  [warn] {label} tool not found: {binary} (from: {cmd})")
            print(f"         Disabling {label.lower()} step.")
            setattr(tools, attr, None)


def _resolve_tools(args: argparse.Namespace, file_cfg: dict) -> ToolCommands:
    """Build ToolCommands from preset + config file + CLI overrides."""
    # Start from preset if specified
    lang = args.lang or file_cfg.get("lang")
    if lang and lang in PRESETS:
        preset = PRESETS[lang]
        tools = ToolCommands(
            test_cmd=preset.test_cmd,
            lint_cmd=preset.lint_cmd,
            fmt_check_cmd=preset.fmt_check_cmd,
            fmt_fix_cmd=preset.fmt_fix_cmd,
            verify_instructions=preset.verify_instructions,
            lang_name=preset.name,
        )
    elif lang:
        print(f"[error] Unknown language: {lang}")
        print(f"        Available: {', '.join(sorted(PRESETS.keys()))}")
        sys.exit(1)
    else:
        tools = ToolCommands()

    # Config file overrides preset
    for key in ("test_cmd", "lint_cmd", "fmt_check_cmd", "fmt_fix_cmd"):
        if key in file_cfg:
            setattr(tools, key, file_cfg[key])
    if "verify_instructions" in file_cfg:
        tools.verify_instructions = file_cfg["verify_instructions"]
    if "lang_name" in file_cfg:
        tools.lang_name = file_cfg["lang_name"]

    # CLI flags override everything (only if explicitly passed)
    if args.test_cmd is not None:
        tools.test_cmd = args.test_cmd or None
    if args.lint_cmd is not None:
        tools.lint_cmd = args.lint_cmd or None
    if args.fmt_check_cmd is not None:
        tools.fmt_check_cmd = args.fmt_check_cmd or None
    if args.fmt_fix_cmd is not None:
        tools.fmt_fix_cmd = args.fmt_fix_cmd or None

    return tools


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="quality_loop.py",
        description="Automated code quality improvement agent for any language.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=HELP_EPILOG,
    )
    parser.add_argument("--pr", metavar="NUMBER",
        help="PR number to operate on (overrides GH_PR_NUMBER in .env)")
    parser.add_argument("--repo", metavar="OWNER/REPO",
        help="GitHub repo (overrides GH_REPO in .env)")
    parser.add_argument("--max-iterations", type=int, metavar="N",
        help="Hard stop after N iterations (default: 8)")
    parser.add_argument("--start-iteration", type=int, metavar="N",
        help="Starting iteration number for commits (default: auto-detect from git log)")
    parser.add_argument("--poll-interval", type=int, metavar="SECONDS",
        help="Seconds between CI/CodeRabbit polls (default: 30)")
    parser.add_argument("--poll-timeout", type=int, metavar="SECONDS",
        help="Max wait time per push before continuing (default: 900)")
    parser.add_argument("--skip-sonar-first", action="store_true",
        help="Skip SonarQube scan on the first iteration (use cached results)")
    parser.add_argument("--dry-run", action="store_true",
        help="Collect and print all signals, but do not invoke Claude Code or push")
    parser.add_argument("--env-file", metavar="PATH", default=".env",
        help="Path to .env file (default: .env in current directory)")

    # Language / tool configuration
    lang_group = parser.add_argument_group("language and tool configuration")
    lang_group.add_argument("--lang", metavar="LANG",
        choices=sorted(PRESETS.keys()), default="rust",
        help=f"Language preset (default: rust): {', '.join(sorted(PRESETS.keys()))}")
    lang_group.add_argument("--config", metavar="PATH", default=".quality_loop.json",
        help="Path to JSON config file (default: .quality_loop.json)")
    lang_group.add_argument("--test-cmd", metavar="CMD", default=None,
        help="Command to run tests (e.g. 'pytest -x'). Empty string to disable.")
    lang_group.add_argument("--lint-cmd", metavar="CMD", default=None,
        help="Command to run linter (e.g. 'ruff check .'). Empty string to disable.")
    lang_group.add_argument("--fmt-check-cmd", metavar="CMD", default=None,
        help="Command to check formatting (e.g. 'black --check .'). Empty string to disable.")
    lang_group.add_argument("--fmt-fix-cmd", metavar="CMD", default=None,
        help="Command to auto-fix formatting (e.g. 'black .'). Empty string to disable.")

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

    # Auto-detect latest PR number if not provided
    if not os.environ.get("GH_PR_NUMBER"):
        repo = os.environ.get("GH_REPO", "")
        gh_args = ["gh", "pr", "view", "--json", "number", "--jq", ".number"]
        if repo:
            gh_args.extend(["-R", repo])
        try:
            result = subprocess.run(gh_args, capture_output=True, text=True, timeout=15)
            pr_num = result.stdout.strip()
            if result.returncode == 0 and pr_num and pr_num.isdigit():
                print(f"[info] Auto-detected latest PR number: {pr_num}")
                os.environ["GH_PR_NUMBER"] = pr_num
        except subprocess.TimeoutExpired:
            print("[warn] PR auto-detection timed out; skipping. Set GH_PR_NUMBER manually.")
        except FileNotFoundError:
            print("[warn] 'gh' CLI not found; skipping PR auto-detection. Set GH_PR_NUMBER manually.")

    required = ["SONAR_TOKEN", "SONAR_URL", "SONAR_PROJECT_KEY", "GH_REPO", "GH_PR_NUMBER"]
    missing = [k for k in required if not os.environ.get(k)]
    if missing:
        print(f"[error] Missing required configuration: {', '.join(missing)}")
        print(f"        Add them to {args.env_file} or pass as CLI flags.")
        print("        Run with --help for full setup instructions.")
        sys.exit(1)

    # Resolve tool commands: preset -> config file -> CLI flags
    file_cfg = _load_config_file(args.config)
    tools = _resolve_tools(args, file_cfg)

    if not tools.test_cmd and not tools.lint_cmd and not tools.fmt_check_cmd:
        print("[error] No tools configured. Use --lang or --test-cmd/--lint-cmd/--fmt-check-cmd.")
        sys.exit(1)

    # Check tool availability
    print(f"\n  Language: {tools.lang_name}")
    _check_tools(tools)

    cmds = [
        ("Test", tools.test_cmd),
        ("Lint", tools.lint_cmd),
        ("Format check", tools.fmt_check_cmd),
        ("Format fix", tools.fmt_fix_cmd),
    ]
    for label, cmd in cmds:
        status = cmd if cmd else "(disabled)"
        print(f"  {label:14s} {status}")

    return Config(
        sonar_token=os.environ["SONAR_TOKEN"],
        sonar_url=os.environ["SONAR_URL"].rstrip("/"),
        sonar_project_key=os.environ["SONAR_PROJECT_KEY"],
        gh_repo=os.environ["GH_REPO"],
        gh_pr_number=os.environ["GH_PR_NUMBER"],
        tools=tools,
        max_iterations=int(os.environ.get("MAX_ITERATIONS", 8)),
        start_iteration=args.start_iteration if args.start_iteration is not None else _detect_last_iteration() + 1,
        poll_interval=int(os.environ.get("POLL_INTERVAL", 30)),
        poll_timeout=int(os.environ.get("POLL_TIMEOUT", 900)),
        skip_sonar_first=args.skip_sonar_first,
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
            if len(arg) > _MAX_ARG_DISPLAY_LEN:
                arg = f"{arg[:_MAX_ARG_DISPLAY_LEN]}... ({len(arg)} chars)"
        redacted.append(arg)
    return redacted


def run(cmd: list[str], check=True, capture=False, timeout: Optional[int] = None) -> subprocess.CompletedProcess:
    print(f"  $ {' '.join(_redact_cmd(cmd))}")
    try:
        return subprocess.run(
            cmd,
            check=check,
            capture_output=capture,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        print(f"  [error] Command timed out after {timeout}s: {' '.join(_redact_cmd(cmd))}")
        return subprocess.CompletedProcess(cmd, returncode=124, stdout="", stderr="")
    except FileNotFoundError:
        print(f"  [error] Command not found: {cmd[0]}")
        return subprocess.CompletedProcess(cmd, returncode=127, stdout="", stderr="")
    except subprocess.CalledProcessError as e:
        print(f"  [error] Command failed (exit {e.returncode}): {' '.join(_redact_cmd(cmd))}")
        return subprocess.CompletedProcess(cmd, returncode=e.returncode,
                                           stdout=e.stdout or "", stderr=e.stderr or "")


def run_shell(cmd: str, check=True, capture=False, timeout: Optional[int] = None) -> subprocess.CompletedProcess:
    """Run a shell command string. Used for user-configured commands that may contain pipes etc."""
    print(f"  $ {cmd}")
    try:
        return subprocess.run(
            cmd,
            shell=True,
            check=check,
            capture_output=capture,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        print(f"  [error] Command timed out after {timeout}s: {cmd}")
        return subprocess.CompletedProcess(cmd, returncode=124, stdout="", stderr="")
    except subprocess.CalledProcessError as e:
        print(f"  [error] Command failed (exit {e.returncode}): {cmd}")
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
# Generic test / lint / format
# ---------------------------------------------------------------------------

def run_tests(tools: ToolCommands) -> tuple[bool, str]:
    """Returns (passed, error_output). Returns (True, "") if no test command configured."""
    if not tools.test_cmd:
        print("  (no test command configured -- skipping)")
        return True, ""
    result = run_shell(tools.test_cmd, check=False, capture=True)
    output = ((result.stdout or "") + "\n" + (result.stderr or "")).strip()
    if result.returncode != 0:
        # Show last 40 lines so user can see what failed
        lines = output.splitlines()
        tail = lines[-40:] if len(lines) > 40 else lines
        print(f"  Test output (exit {result.returncode}):")
        for line in tail:
            print(f"    {line}")
    else:
        print("  Tests passed.")
    return result.returncode == 0, output


def lint_issues(tools: ToolCommands) -> list[str]:
    """Return lint diagnostics as a list of strings."""
    if not tools.lint_cmd:
        return []
    result = run_shell(tools.lint_cmd, check=False, capture=True)
    if result.returncode == 0:
        return []
    # Combine stdout and stderr -- different linters output to different streams
    output = (result.stdout or "").strip()
    stderr = (result.stderr or "").strip()
    lines = []
    if output:
        lines.extend(output.splitlines())
    if stderr and stderr != output:
        lines.extend(stderr.splitlines())
    return lines if lines else ["Lint issues found (see command output above)"]


def fmt_check_issues(tools: ToolCommands) -> list[str]:
    """Return list of formatting issues."""
    if not tools.fmt_check_cmd:
        return []
    result = run_shell(tools.fmt_check_cmd, check=False, capture=True)
    if result.returncode == 0:
        return []
    output = (result.stdout or "").strip()
    if output:
        return output.splitlines()
    return [f"Formatting issues found (run: {tools.fmt_fix_cmd or tools.fmt_check_cmd})"]


def fmt_fix(tools: ToolCommands) -> None:
    """Run the format fix command if configured."""
    if not tools.fmt_fix_cmd:
        return
    run_shell(tools.fmt_fix_cmd, check=False)


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
    ], timeout=300)
    if result.returncode != 0:
        print("  [warn] SonarQube scan failed. Continuing without SonarQube issues.")
        return False
    return True


def sonar_issues(cfg: Config) -> list[dict]:
    """Fetch open issues from SonarQube API."""
    data = _sonar_api(cfg, "/api/issues/search", {
        "componentKeys": cfg.sonar_project_key,
        "resolved": "false",
        "ps": 100,
    })
    return data.get("issues", []) if data else []


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


def sonar_hotspots(cfg: Config) -> list[dict]:
    """Fetch security hotspots that need review from SonarQube API."""
    data = _sonar_api(cfg, "/api/hotspots/search", {
        "projectKey": cfg.sonar_project_key,
        "status": "TO_REVIEW",
        "ps": 100,
    })
    return data.get("hotspots", []) if data else []


def format_sonar_hotspots(hotspots: list[dict]) -> list[str]:
    formatted = []
    for h in hotspots:
        component = h.get("component", "").split(":")[-1]
        line = h.get("line", "?")
        message = h.get("message", "")
        vulnerability = h.get("vulnerabilityProbability", "UNKNOWN")
        category = h.get("securityCategory", "")
        loc = f" ({component}:{line}, category: {category})" if component else ""
        formatted.append(f"[HOTSPOT/{vulnerability}] {message}{loc}")
    return formatted


def _sonar_api(cfg: Config, path: str, params: dict) -> Optional[dict]:
    """Make a SonarQube API request. Returns parsed JSON or None on failure."""
    import urllib.request
    import urllib.parse
    import base64

    qs = urllib.parse.urlencode(params)
    url = f"{cfg.sonar_url}{path}?{qs}"
    req = urllib.request.Request(url)
    token_b64 = base64.b64encode(f"{cfg.sonar_token}:".encode()).decode()
    req.add_header("Authorization", f"Basic {token_b64}")

    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read())
    except Exception as e:
        print(f"  [warn] SonarQube API error ({path}): {e}")
        return None


def sonar_duplication_density(cfg: Config) -> float:
    """Fetch duplicated_lines_density from /api/measures/component. Returns 0.0 on failure."""
    data = _sonar_api(cfg, "/api/measures/component", {
        "component": cfg.sonar_project_key,
        "metricKeys": "duplicated_lines_density",
    })
    if not data:
        return 0.0
    measures = data.get("component", {}).get("measures", [])
    for m in measures:
        if m.get("metric") == "duplicated_lines_density":
            try:
                return float(m.get("value", "0"))
            except (ValueError, TypeError):
                return 0.0
    return 0.0


def sonar_duplicated_files(cfg: Config) -> list[str]:
    """Find files with duplications via /api/measures/component_tree."""
    data = _sonar_api(cfg, "/api/measures/component_tree", {
        "component": cfg.sonar_project_key,
        "metricKeys": "duplicated_blocks,duplicated_lines",
        "qualifiers": "FIL",
        "metricSort": "duplicated_blocks",
        "metricSortFilter": "withMeasuresOnly",
        "s": "metric",
        "asc": "false",
        "ps": 50,
    })
    if not data:
        return []
    files = []
    for comp in data.get("components", []):
        measures = {m["metric"]: m.get("value", "0") for m in comp.get("measures", [])}
        blocks = int(measures.get("duplicated_blocks", "0"))
        if blocks > 0:
            files.append(comp.get("key", ""))
    return files


def sonar_file_duplications(cfg: Config, file_key: str) -> list[dict]:
    """Fetch detailed duplication blocks for a single file via /api/duplications/show."""
    data = _sonar_api(cfg, "/api/duplications/show", {"key": file_key})
    if not data:
        return []
    return data.get("duplications", [])


def sonar_duplications_detailed(cfg: Config) -> list[list[str]]:
    """Fetch detailed duplication info from SonarQube.
    Returns a list of groups -- each group is a list of issue strings
    for one duplicated file, suitable for a separate Claude Code call."""
    # Match Dirigent: skip all duplication items if density is below 3.0%
    density = sonar_duplication_density(cfg)
    print(f"  Duplication density: {density:.1f}%")
    if density < 3.0:
        print("  Below 3.0% threshold -- skipping duplications.")
        return []

    dup_files = sonar_duplicated_files(cfg)
    if not dup_files:
        return []

    print(f"  Files with duplications: {len(dup_files)}")
    groups: list[list[str]] = []
    for file_key in dup_files:
        file_path = file_key.split(":")[-1]
        dups = sonar_file_duplications(cfg, file_key)
        if not dups:
            continue
        lines = []
        for dup in dups:
            blocks = dup.get("blocks", [])
            if len(blocks) < 2:
                continue
            parts = []
            for block in blocks:
                bkey = block.get("_ref", "")
                # Resolve component key from the duplications response files map
                bfile = block.get("component", file_path)
                if ":" in bfile:
                    bfile = bfile.split(":")[-1]
                bfrom = block.get("from", "?")
                bsize = block.get("size", "?")
                parts.append(f"{bfile}:{bfrom}-{int(bfrom) + int(bsize) - 1 if isinstance(bfrom, int) and isinstance(bsize, int) else '?'}")
            lines.append(f"  Duplicated block: {' <-> '.join(parts)}")
        if lines:
            groups.append([f"File: {file_path}"] + lines)

    return groups


# ---------------------------------------------------------------------------
# GitHub / CodeRabbit
# ---------------------------------------------------------------------------

def _check_result(check: dict) -> Optional[str]:
    """Extract the result of a check, handling both CheckRun (conclusion) and StatusContext (state)."""
    result = check.get("conclusion") or check.get("state")
    if result:
        return result.upper()
    return None


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
            return True
        results = [_check_result(c) for c in checks]
        resolved = [r for r in results if r is not None]
        if not resolved:
            return False
        passing = all(r in ("SUCCESS", "SKIPPED", "NEUTRAL") for r in resolved)
        if not passing:
            failed = [r for r in resolved if r not in ("SUCCESS", "SKIPPED", "NEUTRAL")]
            print(f"  Check results: {failed}")
        return passing
    except json.JSONDecodeError:
        return False


def coderabbit_comments(cfg: Config, since: str = "") -> list[str]:
    """Return review comments and review-body findings left by coderabbitai[bot].

    Fetches from two endpoints:
      - /pulls/{n}/comments  -- line-level review comments
      - /pulls/{n}/reviews   -- review bodies (contain grouped nitpick details)
    """
    if since:
        print(f"  Filtering comments created after: {since}")
    results = []

    # 1) Line-level review comments
    output = gh(
        "api",
        f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/comments",
        "--paginate",
    )
    if output:
        try:
            comments = json.loads(output)
        except json.JSONDecodeError:
            comments = []
        cr_total = 0
        cr_skipped_since = 0
        for c in comments:
            user = c.get("user", {}).get("login", "")
            if "coderabbitai" not in user:
                continue
            cr_total += 1
            created = c.get("created_at", "")
            if since and created <= since:
                cr_skipped_since += 1
                continue
            body = c.get("body", "").strip()
            if not body:
                continue
            path = c.get("path", "")
            line = c.get("line") or c.get("original_line") or "?"
            results.append(f"{path}:{line} -- {body[:500]}")
        print(f"  PR comments: {cr_total} from coderabbit, {cr_skipped_since} skipped (before since), {len(results)} kept")
    else:
        print("  [warn] No response from GitHub API for PR comments.")

    # 2) Review bodies (contain nitpick details and summaries)
    review_count_before = len(results)
    output = gh(
        "api",
        f"/repos/{cfg.gh_repo}/pulls/{cfg.gh_pr_number}/reviews",
        "--paginate",
    )
    if output:
        try:
            reviews = json.loads(output)
        except json.JSONDecodeError:
            reviews = []
        cr_total = 0
        cr_skipped_since = 0
        for r in reviews:
            user = r.get("user", {}).get("login", "")
            if "coderabbitai" not in user:
                continue
            cr_total += 1
            submitted = r.get("submitted_at", "")
            if since and submitted <= since:
                cr_skipped_since += 1
                continue
            body = r.get("body", "").strip()
            if not body:
                continue
            # Skip pure summary lines like "Actionable comments posted: 4"
            if body.startswith("Actionable comments posted:"):
                continue
            results.append(f"[review] {body[:500]}")
        print(f"  PR reviews: {cr_total} from coderabbit, {cr_skipped_since} skipped (before since), {len(results) - review_count_before} kept")
    else:
        print("  [warn] No response from GitHub API for PR reviews.")

    return results


def _coderabbit_review_pending(cfg: Config) -> bool:
    """Return True if CodeRabbit has a pending status check (review in progress)."""
    output = gh(
        "pr", "view", cfg.gh_pr_number,
        "--repo", cfg.gh_repo,
        "--json", "statusCheckRollup",
    )
    try:
        data = json.loads(output)
        checks = data.get("statusCheckRollup", [])
        for c in checks:
            name = (c.get("name") or c.get("context") or "").lower()
            if "coderabbit" not in name:
                continue
            result = _check_result(c)
            if result is None:
                return True  # CodeRabbit check exists but has no conclusion yet
        return False
    except json.JSONDecodeError:
        return False


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
    initial_delay = 60
    banner(f"Waiting {initial_delay}s for CI + CodeRabbit to start (timeout {cfg.poll_timeout}s)")
    time.sleep(initial_delay)

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
    """Invoke Claude Code with stream-json output to show live progress."""
    fd = tempfile.NamedTemporaryFile(
        mode="w",
        prefix=f"quality_loop_prompt_iter{iteration}_",
        suffix=".txt",
        delete=False,
    )
    fd.write(prompt)
    prompt_file = fd.name
    fd.close()
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
    tools: ToolCommands,
    sonar: list[str],
    hotspots: list[str],
    lint: list[str],
    fmt: list[str],
    coderabbit: list[str],
) -> str:
    parts = [
        f"You are a code quality agent for a {tools.lang_name} project.",
        "Fix ALL of the following issues. Do not change any functionality --",
        "only improve code quality. After each fix, ensure the code still compiles/runs.",
        "IMPORTANT: Do NOT run git commit. Only edit the files -- the caller handles committing.",
        "",
    ]
    if sonar:
        parts.append("== SonarQube issues ==")
        parts.extend(sonar)
        parts.append("")
    if hotspots:
        parts.append("== SonarQube security hotspots (TO_REVIEW) ==")
        parts.extend(hotspots)
        parts.append("")
    if lint:
        parts.append("== Lint diagnostics ==")
        parts.extend(lint)
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
        f"Fix all issues above. {tools.verify_instructions} Do not modify tests."
    )
    return "\n".join(parts)


def build_dedup_prompt(tools: ToolCommands, dup_group: list[str]) -> str:
    """Build a prompt for fixing one duplication group in a separate Claude call."""
    parts = [
        f"You are a code quality agent for a {tools.lang_name} project.",
        "Reduce code duplication as described below. Extract shared logic into",
        "functions, modules, or helper methods. Do not change any functionality.",
        "After each change, ensure the code still compiles/runs.",
        "IMPORTANT: Do NOT run git commit. Only edit the files -- the caller handles committing.",
        "",
        "== SonarQube duplications ==",
    ]
    parts.extend(dup_group)
    parts.append("")
    parts.append(
        f"{tools.verify_instructions} Do not modify tests."
    )
    return "\n".join(parts)


def _build_compile_fix_prompt(tools: ToolCommands, test_output: str) -> str:
    """Build a prompt for Claude to fix compilation or test errors."""
    # Truncate very long output to keep the prompt manageable
    max_len = 8000
    if len(test_output) > max_len:
        test_output = "... (truncated) ...\n" + test_output[-max_len:]

    return "\n".join([
        f"You are a code quality agent for a {tools.lang_name} project.",
        "The code is failing to compile or tests are failing after recent changes.",
        "Fix the compilation/test errors below. Do not change any functionality",
        "beyond what is needed to fix these errors.",
        "IMPORTANT: Do NOT run git commit. Only edit the files -- the caller handles committing.",
        "",
        "== Build/test errors ==",
        test_output,
        "",
        f"{tools.verify_instructions} Do not modify tests.",
    ])


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


def _iteration_state_path() -> Path:
    """Path to the file that persists the last completed iteration number."""
    import hashlib
    cwd = str(Path.cwd().resolve())
    path_hash = hashlib.sha256(cwd.encode()).hexdigest()[:12]
    repo_id = f"{Path.cwd().name}_{path_hash}"
    state_dir = Path.home() / ".cache" / "quality_loop"
    state_dir.mkdir(parents=True, exist_ok=True)
    return state_dir / f"iteration_{repo_id}.txt"


def _detect_last_iteration() -> int:
    """Detect the last quality loop iteration number.

    Uses three signals (highest wins):
    1. Git log marker commits ("quality loop iteration N")
    2. Persisted state file (guards against Claude committing with its own messages)
    3. Orphan commit detection: counts non-marker commits after the last marker
       that look like Claude Code output, grouped by time proximity (gap > 10 min
       = separate iteration). This catches iterations where Claude committed on
       its own and no marker was written.
    """
    from datetime import datetime

    # --- Signal 1: git log markers ---
    git_highest = 0
    marker_sha = ""
    result = subprocess.run(
        ["git", "log", "--format=%H %s", "-50"],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0 and result.stdout:
        for line in result.stdout.splitlines():
            m = re.search(r"quality loop iteration (\d+)", line)
            if m:
                n = int(m.group(1))
                if n > git_highest:
                    git_highest = n
                    marker_sha = line.split()[0]

    # --- Signal 2: persisted state file ---
    file_highest = 0
    state_path = _iteration_state_path()
    if state_path.exists():
        try:
            file_highest = int(state_path.read_text().strip())
        except (ValueError, OSError):
            pass

    # --- Signal 3: orphan commit detection (timestamp grouping) ---
    orphan_highest = 0
    if git_highest > 0 and marker_sha:
        gap_result = subprocess.run(
            ["git", "log", "--format=%aI %s", f"{marker_sha}..HEAD"],
            check=False, capture_output=True, text=True,
        )
        if gap_result.returncode == 0 and gap_result.stdout:
            # Collect timestamps of commits that look like Claude Code output
            _skip = ("Dirigent:", "Merge", "release:", "docs:", "quality loop", "quality_loop")
            timestamps: list[datetime] = []
            for line in gap_result.stdout.splitlines():
                parts = line.split(" ", 1)
                if len(parts) < 2:
                    continue
                ts_str, msg = parts
                if any(p in msg for p in _skip):
                    continue
                if re.match(r"(feat|fix|chore|refactor):", msg):
                    try:
                        timestamps.append(datetime.fromisoformat(ts_str))
                    except ValueError:
                        pass

            if timestamps:
                timestamps.sort()
                groups = 1
                for i in range(1, len(timestamps)):
                    gap_seconds = abs((timestamps[i] - timestamps[i - 1]).total_seconds())
                    if gap_seconds > 600:  # 10-minute gap = new iteration
                        groups += 1
                orphan_highest = git_highest + groups
                print(f"  [note] Found {len(timestamps)} orphan commit(s) after marker {git_highest} "
                      f"({groups} estimated iteration(s) by timestamp grouping)")

    highest = max(git_highest, file_highest, orphan_highest)
    if file_highest > git_highest and file_highest > 0:
        print(f"  [note] git log shows iteration {git_highest}, but state file has {file_highest} -- using {highest}")
    return highest


def _save_iteration(iteration: int) -> None:
    """Persist the last completed iteration number to disk (atomic write)."""
    target = _iteration_state_path()
    tmp = target.with_suffix(".tmp")
    tmp.write_text(str(iteration))
    tmp.replace(target)


def git_commit_push(iteration: int, files_to_stage: Optional[list[str]] = None, sha_before: str = "") -> bool:
    """Commit and push changes. Returns True if push succeeded.

    If sha_before is provided and Claude Code has made its own commits since
    that SHA, those commits are soft-reset and re-committed under a single
    quality-loop marker commit so that iteration detection stays accurate.
    """
    # Detect commits Claude Code made on its own (despite the "do not commit" instruction)
    claude_committed = False
    if sha_before:
        current = current_sha()
        if current != sha_before:
            count_result = run(
                ["git", "rev-list", "--count", f"{sha_before}..HEAD"],
                check=False, capture=True,
            )
            n_commits = int(count_result.stdout.strip()) if count_result.returncode == 0 else 0
            if n_commits > 0:
                print(f"  [note] Claude Code created {n_commits} commit(s) -- squashing into quality loop marker.")
                reset_result = run(["git", "reset", "--soft", sha_before], check=False)
                if reset_result.returncode != 0:
                    print("  [error] git reset --soft failed -- aborting squash.")
                    return False
                claude_committed = True

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
    if result.returncode == 0 and not claude_committed:
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

@dataclass
class Signals:
    sonar: list[str]
    hotspots: list[str]
    dup_groups: list[list[str]]
    lint: list[str]
    fmt: list[str]
    coderabbit: list[str]

    @property
    def main_total(self) -> int:
        """Count of non-duplication issues (handled in the main Claude call)."""
        return len(self.sonar) + len(self.hotspots) + len(self.lint) + len(self.fmt) + len(self.coderabbit)

    @property
    def dup_total(self) -> int:
        return sum(len(g) for g in self.dup_groups)


def collect_signals(cfg: Config, cr_since: str = "", skip_sonar: bool = False) -> Signals:
    """Collect all quality signals."""
    if skip_sonar:
        print("\n[1/6] SonarQube scan -- skipped (--skip-sonar-first)")
        print("\n[2/6] SonarQube security hotspots -- skipped")
        print("\n[3/6] SonarQube duplications -- skipped")
        s_issues, h_issues, d_groups = [], [], []
    else:
        print("\n[1/6] Running SonarQube scan ...")
        scan_ok = sonar_scan(cfg)
        s_issues = format_sonar_issues(sonar_issues(cfg)) if scan_ok else []
        print(f"  SonarQube issues: {len(s_issues)}")

        print("\n[2/6] Fetching SonarQube security hotspots ...")
        h_issues = format_sonar_hotspots(sonar_hotspots(cfg)) if scan_ok else []
        print(f"  Security hotspots: {len(h_issues)}")

        print("\n[3/6] Fetching SonarQube duplications ...")
        d_groups = sonar_duplications_detailed(cfg) if scan_ok else []
        print(f"  Duplication groups: {len(d_groups)}")

    print("\n[4/6] Running lint ...")
    l_issues = lint_issues(cfg.tools)
    print(f"  Lint issues: {len(l_issues)}")

    print("\n[5/6] Running format check ...")
    f_issues = fmt_check_issues(cfg.tools)
    print(f"  Fmt issues: {len(f_issues)}")

    print("\n[6/6] Fetching CodeRabbit comments ...")
    cr_issues = coderabbit_comments(cfg, since=cr_since)
    if not cr_issues:
        cr_wait_minutes = 15
        print(f"  No comments yet. Waiting {cr_wait_minutes}m for CodeRabbit to finish reviewing ...")
        for remaining in range(cr_wait_minutes, 0, -1):
            print(f"\r  CodeRabbit wait: {remaining:2d}m remaining ", end="", flush=True)
            time.sleep(60)
        print("\r  CodeRabbit wait: done.              ", flush=True)
        cr_issues = coderabbit_comments(cfg, since=cr_since)
    cr_all = len(cr_issues)
    cr_issues = _filter_unseen_comments(cfg, cr_issues)
    print(f"  CodeRabbit comments: {len(cr_issues)} new ({cr_all} total, {cr_all - len(cr_issues)} already seen)")

    return Signals(
        sonar=s_issues, hotspots=h_issues, dup_groups=d_groups,
        lint=l_issues, fmt=f_issues, coderabbit=cr_issues,
    )


def _apply_fixes(cfg: Config, prompt: str, iteration: int) -> None:
    """Invoke Claude Code, auto-format, and verify tests still pass.
    If tests/compilation break, retries by asking Claude to fix the errors."""
    max_compile_retries = 3

    print("  Invoking Claude Code ...")
    success = claude_fix(prompt, iteration)
    if not success:
        print("[warn] Claude Code exited with an error. Continuing anyway.")

    if cfg.tools.fmt_fix_cmd:
        print("\n  Running format fix ...")
        fmt_fix(cfg.tools)

    for attempt in range(max_compile_retries + 1):
        print("\n  Running tests (regression guard) ...")
        passed, test_output = run_tests(cfg.tools)
        if passed:
            return

        if attempt == max_compile_retries:
            print(f"[error] Tests still failing after {max_compile_retries} fix attempts. Stopping.")
            print("  Check the diff, fix manually, and re-run.")
            sys.exit(1)

        print(f"\n[warn] Tests/compilation broke after changes (attempt {attempt + 1}/{max_compile_retries}). Asking Claude Code to fix ...")
        fix_prompt = _build_compile_fix_prompt(cfg.tools, test_output)
        success = claude_fix(fix_prompt, iteration)
        if not success:
            print("[warn] Claude Code exited with an error. Continuing anyway.")

        if cfg.tools.fmt_fix_cmd:
            print("\n  Running format fix ...")
            fmt_fix(cfg.tools)


def _commit_and_wait(cfg: Config, iteration: int, files_to_stage: Optional[list[str]] = None,
                     sha_before_claude: str = "") -> bool:
    """Commit, push, and wait for CI/CodeRabbit if the push succeeded."""
    sha_before = current_sha()
    pushed = git_commit_push(iteration, files_to_stage=files_to_stage,
                             sha_before=sha_before_claude or sha_before)
    sha_after = current_sha()

    if sha_before == sha_after:
        print("  No changes committed -- Claude may not have produced a fix.")
        return False

    if pushed:
        wait_for_coderabbit(cfg)
        return True

    print("  Skipping CI/CodeRabbit wait (push did not succeed).")
    return False


def _cr_since_path(cfg: Config) -> Path:
    """Path to the file that persists the CodeRabbit comment timestamp across restarts."""
    return Path(f"/tmp/quality_loop_cr_since_{cfg.gh_repo.replace('/', '_')}_{cfg.gh_pr_number}.txt")


def _load_cr_since(cfg: Config) -> str:
    """Load persisted cr_since timestamp, or return empty string."""
    p = _cr_since_path(cfg)
    print(f"  cr_since file: {p}")
    if p.exists():
        ts = p.read_text().strip()
        if ts:
            print(f"  Resuming with CodeRabbit comments since {ts}")
            return ts
        print("  cr_since file exists but is empty.")
    else:
        print("  cr_since file not found -- fetching all comments.")
    return ""


def _save_cr_since(cfg: Config, ts: str) -> None:
    """Persist cr_since timestamp for restart resilience."""
    _cr_since_path(cfg).write_text(ts)


def _cr_seen_path(cfg: Config) -> Path:
    """Path to the file that stores hashes of already-seen CodeRabbit comments."""
    return Path(f"/tmp/quality_loop_cr_seen_{cfg.gh_repo.replace('/', '_')}_{cfg.gh_pr_number}.txt")


def _load_seen_cr_hashes(cfg: Config) -> set[str]:
    """Load set of hashes for CodeRabbit comments already processed."""
    p = _cr_seen_path(cfg)
    if p.exists():
        return set(line.strip() for line in p.read_text().splitlines() if line.strip())
    return set()


def _save_seen_cr_hashes(cfg: Config, hashes: set[str]) -> None:
    """Persist seen CodeRabbit comment hashes."""
    _cr_seen_path(cfg).write_text("\n".join(sorted(hashes)) + "\n")


def _hash_comment(comment: str) -> str:
    """Create a short hash of a CodeRabbit comment string."""
    return hashlib.sha256(comment.encode()).hexdigest()[:16]


def _filter_unseen_comments(cfg: Config, comments: list[str]) -> list[str]:
    """Filter out CodeRabbit comments we've already seen. Updates the seen set."""
    seen = _load_seen_cr_hashes(cfg)
    unseen = []
    for c in comments:
        h = _hash_comment(c)
        if h not in seen:
            unseen.append(c)
            seen.add(h)
    _save_seen_cr_hashes(cfg, seen)
    return unseen


def _run_iteration(cfg: Config, iteration: int, cr_since: str, skip_sonar: bool = False) -> str:
    """Run one quality loop iteration. Returns updated cr_since timestamp,
    or empty string to signal that all issues are resolved."""
    from datetime import datetime, timezone

    signals = collect_signals(cfg, cr_since=cr_since, skip_sonar=skip_sonar)
    total = signals.main_total + signals.dup_total

    if total == 0:
        return ""

    print(f"\n  Total issues: {total} ({signals.main_total} main + {signals.dup_total} duplication).")

    sha_before_claude = current_sha()
    dirty_before = _get_dirty_files()

    # --- Main Claude call: sonar issues, hotspots, lint, fmt, coderabbit ---
    if signals.main_total > 0:
        prompt = build_fix_prompt(
            cfg.tools, signals.sonar, signals.hotspots,
            signals.lint, signals.fmt, signals.coderabbit,
        )
        if cfg.dry_run:
            print("\n[dry-run] Main prompt:\n")
            print(prompt)
        else:
            banner(f"Iteration {iteration} -- main fixes ({signals.main_total} issues)")
            _apply_fixes(cfg, prompt, iteration)

    # --- Separate Claude calls for duplication groups ---
    if signals.dup_groups:
        for i, group in enumerate(signals.dup_groups, 1):
            dup_prompt = build_dedup_prompt(cfg.tools, group)
            if cfg.dry_run:
                print(f"\n[dry-run] Duplication prompt {i}/{len(signals.dup_groups)}:\n")
                print(dup_prompt)
            else:
                banner(f"Iteration {iteration} -- dedup {i}/{len(signals.dup_groups)}: {group[0]}")
                _apply_fixes(cfg, dup_prompt, iteration)

    if cfg.dry_run:
        print("\n[dry-run] Stopping after first iteration.")
        sys.exit(0)

    dirty_after = _get_dirty_files()
    new_files = sorted(dirty_after - dirty_before)

    new_cr_since = datetime.now(timezone.utc).isoformat()
    pushed = _commit_and_wait(cfg, iteration, files_to_stage=new_files or None,
                              sha_before_claude=sha_before_claude)
    if pushed:
        _save_cr_since(cfg, new_cr_since)

    return new_cr_since


def main() -> None:
    from datetime import datetime, timezone

    args = parse_args()
    cfg = build_config(args)

    if cfg.start_iteration > 1:
        print(f"  Resuming from iteration {cfg.start_iteration} (detected from git log / state file)")

    if cfg.dry_run:
        print("[dry-run] No Claude Code invocations or git pushes will happen.")

    banner("Baseline: tests")
    passed, test_output = run_tests(cfg.tools)
    if not passed:
        max_baseline_retries = 3
        print(f"[warn] Baseline tests/compilation failing. Asking Claude Code to fix (up to {max_baseline_retries} attempts) ...")
        for attempt in range(max_baseline_retries):
            fix_prompt = _build_compile_fix_prompt(cfg.tools, test_output)
            claude_fix(fix_prompt, 0)
            if cfg.tools.fmt_fix_cmd:
                fmt_fix(cfg.tools)
            passed, test_output = run_tests(cfg.tools)
            if passed:
                print("  Baseline tests now green.")
                # Commit the baseline fix before entering the quality loop
                baseline_iter = cfg.start_iteration - 1 if cfg.start_iteration > 1 else 0
                baseline_cr_since = datetime.now(timezone.utc).isoformat()
                pushed = _commit_and_wait(cfg, baseline_iter)
                if pushed:
                    _save_cr_since(cfg, baseline_cr_since)
                break
            if attempt < max_baseline_retries - 1:
                print(f"[warn] Still failing (attempt {attempt + 1}/{max_baseline_retries}). Retrying ...")
        else:
            print(f"[error] Could not fix baseline failures after {max_baseline_retries} attempts. Fix manually.")
            sys.exit(1)
    else:
        print("  Baseline tests green.")

    cr_since = _load_cr_since(cfg)

    end_iteration = cfg.start_iteration + cfg.max_iterations
    for iteration in range(cfg.start_iteration, end_iteration):
        banner(f"Iteration {iteration}/{end_iteration - 1}")
        skip_sonar = cfg.skip_sonar_first and iteration == cfg.start_iteration
        cr_since = _run_iteration(cfg, iteration, cr_since, skip_sonar=skip_sonar)
        _save_iteration(iteration)
        if not cr_since:
            banner("All signals clean -- loop complete!")
            sys.exit(0)

    print(f"\n[warn] Reached max iterations ({cfg.max_iterations}). Stopping.")
    print("  Some issues may remain. Check the PR manually.")
    sys.exit(2)


if __name__ == "__main__":
    main()
