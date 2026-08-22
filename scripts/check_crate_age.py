#!/usr/bin/env python3
"""
check_crate_age.py -- Enforce the dependency cooldown for the Dirigent repository.

A freshly published crate version is the most likely to be yanked, broken, or
compromised. Before adopting one -- typically while fixing a `cargo audit`
finding -- verify it has been on crates.io long enough for the ecosystem to
notice a problem. The repository rule is 24 hours (see the comment above the
dependency block in Cargo.toml and .cargo/audit.toml).

Each argument is a crate name, optionally pinned to a version:

    h2@0.4.18       check exactly this version
    h2              check the newest non-yanked version

With no arguments the script reads Cargo.lock and checks every version pinned
there, which is the cheap way to confirm a `cargo update` did not pull in
something published minutes ago.

Usage:
    python3 scripts/check_crate_age.py h2@0.4.18 webbrowser@1.2.4
    python3 scripts/check_crate_age.py h2                 # newest version
    python3 scripts/check_crate_age.py                    # every crate in Cargo.lock
    python3 scripts/check_crate_age.py --cooldown-hours 48 h2@0.4.18

Requirements: network access to crates.io. Standard library only.

EXIT CODES
  0  every checked version is older than the cooldown -- safe to adopt
  1  at least one version is still inside the cooldown window
  2  a crate or version could not be looked up, or Cargo.lock is unreadable
"""

import argparse
import json
import sys
import tomllib
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

CRATES_IO_API = "https://crates.io/api/v1/crates"
# crates.io asks for a descriptive agent that identifies the caller.
USER_AGENT = "dirigent-check-crate-age (https://github.com/choas/Dirigent)"
REPO_ROOT = Path(__file__).resolve().parent.parent


def fetch_versions(crate):
    """Return the crates.io version list for `crate`, newest first."""
    request = urllib.request.Request(
        f"{CRATES_IO_API}/{crate}/versions",
        headers={"User-Agent": USER_AGENT},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as exc:
        raise LookupError(f"crates.io returned HTTP {exc.code} for '{crate}'") from exc
    except (urllib.error.URLError, TimeoutError) as exc:
        raise LookupError(f"could not reach crates.io for '{crate}': {exc}") from exc
    except json.JSONDecodeError as exc:
        raise LookupError(f"malformed crates.io response for '{crate}'") from exc

    versions = payload.get("versions")
    if not versions:
        raise LookupError(f"crates.io listed no versions for '{crate}'")
    return versions


def select_version(crate, versions, wanted):
    """Pick `wanted` from `versions`, or the newest non-yanked release."""
    if wanted is not None:
        for version in versions:
            if version["num"] == wanted:
                return version
        raise LookupError(f"'{crate}' has no version {wanted} on crates.io")

    for version in versions:
        if not version["yanked"]:
            return version
    raise LookupError(f"every published version of '{crate}' is yanked")


def parse_published(version):
    """Parse a crates.io `created_at` timestamp into an aware datetime."""
    # crates.io returns RFC 3339 with a 'Z' suffix and sub-second precision;
    # fromisoformat only learned to accept 'Z' in 3.11.
    stamp = version["created_at"].replace("Z", "+00:00")
    return datetime.fromisoformat(stamp)


def parse_spec(spec):
    """Split a `crate` or `crate@version` argument."""
    name, separator, version = spec.partition("@")
    if not name or (separator and not version):
        raise ValueError(f"malformed crate spec: '{spec}'")
    return name, (version if separator else None)


def specs_from_lockfile(path):
    """Return every (name, version) pair pinned in Cargo.lock, minus local crates."""
    try:
        with open(path, "rb") as handle:
            lockfile = tomllib.load(handle)
    except OSError as exc:
        raise LookupError(f"could not read {path}: {exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise LookupError(f"could not parse {path}: {exc}") from exc

    # Workspace members and path/git dependencies carry no registry source and
    # have no crates.io publication date to check.
    return [
        (package["name"], package["version"])
        for package in lockfile.get("package", [])
        if package.get("source", "").startswith("registry+")
    ]


def format_age(delta):
    hours = delta.total_seconds() / 3600
    if hours < 48:
        return f"{hours:.1f}h"
    return f"{hours / 24:.1f}d"


def main():
    parser = argparse.ArgumentParser(
        description="Verify crate versions are older than the dependency cooldown.",
        epilog="With no CRATE arguments, every registry crate in Cargo.lock is checked.",
    )
    parser.add_argument(
        "crates",
        metavar="CRATE",
        nargs="*",
        help="crate name, optionally pinned as name@version",
    )
    parser.add_argument(
        "--cooldown-hours",
        type=float,
        default=24.0,
        help="minimum age in hours before a version may be adopted (default: 24)",
    )
    parser.add_argument(
        "--lockfile",
        type=Path,
        default=REPO_ROOT / "Cargo.lock",
        help="Cargo.lock to read when no CRATE arguments are given",
    )
    args = parser.parse_args()

    try:
        if args.crates:
            specs = [parse_spec(spec) for spec in args.crates]
        else:
            specs = specs_from_lockfile(args.lockfile)
            print(f"Checking {len(specs)} crates pinned in {args.lockfile}\n")
    except (ValueError, LookupError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    now = datetime.now(timezone.utc)
    too_fresh = []
    failed = []

    for name, wanted in specs:
        try:
            version = select_version(name, fetch_versions(name), wanted)
            published = parse_published(version)
        except LookupError as exc:
            print(f"ERROR   {name}: {exc}", file=sys.stderr)
            failed.append(name)
            continue

        age = now - published
        label = f"{name}@{version['num']}"
        yanked = " (YANKED)" if version["yanked"] else ""

        if age.total_seconds() < args.cooldown_hours * 3600:
            print(f"TOO NEW {label}{yanked} -- published {format_age(age)} ago")
            too_fresh.append(label)
        elif not args.crates:
            # The bulk lockfile scan is noisy; only surface what needs action.
            if yanked:
                print(f"WARN    {label} is yanked")
        else:
            print(f"OK      {label}{yanked} -- published {format_age(age)} ago")

    if failed:
        print(
            f"\n{len(failed)} crate(s) could not be checked: {', '.join(failed)}",
            file=sys.stderr,
        )
        return 2

    if too_fresh:
        print(
            f"\n{len(too_fresh)} version(s) inside the "
            f"{args.cooldown_hours:g}h cooldown: {', '.join(too_fresh)}"
        )
        print("Wait for the cooldown to elapse before adopting them.")
        return 1

    print(f"\nAll checked versions are older than {args.cooldown_hours:g}h.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
