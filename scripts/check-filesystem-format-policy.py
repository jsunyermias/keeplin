#!/usr/bin/env python3
"""Enforce the syntactic filesystem-format bump and release-latch policy."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Sequence


LIFECYCLE = "keeplin-core/src/storage/fs/lifecycle.rs"
LATCH = ".github/keeplin-release-boundary.json"
POLICY = "scripts/check-filesystem-format-policy.py"
VERSION_RE = re.compile(r"FORMAT_VERSION\s*:\s*u32\s*=\s*(\d+)\s*;")
ADR_RE = re.compile(r"\bADR[ -]?(\d{4})\b", re.IGNORECASE)
EXCEPTION_RE = re.compile(
    r"^- Filesystem-format-exception:\s*(\d+)\s*->\s*(\d+)\s*$",
    re.MULTILINE | re.IGNORECASE,
)


def _git(root: Path, *arguments: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", *arguments], cwd=root, capture_output=True, text=True, check=False
    )
    if check and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result.stdout


def _file_at(root: Path, revision: str, path: str) -> str | None:
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout if result.returncode == 0 else None


def _version(source: str | None) -> int | None:
    match = VERSION_RE.search(source or "")
    return int(match.group(1)) if match else None


def _function(source: str | None, name: str) -> str | None:
    if source is None:
        return None
    match = re.search(rf"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{name}\b", source)
    if not match:
        return None
    opening = source.find("{", match.start())
    if opening < 0:
        return None
    depth = 0
    for index in range(opening, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[match.start() : index + 1]
    return None


def _accepted_exception(
    root: Path, base: str, head: str, old_version: int, new_version: int
) -> bool:
    references = ADR_RE.findall(_git(root, "log", "--format=%B", f"{base}..{head}"))
    added_lines = _git(root, "diff", "--unified=0", base, head).splitlines()
    references.extend(
        match.group(1)
        for line in added_lines
        if line.startswith("+") and not line.startswith("+++")
        for match in ADR_RE.finditer(line)
    )
    candidates = _git(
        root, "ls-tree", "-r", "--name-only", base, "docs/adr", check=False
    ).splitlines()
    for number in set(references):
        path = next((item for item in candidates if f"docs/adr/{number}-" in item), None)
        text = _file_at(root, base, path) if path else None
        transitions = {
            (int(match.group(1)), int(match.group(2)))
            for match in EXCEPTION_RE.finditer(text or "")
        }
        if (
            re.search(r"^- status:\s*accepted\s*$", text or "", re.MULTILINE | re.IGNORECASE)
            and (old_version, new_version) in transitions
        ):
            return True
    return False


def _latch_is_immutable(root: Path, base: str, head: str) -> bool:
    revisions = _git(root, "rev-list", "--reverse", f"{base}..{head}").splitlines()
    for revision in revisions:
        current = _file_at(root, revision, LATCH)
        parents = _git(root, "show", "-s", "--format=%P", revision).split()
        for parent in parents:
            previous = _file_at(root, parent, LATCH)
            if previous is not None and current != previous:
                return False
    return True


def _policy_is_immutable(root: Path, base: str, head: str) -> bool:
    revisions = _git(root, "rev-list", "--reverse", f"{base}..{head}").splitlines()
    for revision in revisions:
        current = _file_at(root, revision, POLICY)
        parents = _git(root, "show", "-s", "--format=%P", revision).split()
        for parent in parents:
            previous = _file_at(root, parent, POLICY)
            if previous is not None and current != previous:
                return False
    return True


def evaluate(root: Path, base: str, head: str) -> list[str]:
    failures: list[str] = []
    if not _latch_is_immutable(root, base, head):
        failures.append(
            f"{LATCH} is an immutable release-boundary latch once committed; it may not be modified or deleted"
        )
    if not _policy_is_immutable(root, base, head):
        failures.append(
            f"{POLICY} is immutable once committed; it may not be modified or deleted"
        )

    old_source = _file_at(root, base, LIFECYCLE)
    new_source = _file_at(root, head, LIFECYCLE)
    old_version = _version(old_source)
    new_version = _version(new_source)
    if old_version is None:
        failures.append(f"cannot locate a decimal FORMAT_VERSION declaration in {base}:{LIFECYCLE}")
        return failures
    if new_version is None:
        failures.append(f"cannot locate a decimal FORMAT_VERSION declaration in {head}:{LIFECYCLE}")
        return failures
    if old_version == new_version:
        return failures

    exception = _accepted_exception(root, base, head, old_version, new_version)
    old_dispatch = _function(old_source, "apply_format_migration")
    new_dispatch = _function(new_source, "apply_format_migration")
    dispatch_changed = new_dispatch is not None and new_dispatch != old_dispatch
    diff = _git(root, "diff", "--unified=0", base, head, "--", "*.rs")
    transition = re.compile(
        rf"^\+\s*(?:async\s+)?fn\s+[a-z0-9_]*preserv[a-z0-9_]*v{old_version}_to_v{new_version}[a-z0-9_]*\s*\(",
        re.MULTILINE,
    )
    reverse_transition = re.compile(
        rf"^\+\s*(?:async\s+)?fn\s+[a-z0-9_]*v{old_version}_to_v{new_version}[a-z0-9_]*preserv[a-z0-9_]*\s*\(",
        re.MULTILINE,
    )
    preservation_test_added = bool(transition.search(diff) or reverse_transition.search(diff))
    if not exception and not (dispatch_changed and preservation_test_added):
        failures.append(
            f"FORMAT_VERSION changed from {old_version} to {new_version}. The same change must modify or add "
            "apply_format_migration and add a data-preservation test whose name identifies "
            f"v{old_version}_to_v{new_version}, or cite a separately accepted ADR authorizing a bounded "
            "exception. This gate checks syntax only; a green result cannot judge whether the migration "
            "or test substantively preserves data, which remains a review obligation."
        )
    return failures


def main(argv: Sequence[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    if len(arguments) not in (0, 2):
        print("usage: check-filesystem-format-policy.py [base head]", file=sys.stderr)
        return 2
    root = Path(__file__).resolve().parents[1]
    if arguments:
        base, head = arguments
    else:
        base = os.environ.get("FORMAT_POLICY_BASE", "")
        head = os.environ.get("FORMAT_POLICY_HEAD", "HEAD")
        if not base:
            print("FORMAT POLICY: FORMAT_POLICY_BASE is required", file=sys.stderr)
            return 2
    try:
        failures = evaluate(root, base, head)
    except RuntimeError as error:
        print(f"FORMAT POLICY: cannot inspect {base}..{head}: {error}", file=sys.stderr)
        return 2
    if failures:
        for failure in failures:
            print(f"FORMAT POLICY: {failure}")
        return 1
    print("filesystem-format policy check OK (syntactic evidence only; substantive review remains required)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
