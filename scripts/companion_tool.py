#!/usr/bin/env python3
"""Deterministic verifier, synchronizer and context-pack builder for companions."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


RUST_MARKER_RE = re.compile(r"^(?P<indent>[ \t]*)// md:(?P<name>.+?)[ \t]*$")
SHELL_MARKER_RE = re.compile(r"^(?P<indent>[ \t]*)# md:(?P<name>.+?)[ \t]*$")
SQL_MARKER_RE = re.compile(r"^[ \t]*-- md:(?P<name>.+?)[ \t]*$", re.MULTILINE)
SHELL_SHEBANG_RE = re.compile(r"^#![^\r\n]*(?:[/\s])(?:ba|da|k|z)?sh(?:\s|$)")
CLOSE_FENCE_RE = re.compile(r"^```[ \t]*(?=\r?$)", re.MULTILINE)
CONTRACT_ID_RE = re.compile(
    r"^`(?P<id>[a-z0-9]+(?:[._:/-][a-z0-9]+)*)`(?:\s+(?:—|-)\s+.+)?$"
)
RISK_RULES = (
    ("security", ("auth", "password", "secret", "jwt", "encrypt", "cipher", "crypto", "permission")),
    ("migration", ("migration", "schema_version", "format_version", "migrate")),
    ("protocol", ("protocol", "proto", "wire", "handshake", "sync", "collab", "websocket")),
    ("persistence", ("storage", "database", "repository", "backend", "postgres", "sqlite", "sql")),
)
CONTRACT_TERMS = ("contract", "must", "expects", "wire", "protocol", "format", "compatib", "security")
IGNORED_DIRS = {".git", "target", "graphify-out"}
CONTRACT_REGISTRY = "docs/cross-repo-contracts.txt"
REVIEW_DEBT_REGISTRY = "docs/review-debt.md"
REVIEW_DEBT_SECTIONS = {
    "Open": ("Merged", "Change", "Implementer", "What went unreviewed", "Follow-up"),
    "Cleared": ("Merged", "Change", "Reviewer", "Review"),
}
REVIEW_DEBT_EMPTY_CELL = "—"
REVIEW_DEBT_UNANSWERED = {"", "—", "-", "tbd", "pendiente"}
PULL_REQUEST_RE = re.compile(
    r"https://github\.com/jsunyermias/(?P<repo>keeplin|keeplin-srv)/pull/(?P<number>\d+)\b"
)
WEB_LINK_RE = re.compile(
    r"\bhttps?://[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?(?::\d+)?"
    r"(?:/[^\s<>()]*)?(?=[\s<>()]|$)"
)
SCHEMA_VERSION = 2


class CompanionError(Exception):
    pass


@dataclass(frozen=True)
class SourceBlock:
    name: str
    marker_line: str
    start_line: int
    code: str | None

    @property
    def is_container(self) -> bool:
        return self.code is None


@dataclass(frozen=True)
class Fence:
    start: int
    content_start: int
    content_end: int
    end: int
    content: str
    marker_name: str | None
    newline: str


def _read(path: Path) -> tuple[str, str]:
    raw = path.read_bytes().decode("utf-8")
    newline = "\r\n" if raw.count("\r\n") > raw.count("\n") / 2 else "\n"
    return raw.replace("\r\n", "\n"), newline


def _write(path: Path, text: str, newline: str = "\n") -> None:
    path.write_bytes(text.replace("\n", newline).encode("utf-8"))


def source_kind(path: Path) -> str | None:
    """Return the fidelity grammar for a supported source, or None."""
    if path.suffix == ".rs":
        return "rust"
    if path.suffix == ".sh":
        return "shell"
    if path.suffix == ".sql":
        return "sql"
    if not path.suffix and path.is_file():
        try:
            first = path.read_bytes().splitlines()[0].decode("utf-8")
        except (IndexError, OSError, UnicodeError):
            return None
        if SHELL_SHEBANG_RE.match(first):
            return "shell"
    return None


def companion_for_source(path: Path) -> Path:
    kind = source_kind(path)
    if kind in {"rust", "sql"}:
        return path.with_suffix(".md")
    if kind == "shell":
        return Path(str(path) + ".md")
    raise CompanionError(f"unsupported companion source: {path}")


def iter_sources(root: Path) -> list[Path]:
    return sorted(
        (
            p
            for p in root.rglob("*")
            if p.is_file()
            and not any(part in IGNORED_DIRS for part in p.relative_to(root).parts)
            and source_kind(p) is not None
        ),
        key=lambda p: p.relative_to(root).as_posix(),
    )


def _mask_rust(text: str) -> str:
    """Mask strings/comments while preserving byte positions and newlines."""
    out = list(text)
    i = 0
    n = len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
            continue
        if text.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue
        raw = re.match(r"(?:br|rb|r|b)?(#+)?\"", text[i:])
        if raw:
            prefix = raw.group(0)
            hashes = raw.group(1) or ""
            j = i + len(prefix)
            closing = '"' + hashes
            if hashes or prefix.startswith("r") or prefix.startswith("br") or prefix.startswith("rb"):
                end = text.find(closing, j)
                j = n if end < 0 else end + len(closing)
            else:
                escaped = False
                while j < n:
                    ch = text[j]
                    j += 1
                    if ch == '"' and not escaped:
                        break
                    escaped = ch == "\\" and not escaped
                    if ch != "\\":
                        escaped = False
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue
        if text[i] == "'":
            char = re.match(r"'(?:\\.|[^'\\\n])'", text[i:])
            if char:
                j = i + len(char.group(0))
                for k in range(i, j):
                    out[k] = " "
                i = j
                continue
        i += 1
    return "".join(out)


def _item_kind(masked: str) -> str:
    lines = masked.splitlines()
    i = 0
    bracket_depth = 0
    while i < len(lines):
        stripped = lines[i].strip()
        if not stripped:
            i += 1
            continue
        if stripped.startswith("#[") or bracket_depth:
            bracket_depth += stripped.count("[") - stripped.count("]")
            i += 1
            if bracket_depth <= 0:
                bracket_depth = 0
            continue
        head = stripped
        break
    else:
        return "unknown"
    head = re.sub(r"^(?:pub(?:\([^)]*\))?\s+)?", "", head)
    head = re.sub(r"^(?:unsafe\s+|async\s+|default\s+)*", "", head)
    match = re.match(r"(use|const|static|type|fn|struct|enum|trait|impl|mod|extern|union|macro_rules)\b", head)
    return match.group(1) if match else "unknown"


def _extract_item(lines: list[str], marker_index: int, source: Path) -> str:
    body_lines = lines[marker_index + 1 :]
    body = "\n".join(body_lines)
    masked = _mask_rust(body)
    kind = _item_kind(masked)
    semicolon_item = kind in {"use", "const", "static", "type"}
    curly = square = paren = 0
    saw_curly = False
    line = 0
    for pos, ch in enumerate(masked):
        if ch == "\n":
            line += 1
            continue
        if ch == "{":
            curly += 1
            saw_curly = True
        elif ch == "}":
            curly -= 1
            if curly < 0:
                raise CompanionError(f"unbalanced closing brace after marker at {source}:{marker_index + 1}")
            if saw_curly and curly == square == paren == 0 and not semicolon_item:
                end = marker_index + 1 + line
                return "\n".join(lines[marker_index : end + 1])
        elif ch == "[":
            square += 1
        elif ch == "]":
            square -= 1
        elif ch == "(":
            paren += 1
        elif ch == ")":
            paren -= 1
        elif ch == ";" and curly == square == paren == 0:
            end = marker_index + 1 + line
            return "\n".join(lines[marker_index : end + 1])
    raise CompanionError(
        f"cannot find the end of block '{lines[marker_index].strip()}' at {source}:{marker_index + 1}"
    )


def _container_preamble_error(
    lines: list[str], marker_index: int, next_index: int, name: str, source: Path
) -> str | None:
    """Reject real code hidden between a container marker and its first child."""
    preamble = "\n".join(lines[marker_index + 1 : next_index])
    masked = _mask_rust(preamble)
    length = len(masked)

    def skip_space(position: int) -> int:
        while position < length and masked[position].isspace():
            position += 1
        return position

    def skip_attribute(position: int) -> int | None:
        match = re.match(r"#!?\[", masked[position:])
        if not match:
            return None
        depth = 0
        for cursor in range(position + match.end() - 1, length):
            if masked[cursor] == "[":
                depth += 1
            elif masked[cursor] == "]":
                depth -= 1
                if depth == 0:
                    return cursor + 1
        return None

    position = skip_space(0)
    declaration_start = position
    while position < length:
        attribute_end = skip_attribute(position)
        if attribute_end is None:
            break
        position = skip_space(attribute_end)

    declaration = re.match(
        r"(?:(?:pub(?:\s*\([^)]*\))?)\s+)?(?:unsafe\s+|async\s+|default\s+)*(?:impl|mod|trait)\b",
        masked[position:],
    )
    if declaration:
        body_open = masked.find("{", position + declaration.end())
        terminator = masked.find(";", position + declaration.end())
        if body_open < 0 or (terminator >= 0 and terminator < body_open):
            position = declaration_start
        else:
            position = body_open + 1
    else:
        # RULE 6 admits only impl/mod/trait as containers; a type definition is a leaf
        # (RULE 5). Saying so beats reporting the type's own declaration as UNCOVERED.
        type_item = re.match(
            r"(?:(?:pub(?:\s*\([^)]*\))?)\s+)?(?:enum|struct|union)\b", masked[position:]
        )
        if type_item:
            line_number = marker_index + 2 + masked[:position].count("\n")
            return (
                f"CONTAINER '// md:{name}' must be an impl, mod or trait (RULE 6); "
                f"{source}:{line_number} declares a type, which is a leaf block (RULE 5)"
            )
        # Documentation-only grouping containers can have an empty preamble.
        position = declaration_start

    while True:
        position = skip_space(position)
        if position >= length:
            return None
        attribute_end = skip_attribute(position)
        if attribute_end is not None:
            position = attribute_end
            continue
        if masked[position] in "{}":
            position += 1
            continue
        line_number = marker_index + 2 + masked[:position].count("\n")
        snippet = lines[line_number - 1].strip()
        return (
            f"UNCOVERED code between container '// md:{name}' and its first child marker "
            f"at {source}:{line_number}: {snippet}"
        )


def _leading_scaffolding_error(lines: list[str], first_marker: int, source: Path) -> str | None:
    """Allow only non-item crate scaffolding before the first marker."""
    prefix = "\n".join(lines[:first_marker])
    masked = _mask_rust(prefix)
    length = len(masked)
    position = 0

    if masked.startswith("\ufeff"):
        position = 1

    if masked.startswith("#!", position) and not masked.startswith("#![", position):
        newline = masked.find("\n", position)
        position = length if newline < 0 else newline + 1

    def skip_space(cursor: int) -> int:
        while cursor < length and masked[cursor].isspace():
            cursor += 1
        return cursor

    position = skip_space(position)
    while masked.startswith("#![", position):
        depth = 0
        end = None
        for cursor in range(position + 2, length):
            if masked[cursor] == "[":
                depth += 1
            elif masked[cursor] == "]":
                depth -= 1
                if depth == 0:
                    end = cursor + 1
                    break
        if end is None:
            break
        position = skip_space(end)

    if position >= length:
        return None
    line_number = masked[:position].count("\n") + 1
    snippet = lines[line_number - 1].strip()
    return (
        f"UNCOVERED code before first '// md:' marker at "
        f"{source}:{line_number}: {snippet}"
    )


def _parse_rust_source(path: Path) -> list[SourceBlock]:
    text, _ = _read(path)
    lines = text.splitlines()
    found: list[tuple[int, str, str]] = []
    for index, line in enumerate(lines):
        match = RUST_MARKER_RE.match(line)
        if match:
            found.append((index, match.group("name").strip(), line))
    first_marker = found[0][0] if found else len(lines)
    uncovered = _leading_scaffolding_error(lines, first_marker, path)
    if uncovered:
        raise CompanionError(uncovered)
    names = [name for _, name, _ in found]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise CompanionError(f"duplicate source marker(s) in {path}: {', '.join(duplicates)}")
    blocks: list[SourceBlock] = []
    for ordinal, (index, name, marker_line) in enumerate(found):
        next_index = found[ordinal + 1][0] if ordinal + 1 < len(found) else len(lines)
        next_name = found[ordinal + 1][1] if ordinal + 1 < len(found) else None
        if next_name is not None and next_name.startswith(name + " >"):
            uncovered = _container_preamble_error(lines, index, next_index, name, path)
            if uncovered:
                raise CompanionError(uncovered)
            code = None
        elif name == "Overview":
            end = next_index
            while end > index + 1 and not lines[end - 1].strip():
                end -= 1
            code = "\n".join(lines[index:end])
        else:
            # A block can never cross the next marker. Bounding the scanner keeps
            # verification linear even for large modules with many small blocks.
            first_item = _extract_item(lines[:next_index], index, path)
            first_item_end = index + len(first_item.splitlines())
            end = next_index
            while end > first_item_end and not lines[end - 1].strip():
                end -= 1
            # Closing braces after the first item belong to a surrounding impl/mod
            # container. Unmarked associated helpers before them remain part of this
            # leaf block, matching RULE 5's inseparable-helper grouping.
            marker_indent = len(marker_line) - len(marker_line.lstrip(" \t"))
            while (
                end > first_item_end
                and lines[end - 1].strip() == "}"
                and len(lines[end - 1]) - len(lines[end - 1].lstrip(" \t")) < marker_indent
            ):
                end -= 1
                while end > first_item_end and not lines[end - 1].strip():
                    end -= 1
            code = "\n".join(lines[index:end])
        blocks.append(SourceBlock(name=name, marker_line=marker_line, start_line=index + 1, code=code))
    return blocks


def _leading_shell_scaffolding_error(
    lines: list[str], first_marker: int, source: Path
) -> str | None:
    """Allow only BOM, a first-line shebang and blanks before the first shell marker."""
    prefix = lines[:first_marker]
    for index, line in enumerate(prefix):
        candidate = line
        if index == 0:
            candidate = candidate.lstrip("\ufeff")
            if candidate.startswith("#!"):
                continue
        if candidate.strip():
            return (
                f"UNCOVERED code before first '# md:' marker at "
                f"{source}:{index + 1}: {line.strip()}"
            )
    return None


def _parse_shell_source(path: Path) -> list[SourceBlock]:
    text, _ = _read(path)
    lines = text.splitlines()
    found: list[tuple[int, str, str]] = []
    for index, line in enumerate(lines):
        match = SHELL_MARKER_RE.match(line)
        if match:
            found.append((index, match.group("name").strip(), line))
    first_marker = found[0][0] if found else len(lines)
    uncovered = _leading_shell_scaffolding_error(lines, first_marker, path)
    if uncovered:
        raise CompanionError(uncovered)
    names = [name for _, name, _ in found]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise CompanionError(f"duplicate source marker(s) in {path}: {', '.join(duplicates)}")
    blocks = []
    for ordinal, (index, name, marker_line) in enumerate(found):
        end = found[ordinal + 1][0] if ordinal + 1 < len(found) else len(lines)
        while end > index + 1 and not lines[end - 1].strip():
            end -= 1
        blocks.append(
            SourceBlock(
                name=name,
                marker_line=marker_line,
                start_line=index + 1,
                code="\n".join(lines[index:end]),
            )
        )
    return blocks


def parse_source(path: Path) -> list[SourceBlock]:
    kind = source_kind(path)
    if kind == "rust":
        return _parse_rust_source(path)
    if kind == "shell":
        return _parse_shell_source(path)
    raise CompanionError(f"marked-block parsing is unsupported for {path}")


def parse_fences(
    text: str, language: str = "rust", marker_re: re.Pattern[str] | None = RUST_MARKER_RE
) -> list[Fence]:
    fences: list[Fence] = []
    opening_re = re.compile(
        rf"^```{re.escape(language)}[ \t]*(?P<newline>\r\n|\n)", re.MULTILINE
    )
    for opening in opening_re.finditer(text):
        content_start = opening.end()
        closing = CLOSE_FENCE_RE.search(text, content_start)
        if closing is None:
            raise CompanionError(f"unclosed {language} fence at character {opening.start()}")
        content_end = closing.start()
        if text[content_start:content_end].endswith("\r\n"):
            content_end -= 2
        elif content_end > content_start and text[content_end - 1] == "\n":
            content_end -= 1
        content = text[content_start:content_end].replace("\r\n", "\n")
        first = content.split("\n", 1)[0] if content else ""
        marker = marker_re.match(first) if marker_re else None
        fences.append(
            Fence(
                start=opening.start(),
                content_start=content_start,
                content_end=content_end,
                end=closing.end(),
                content=content,
                marker_name=marker.group("name").strip() if marker else None,
                newline=opening.group("newline"),
            )
        )
    return fences


def _verify_marked_pair(source: Path, companion: Path, kind: str) -> list[str]:
    errors: list[str] = []
    language = "rust" if kind == "rust" else "bash"
    marker_re = RUST_MARKER_RE if kind == "rust" else SHELL_MARKER_RE
    marker_prefix = "// md:" if kind == "rust" else "# md:"
    try:
        blocks = parse_source(source)
    except CompanionError as exc:
        return [str(exc)]
    text = companion.read_bytes().decode("utf-8")
    try:
        fences = parse_fences(text, language, marker_re)
    except CompanionError as exc:
        return [f"{companion}: {exc}"]
    if kind == "shell":
        # Shell companions may contain illustrative bash snippets. Only a
        # fence whose first line is a marker participates in fidelity checks.
        fences = [fence for fence in fences if fence.marker_name is not None]
    by_name = {block.name: block for block in blocks}
    fence_names = [f.marker_name for f in fences if f.marker_name]
    duplicate_fences = sorted({name for name in fence_names if fence_names.count(name) > 1})
    for name in duplicate_fences:
        errors.append(f"DUPLICATE fence for '{marker_prefix}{name}' in {companion}")
    for fence in fences:
        if fence.marker_name is None:
            errors.append(
                f"ORPHAN {language} fence without a leading '{marker_prefix}' marker in {companion}"
            )
            continue
        block = by_name.get(fence.marker_name)
        if block is None:
            errors.append(
                f"ORPHAN fence '{marker_prefix}{fence.marker_name}' has no source block in {source}"
            )
        elif block.is_container:
            errors.append(
                f"CONTAINER '{marker_prefix}{fence.marker_name}' must not have a {language} fence in {companion}"
            )
        elif fence.content != block.code:
            errors.append(
                f"STALE/TRUNCATED fence '{marker_prefix}{fence.marker_name}' in {companion}; "
                f"source block starts at {source}:{block.start_line}"
            )
    for block in blocks:
        count = fence_names.count(block.name)
        if not block.is_container and count == 0:
            errors.append(f"MISSING fence for '{marker_prefix}{block.name}' in {companion}")
        if block.is_container and count == 0 and f"{marker_prefix}{block.name}" not in text:
            errors.append(f"MISSING container section for '{marker_prefix}{block.name}' in {companion}")
    expected_order = [block.name for block in blocks if not block.is_container]
    actual_order = [name for name in fence_names if name in by_name and not by_name[name].is_container]
    if actual_order != expected_order and not duplicate_fences:
        errors.append(
            f"REORDERED {language} fences in {companion}; fences must follow source leaf order"
        )
    return errors


def _verify_sql_pair(source: Path, companion: Path) -> list[str]:
    text, _ = _read(source)
    marker = SQL_MARKER_RE.search(text)
    if marker:
        line = text[: marker.start()].count("\n") + 1
        return [f"FORBIDDEN '-- md:' marker in immutable SQL migration {source}:{line}"]
    try:
        fences = parse_fences(companion.read_bytes().decode("utf-8"), "sql", None)
    except CompanionError as exc:
        return [f"{companion}: {exc}"]
    if not fences:
        return [f"MISSING single complete sql fence in {companion} for {source}"]
    if len(fences) != 1:
        return [f"DUPLICATE sql fences in {companion}; exactly one complete-file fence is required"]
    if fences[0].content != text:
        return [f"STALE/TRUNCATED complete sql fence in {companion}; expected verbatim {source}"]
    return []


def verify_pair(source: Path, companion: Path) -> list[str]:
    kind = source_kind(source)
    if kind in {"rust", "shell"}:
        return _verify_marked_pair(source, companion, kind)
    if kind == "sql":
        return _verify_sql_pair(source, companion)
    return [f"unsupported companion source: {source}"]


def _resolve_sources(root: Path, requested: Sequence[str]) -> list[Path]:
    if not requested:
        return iter_sources(root)
    sources: set[Path] = set()
    for raw in requested:
        path = (root / raw).resolve() if not Path(raw).is_absolute() else Path(raw).resolve()
        if path.suffix == ".md" and path.is_file():
            candidates = [Path(str(path)[:-3]), path.with_suffix(".rs"), path.with_suffix(".sql")]
            path = next(
                (
                    candidate
                    for candidate in candidates
                    if candidate.is_file()
                    and source_kind(candidate) is not None
                    and companion_for_source(candidate) == path
                ),
                path,
            )
        if path.is_dir():
            sources.update(
                p
                for p in path.rglob("*")
                if p.is_file()
                and not any(part in IGNORED_DIRS for part in p.parts)
                and source_kind(p) is not None
            )
        elif path.is_file() and source_kind(path) is not None:
            sources.add(path)
        else:
            raise CompanionError(f"not a supported source, companion, or directory: {raw}")
    return sorted(sources, key=lambda p: p.relative_to(root).as_posix())


def sync(root: Path, requested: Sequence[str], check: bool) -> tuple[int, int, list[str]]:
    changed = 0
    checked = 0
    errors: list[str] = []
    for source in _resolve_sources(root, requested):
        companion = companion_for_source(source)
        if not companion.is_file():
            errors.append(f"MISSING companion doc: {companion}")
            continue
        checked += 1
        kind = source_kind(source)
        language = "rust" if kind == "rust" else "bash" if kind == "shell" else "sql"
        marker_re = RUST_MARKER_RE if kind == "rust" else SHELL_MARKER_RE if kind == "shell" else None
        try:
            blocks = (
                {block.name: block for block in parse_source(source)}
                if kind in {"rust", "shell"}
                else {}
            )
            text = companion.read_bytes().decode("utf-8")
            fences = parse_fences(text, language, marker_re)
        except CompanionError as exc:
            errors.append(str(exc))
            continue
        if kind == "shell":
            fences = [fence for fence in fences if fence.marker_name is not None]
        replacements: list[tuple[int, int, str]] = []
        if kind == "sql":
            expected, _ = _read(source)
            if len(fences) == 1 and fences[0].content != expected:
                replacements.append(
                    (
                        fences[0].content_start,
                        fences[0].content_end,
                        expected.replace("\n", fences[0].newline),
                    )
                )
        else:
            for fence in fences:
                if fence.marker_name and fence.marker_name in blocks and blocks[fence.marker_name].code is not None:
                    expected = blocks[fence.marker_name].code
                    if fence.content != expected:
                        replacement = (expected or "").replace("\n", fence.newline)
                        replacements.append((fence.content_start, fence.content_end, replacement))
        pair_errors = verify_pair(source, companion)
        non_stale = [e for e in pair_errors if not e.startswith("STALE/TRUNCATED")]
        if non_stale:
            errors.extend(non_stale)
            continue
        if replacements:
            changed += 1
            if check:
                errors.extend(e for e in pair_errors if e.startswith("STALE/TRUNCATED"))
            else:
                for start, end, replacement in reversed(replacements):
                    text = text[:start] + replacement + text[end:]
                companion.write_bytes(text.encode("utf-8"))
    return checked, changed, errors


def _sections(text: str, heading: str) -> list[str]:
    pattern = re.compile(rf"^\*\*{re.escape(heading)}\*\*.*?$", re.MULTILINE)
    sections = []
    for match in pattern.finditer(text):
        rest = text[match.end() :]
        boundary = re.search(r"^(?:\*\*|#{1,6}\s|---\s*$)", rest, re.MULTILINE)
        sections.append(rest[: boundary.start()] if boundary else rest)
    return sections


def _section(text: str, heading: str) -> str:
    sections = _sections(text, heading)
    return sections[0] if sections else ""


def _bullets(section: str) -> list[str]:
    values: list[str] = []
    current: list[str] = []
    for line in section.splitlines():
        if line.startswith("- "):
            if current:
                values.append(re.sub(r"\s+", " ", " ".join(current)).strip())
            current = [line[2:].strip()]
        elif current and line.strip():
            current.append(line.strip())
        elif current:
            values.append(re.sub(r"\s+", " ", " ".join(current)).strip())
            current = []
    if current:
        values.append(re.sub(r"\s+", " ", " ".join(current)).strip())
    return values


def _cross_repo_contracts(text: str) -> list[str]:
    contracts = set()
    for section in _sections(text, "Cross-repo contracts"):
        for bullet in _bullets(section):
            match = CONTRACT_ID_RE.fullmatch(bullet)
            if match:
                contracts.add(match.group("id"))
    return sorted(contracts)


def _manifest_invariants(text: str) -> list[dict[str, str]]:
    invariants: list[dict[str, str]] = []
    seen: set[tuple[str, str]] = set()

    def append(origin: str, value: str, basis: str) -> None:
        key = (origin, value)
        if value and key not in seen:
            seen.add(key)
            invariants.append({"origin": origin, "value": value, "basis": basis})

    for section in _sections(text, "Invariants"):
        for bullet in _bullets(section):
            origin = "EXTRACTED" if "(EXTRACTED" in bullet else "INFERRED"
            append(origin, bullet, "authored-invariant")
    for section in _sections(text, "Dependencies"):
        for bullet in _bullets(section):
            expectation = re.search(r"(?:^|;\s*)expects:\s*(.+)$", bullet, re.IGNORECASE)
            if expectation:
                append("INFERRED", expectation.group(1).strip(), "dependency-expectation")
    return invariants


def _path_from_bullet(value: str, source: Path, root: Path) -> str | None:
    match = re.search(r"`([^`]+)`", value)
    if not match:
        return None
    raw = match.group(1).replace("\\", "/")
    candidates = [root / raw, source.parent / raw]
    if raw.endswith(".md"):
        candidates.append(root / (raw[:-3] + ".rs"))
    for candidate in candidates:
        if candidate.is_file() and candidate.suffix == ".rs":
            return candidate.relative_to(root).as_posix()
    return None


def _risk(path: str, text: str) -> dict[str, object]:
    title = next((line for line in text.splitlines() if line.startswith("# ")), "")
    # Repeated context intentionally mentions global risks in many companions; using the
    # whole document would classify almost everything as security-sensitive. Paths, titles
    # and authored invariants are the bounded signals for this explanatory inference.
    haystack = (path + "\n" + title + "\n" + _section(text, "Invariants")).lower()
    for level, terms in RISK_RULES:
        hits = sorted({term for term in terms if term in haystack})
        if hits:
            return {"origin": "INFERRED", "value": level, "basis": hits[:5]}
    return {"origin": "INFERRED", "value": "normal", "basis": ["no elevated-risk term matched"]}


def _package_test_command(source: Path, root: Path) -> str:
    current = source.parent
    while current != root.parent:
        cargo = current / "Cargo.toml"
        if cargo.is_file():
            cargo_text, _ = _read(cargo)
            package = re.search(r"(?ms)^\[package\].*?^name\s*=\s*\"([^\"]+)\"", cargo_text)
            if package:
                return f"cargo test -p {package.group(1)}"
        if current == root:
            break
        current = current.parent
    return "cargo test --workspace"


def build_manifest(root: Path) -> dict[str, object]:
    draft: list[dict[str, object]] = []
    for source in iter_sources(root):
        companion = companion_for_source(source)
        if not companion.is_file():
            continue
        source_rel = source.relative_to(root).as_posix()
        companion_rel = companion.relative_to(root).as_posix()
        text, _ = _read(companion)
        blocks = parse_source(source) if source_kind(source) in {"rust", "shell"} else []
        deps_section = _section(text, "Direct dependencies")
        dependents_section = _section(text, "Direct dependents")
        dependencies = []
        for bullet in _bullets(deps_section):
            dep = _path_from_bullet(bullet, source, root)
            if dep:
                dependencies.append(
                    {
                        "path": dep,
                        "origin": "EXTRACTED" if "(EXTRACTED" in bullet else "INFERRED",
                        "required": any(term in bullet.lower() for term in CONTRACT_TERMS),
                    }
                )
        dependents = []
        for bullet in _bullets(dependents_section):
            dependent = _path_from_bullet(bullet, source, root)
            if dependent:
                dependents.append(
                    {
                        "path": dependent,
                        "origin": "EXTRACTED" if "(EXTRACTED" in bullet else "INFERRED",
                    }
                )
        invariants = _manifest_invariants(text)
        contracts = _cross_repo_contracts(text)
        title = next((line[2:].strip() for line in text.splitlines() if line.startswith("# ")), companion_rel)
        draft.append(
            {
                "source": {"origin": "EXTRACTED", "value": source_rel},
                "companion": {"origin": "EXTRACTED", "value": companion_rel},
                "purpose": {"origin": "EXTRACTED", "value": title},
                "markers": {"origin": "EXTRACTED", "value": [block.name for block in blocks]},
                "invariants": {"origin": "INFERRED", "value": invariants},
                "cross_repo_contracts": {"origin": "INFERRED", "value": contracts},
                "direct_dependencies": dependencies,
                "direct_dependents": dependents,
                "tests": {"origin": "INFERRED", "value": [_package_test_command(source, root)]},
                "risk": _risk(source_rel, text),
            }
        )
    risks = {entry["source"]["value"]: entry["risk"]["value"] for entry in draft}  # type: ignore[index]
    for entry in draft:
        high_risk = []
        for dependent in entry["direct_dependents"]:  # type: ignore[assignment]
            if risks.get(dependent["path"], "normal") != "normal":
                high_risk.append(dependent)
        entry["high_risk_dependents"] = high_risk
    return {
        "schema_version": SCHEMA_VERSION,
        "generator": "scripts/companion_tool.py",
        "entries": draft,
    }


def manifest_text(root: Path) -> str:
    return _manifest_text(build_manifest(root))


def _manifest_text(manifest: dict[str, object]) -> str:
    return json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def _pinned_contracts(root: Path) -> tuple[list[str] | None, str | None]:
    """Read the checked-in cross-repo contract registry shared by both repositories."""
    registry = root / CONTRACT_REGISTRY
    if not registry.is_file():
        return None, f"MISSING cross-repo contract registry: {CONTRACT_REGISTRY}"
    text, _ = _read(registry)
    pinned = []
    for line in text.splitlines():
        entry = line.split("#", 1)[0].strip()
        if entry:
            pinned.append(entry)
    return sorted(set(pinned)), None


def verify_contract_registry(root: Path, manifest: dict[str, object]) -> list[str]:
    """Fail when the emitted contract identifiers drift from the pinned registry.

    Each repository builds its manifest alone, so nothing else can notice one side
    adding, renaming or silently losing a shared identifier.
    """
    pinned, error = _pinned_contracts(root)
    if pinned is None:
        return [error or ""]
    emitted = set()
    for entry in manifest["entries"]:  # type: ignore[index]
        field = entry.get("cross_repo_contracts") or {}  # type: ignore[union-attr]
        emitted.update(field.get("value") or [])
    errors = []
    for missing in sorted(set(pinned) - emitted):
        errors.append(
            f"MISSING cross-repo contract '{missing}': pinned in {CONTRACT_REGISTRY} but no "
            f"companion declares it. Restore the declaration or retire it in both repositories."
        )
    for extra in sorted(emitted - set(pinned)):
        errors.append(
            f"UNPINNED cross-repo contract '{extra}': declared in a companion but absent from "
            f"{CONTRACT_REGISTRY}. Add it there and in the companion repository."
        )
    return errors


def _review_debt_rows(text: str) -> tuple[dict[str, list[tuple[int, list[str]]]], list[str]]:
    """Split the registry into its declared sections and their table rows.

    Returns one entry per section named in REVIEW_DEBT_SECTIONS, each a list of
    (line number, cells). Header and separator rows are consumed here so callers
    only see data rows.
    """
    sections: dict[str, list[tuple[int, list[str]]]] = {name: [] for name in REVIEW_DEBT_SECTIONS}
    errors: list[str] = []
    current_heading: str | None = None
    current: str | None = None
    header_seen: set[str] = set()
    separator_expected: set[str] = set()
    unrecognized_table_rows: dict[str, int] = {}
    fence: str | None = None
    indented_code = False
    previous_blank = False
    for number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        fence_match = re.match(r"^\s{0,3}(`{3,}|~{3,})(.*)$", line)
        if fence_match:
            marker, suffix = fence_match.groups()
            if fence is None:
                fence = marker
                previous_blank = False
                continue
            if marker[0] == fence[0] and len(marker) >= len(fence) and not suffix.strip():
                fence = None
                previous_blank = False
                continue
        if fence is not None:
            continue
        if indented_code:
            if not stripped or line.startswith("    "):
                continue
            indented_code = False
        if previous_blank and line.startswith("    "):
            indented_code = True
            continue
        previous_blank = not stripped
        if stripped.startswith("## "):
            heading = stripped[3:].strip()
            current_heading = heading
            current = heading if heading in REVIEW_DEBT_SECTIONS else None
            continue
        if not stripped.startswith("|"):
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if current is None:
            heading = current_heading or "(before any level-two heading)"
            count = unrecognized_table_rows.get(heading, 0)
            unrecognized_table_rows[heading] = count + 1
            if count >= 2 or (count == 0 and not set("".join(cells)) <= {"-", ":", " "}):
                errors.append(
                    f"{REVIEW_DEBT_REGISTRY}:{number}: ROW under unrecognized heading "
                    f"'## {heading}'. Review-debt entries belong only under exact "
                    f"'## Open' or '## Cleared' headings."
                )
            continue
        if current not in header_seen:
            header_seen.add(current)
            separator_expected.add(current)
            expected = list(REVIEW_DEBT_SECTIONS[current])
            if cells != expected:
                errors.append(
                    f"{REVIEW_DEBT_REGISTRY}:{number}: HEADER of '## {current}' is "
                    f"{cells}, expected {expected}. The check reads these columns by name."
                )
            continue
        separator_like = set("".join(cells)) <= {"-", ":", " "}
        if current in separator_expected:
            separator_expected.remove(current)
            if separator_like:
                continue
        elif separator_like:
            errors.append(
                f"{REVIEW_DEBT_REGISTRY}:{number}: MALFORMED separator-like row in "
                f"'## {current}'. Only the row immediately after the table header may "
                f"be a separator."
            )
            continue
        sections[current].append((number, cells))
    for name in REVIEW_DEBT_SECTIONS:
        if name not in header_seen:
            errors.append(f"MISSING '## {name}' section with its table header in {REVIEW_DEBT_REGISTRY}")
    return sections, errors


def verify_review_debt(root: Path) -> list[str]:
    """Fail when the review-debt registry is malformed.

    A waived merge is only recoverable if its entry stays complete and unambiguous.
    Nothing else notices a half-written row, a debt recorded twice, or a cleared
    entry that never links the review that cleared it.
    """
    registry = root / REVIEW_DEBT_REGISTRY
    if not registry.is_file():
        return [f"MISSING review-debt registry: {REVIEW_DEBT_REGISTRY}"]
    text, _ = _read(registry)
    sections, errors = _review_debt_rows(text)
    seen: dict[tuple[str, str], tuple[str, int]] = {}
    for name, rows in sections.items():
        columns = REVIEW_DEBT_SECTIONS[name]
        placeholder_rows = [number for number, cells in rows if set(cells) == {REVIEW_DEBT_EMPTY_CELL}]
        if placeholder_rows and len(rows) > 1:
            errors.append(
                f"{REVIEW_DEBT_REGISTRY}:{placeholder_rows[0]}: PLACEHOLDER row in '## {name}' "
                f"alongside real entries. It marks an empty section; remove it."
            )
        for number, cells in rows:
            if set(cells) == {REVIEW_DEBT_EMPTY_CELL}:
                continue
            if len(cells) != len(columns):
                errors.append(
                    f"{REVIEW_DEBT_REGISTRY}:{number}: ROW in '## {name}' has {len(cells)} "
                    f"cells, expected {len(columns)}"
                )
                continue
            for column, cell in zip(columns, cells):
                if cell.casefold() in REVIEW_DEBT_UNANSWERED:
                    errors.append(
                        f"{REVIEW_DEBT_REGISTRY}:{number}: UNANSWERED '{column}' in '## {name}'. "
                        f"An entry nobody can act on is the same as no entry."
                    )
            change = cells[columns.index("Change")]
            matches = list(PULL_REQUEST_RE.finditer(change))
            if not matches:
                errors.append(
                    f"{REVIEW_DEBT_REGISTRY}:{number}: NO pull request link in 'Change' of "
                    f"'## {name}'. Link the pull request so the diff stays reachable."
                )
            if name == "Cleared" and not WEB_LINK_RE.search(cells[columns.index("Review")]):
                errors.append(
                    f"{REVIEW_DEBT_REGISTRY}:{number}: CLEARED without a link to the review. "
                    f"A claim that a review happened is not the review."
                )
            for match in matches:
                key = (match.group("repo"), match.group("number"))
                previous = seen.get(key)
                if previous:
                    if previous[0] == name:
                        errors.append(
                            f"{REVIEW_DEBT_REGISTRY}:{number}: {key[0]}#{key[1]} appears more "
                            f"than once in '## {name}' (first seen on line {previous[1]}). "
                            f"Each pull request belongs in one row only."
                        )
                    else:
                        errors.append(
                            f"{REVIEW_DEBT_REGISTRY}:{number}: {key[0]}#{key[1]} appears in both "
                            f"'## {previous[0]}' (line {previous[1]}) and '## {name}'. A debt is "
                            f"open or cleared, never both."
                        )
                seen.setdefault(key, (name, number))
    return errors


def write_or_check_manifest(root: Path, output: Path, check: bool) -> list[str]:
    manifest = build_manifest(root)
    errors = verify_contract_registry(root, manifest)
    if errors:
        return errors
    expected = _manifest_text(manifest)
    if check:
        if not output.is_file():
            return [f"MISSING generated context manifest: {output}"]
        actual, _ = _read(output)
        return [] if actual == expected else [f"STALE generated context manifest: {output}"]
    output.parent.mkdir(parents=True, exist_ok=True)
    _write(output, expected)
    return []


def _resolve_target(entries: list[dict[str, object]], target: str) -> dict[str, object]:
    normalized = target.replace("\\", "/").lstrip("./")
    exact = [e for e in entries if normalized in {e["source"]["value"], e["companion"]["value"]}]  # type: ignore[index]
    if len(exact) == 1:
        return exact[0]
    symbol = [e for e in entries if any(normalized == marker or normalized in marker for marker in e["markers"]["value"])]  # type: ignore[index]
    if len(symbol) == 1:
        return symbol[0]
    if not exact and not symbol:
        raise CompanionError(f"target not found in manifest: {target}")
    raise CompanionError(f"target is ambiguous; use a source path: {target}")


def build_pack(
    root: Path,
    target: str,
    profile: str,
    output: Path | None,
    list_only: bool,
    max_bytes: int,
    max_files: int,
) -> dict[str, object]:
    manifest = build_manifest(root)
    entries: list[dict[str, object]] = manifest["entries"]  # type: ignore[assignment]
    entry = _resolve_target(entries, target)
    by_source = {e["source"]["value"]: e for e in entries}  # type: ignore[index]
    selected = {entry["source"]["value"]}  # type: ignore[index]
    if profile in {"edit", "review", "cross-repo"}:
        selected.update(
            dep["path"] for dep in entry["direct_dependencies"] if dep.get("required") and dep["path"] in by_source  # type: ignore[index]
        )
    if profile in {"review", "cross-repo"}:
        selected.update(dep["path"] for dep in entry["high_risk_dependents"] if dep["path"] in by_source)  # type: ignore[index]
    selected_entries = [by_source[path] for path in sorted(selected)]
    if len(selected_entries) > max_files:
        raise CompanionError(f"pack needs {len(selected_entries)} files, above --max-files={max_files}")
    files: list[tuple[str, bytes]] = []
    for selected_entry in selected_entries:
        companion_rel = selected_entry["companion"]["value"]  # type: ignore[index]
        files.append((companion_rel, (root / companion_rel).read_bytes()))
    pack_meta = {
        "schema_version": SCHEMA_VERSION,
        "profile": profile,
        "target": entry["source"]["value"],  # type: ignore[index]
        "files": [name for name, _ in files],
        "external_contracts": entry["cross_repo_contracts"]["value"] if profile == "cross-repo" else [],  # type: ignore[index]
    }
    metadata = (json.dumps(pack_meta, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")
    files.append(("context-pack.json", metadata))
    total_bytes = sum(len(data) for _, data in files)
    estimate = {
        "profile": profile,
        "target": pack_meta["target"],
        "files": [name for name, _ in files],
        "file_count": len(files),
        "uncompressed_bytes": total_bytes,
        "estimated_tokens": math.ceil(total_bytes / 4),
        "sha256_inputs": hashlib.sha256(b"".join(name.encode() + b"\0" + data for name, data in files)).hexdigest(),
    }
    if total_bytes > max_bytes:
        raise CompanionError(f"pack estimate is {total_bytes} bytes, above --max-bytes={max_bytes}")
    if not list_only:
        assert output is not None
        output.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for name, data in files:
                info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = 0o100644 << 16
                archive.writestr(info, data)
        estimate["output"] = output.as_posix()
        estimate["zip_bytes"] = output.stat().st_size
        estimate["zip_sha256"] = hashlib.sha256(output.read_bytes()).hexdigest()
    return estimate


def _root(value: str | None) -> Path:
    return Path(value).resolve() if value else Path(__file__).resolve().parents[1]


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", help="repository root (defaults to the script's repository)")
    sub = parser.add_subparsers(dest="command", required=True)

    sync_parser = sub.add_parser("sync", help="verify or synchronize supported source fences")
    sync_parser.add_argument("--check", action="store_true", help="report drift without writing")
    sync_parser.add_argument("paths", nargs="*")

    manifest_parser = sub.add_parser("manifest", help="generate the context manifest")
    manifest_parser.add_argument("--check", action="store_true", help="report a stale manifest without writing")
    manifest_parser.add_argument("--output", default="docs/context-manifest.json")

    sub.add_parser("review-debt", help="verify the review-debt registry is well formed")

    pack_parser = sub.add_parser("pack", help="list or build a bounded context ZIP")
    pack_parser.add_argument("target", help="source/companion path or unique marker symbol")
    pack_parser.add_argument("--profile", choices=("understand", "edit", "review", "cross-repo"), default="understand")
    pack_parser.add_argument("--output")
    pack_parser.add_argument("--list", action="store_true", dest="list_only")
    pack_parser.add_argument("--max-bytes", type=int, default=2_000_000)
    pack_parser.add_argument("--max-files", type=int, default=25)

    args = parser.parse_args(argv)
    root = _root(args.repo_root)
    try:
        if args.command == "sync":
            checked, changed, errors = sync(root, args.paths, args.check)
            for error in errors:
                print(error, file=sys.stderr)
            if errors:
                return 1
            action = "verified" if args.check else "synchronized"
            print(f"companion code {action}: {checked} source files ({changed} companion files differed)")
            return 0
        if args.command == "review-debt":
            errors = verify_review_debt(root)
            for error in errors:
                print(error, file=sys.stderr)
            if errors:
                return 1
            print(f"review-debt registry verified: {REVIEW_DEBT_REGISTRY}")
            return 0
        if args.command == "manifest":
            output = (root / args.output).resolve() if not Path(args.output).is_absolute() else Path(args.output)
            errors = write_or_check_manifest(root, output, args.check)
            for error in errors:
                print(error, file=sys.stderr)
            if errors:
                return 1
            print(f"context manifest {'verified' if args.check else 'written'}: {output}")
            return 0
        target_slug = re.sub(r"[^A-Za-z0-9_.-]+", "-", Path(args.target).stem).strip("-") or "target"
        output = None if args.list_only else Path(args.output or f"context-{target_slug}-{args.profile}.zip")
        if output is not None and not output.is_absolute():
            output = (Path.cwd() / output).resolve()
        estimate = build_pack(root, args.target, args.profile, output, args.list_only, args.max_bytes, args.max_files)
        print(json.dumps(estimate, ensure_ascii=False, indent=2, sort_keys=True))
        return 0
    except (CompanionError, OSError, UnicodeError) as exc:
        print(f"companion tool error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
