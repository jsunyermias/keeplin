#!/usr/bin/env python3
"""Require the bounded-history sentence in reader-visible prose.

This standard-library checker implements a declared Markdown subset. It recognizes backtick
and tilde fences at the document margin or inside one blockquote level, including closing
fences at least as long as their opener; blank-line-delimited code indented by four spaces or
one tab; HTML comments outside code; same-line backtick code spans; and simple single-line
link-reference definitions. The fenced and indented state machine is shared with
``scripts/companion_tool.py`` rather than duplicated here.

It does not parse blockquotes deeper than one level; list-item continuation indentation or
code blocks nested in list items; raw inline or block HTML other than ``<!-- -->`` comments;
reference definitions split across lines; multiline inline-code spans; or inline link/image
destinations and titles. A canonical sentence hidden in one of those constructs is neither
guaranteed to count nor guaranteed to be ignored. This is not a CommonMark conformance claim.

The three policy surfaces are manually enrolled below. Enrolment is not inferred from prose,
so a new document is outside the check until a maintainer adds it explicitly.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

from companion_tool import reader_visible_markdown


CANONICAL = (
    "Terminal truncation is not detected: it can erase the record that established "
    "reification, after which the shorter authentic prefix may converge with that finding "
    "advisory."
)
SURFACES = ("AGENTS.md", ".github/scripts/README.md", "docs/review-stalls.md")


def _contains_canonical_sentence(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    prose = " ".join(reader_visible_markdown(text).split())
    return CANONICAL in prose


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
            contains_sentence = _contains_canonical_sentence(path)
        except (OSError, UnicodeError) as error:
            print(f"BOUNDED HISTORY: cannot read {surface} from {root}: {error}")
            failed = True
            continue
        if not contains_sentence:
            print(
                f"BOUNDED HISTORY: {surface} does not carry the canonical "
                "bounded-history sentence verbatim"
            )
            failed = True
    if failed:
        print("\nThe canonical sentence is:")
        print(f"  {CANONICAL}")
        return 1
    print(f"bounded-history check OK: {len(SURFACES)} surfaces carry the canonical sentence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
