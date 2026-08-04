#!/usr/bin/env python3
"""Require one raw bounded-history anchor line in three manually enrolled surfaces.

This checker does not parse Markdown or enforce rendered visibility. An exact anchor line inside
an HTML comment or fenced code block satisfies the deliberately narrow enrolment contract.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence


ANCHOR = (
    "Bounded-history anchor: terminal truncation can erase reification history and "
    "enable advisory convergence."
)
SURFACES = ("AGENTS.md", ".github/scripts/README.md", "docs/review-stalls.md")


def _carries_anchor(path: Path) -> bool:
    return ANCHOR in path.read_text(encoding="utf-8").splitlines()


def main(argv: Sequence[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    if len(arguments) > 1:
        print("usage: check-bounded-history.py [root]", file=sys.stderr)
        return 2
    root = Path(arguments[0]).resolve() if arguments else Path(__file__).resolve().parents[1]
    failed = False
    for surface in SURFACES:
        path = root / surface
        if not path.is_file():
            print(f"BOUNDED HISTORY: {surface} is missing from {root}")
            failed = True
            continue
        try:
            carries_anchor = _carries_anchor(path)
        except (OSError, UnicodeError) as error:
            print(f"BOUNDED HISTORY: cannot read {surface} from {root}: {error}")
            failed = True
            continue
        if not carries_anchor:
            print(f"BOUNDED HISTORY: {surface} does not carry the exact bounded-history anchor")
            failed = True
    if failed:
        print("\nThe required standalone anchor is:")
        print(f"  {ANCHOR}")
        return 1
    print(f"bounded-history check OK: {len(SURFACES)} surfaces carry the exact anchor")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
