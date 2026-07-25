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


MARKER_RE = re.compile(r"^(?P<indent>[ \t]*)// md:(?P<name>.+?)[ \t]*$")
RUST_FENCE_RE = re.compile(r"^```rust[ \t]*$", re.MULTILINE)
CLOSE_FENCE_RE = re.compile(r"^```[ \t]*$", re.MULTILINE)
RISK_RULES = (
    ("security", ("auth", "password", "secret", "jwt", "encrypt", "cipher", "crypto", "permission")),
    ("migration", ("migration", "schema_version", "format_version", "migrate")),
    ("protocol", ("protocol", "proto", "wire", "handshake", "sync", "collab", "websocket")),
    ("persistence", ("storage", "database", "repository", "backend", "postgres", "sqlite", "sql")),
)
CONTRACT_TERMS = ("contract", "must", "expects", "wire", "protocol", "format", "compatib", "security")
IGNORED_DIRS = {".git", "target", "graphify-out"}
SCHEMA_VERSION = 1


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


def _read(path: Path) -> tuple[str, str]:
    raw = path.read_bytes().decode("utf-8")
    newline = "\r\n" if raw.count("\r\n") > raw.count("\n") / 2 else "\n"
    return raw.replace("\r\n", "\n"), newline


def _write(path: Path, text: str, newline: str = "\n") -> None:
    path.write_bytes(text.replace("\n", newline).encode("utf-8"))


def iter_sources(root: Path) -> list[Path]:
    return sorted(
        (p for p in root.rglob("*.rs") if not any(part in IGNORED_DIRS for part in p.relative_to(root).parts)),
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


def parse_source(path: Path) -> list[SourceBlock]:
    text, _ = _read(path)
    lines = text.splitlines()
    found: list[tuple[int, str, str]] = []
    for index, line in enumerate(lines):
        match = MARKER_RE.match(line)
        if match:
            found.append((index, match.group("name").strip(), line))
    names = [name for _, name, _ in found]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise CompanionError(f"duplicate source marker(s) in {path}: {', '.join(duplicates)}")
    blocks: list[SourceBlock] = []
    for ordinal, (index, name, marker_line) in enumerate(found):
        next_index = found[ordinal + 1][0] if ordinal + 1 < len(found) else len(lines)
        next_name = found[ordinal + 1][1] if ordinal + 1 < len(found) else None
        if next_name is not None and next_name.startswith(name + " >"):
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


def parse_fences(text: str) -> list[Fence]:
    fences: list[Fence] = []
    for opening in RUST_FENCE_RE.finditer(text):
        content_start = opening.end() + (1 if text[opening.end() :].startswith("\n") else 0)
        closing = CLOSE_FENCE_RE.search(text, content_start)
        if closing is None:
            raise CompanionError(f"unclosed rust fence at character {opening.start()}")
        content_end = closing.start()
        if content_end > content_start and text[content_end - 1] == "\n":
            content_end -= 1
        content = text[content_start:content_end]
        first = content.split("\n", 1)[0] if content else ""
        marker = MARKER_RE.match(first)
        fences.append(
            Fence(
                start=opening.start(),
                content_start=content_start,
                content_end=content_end,
                end=closing.end(),
                content=content,
                marker_name=marker.group("name").strip() if marker else None,
            )
        )
    return fences


def verify_pair(source: Path, companion: Path) -> list[str]:
    errors: list[str] = []
    try:
        blocks = parse_source(source)
    except CompanionError as exc:
        return [str(exc)]
    text, _ = _read(companion)
    try:
        fences = parse_fences(text)
    except CompanionError as exc:
        return [f"{companion}: {exc}"]
    by_name = {block.name: block for block in blocks}
    fence_names = [f.marker_name for f in fences if f.marker_name]
    duplicate_fences = sorted({name for name in fence_names if fence_names.count(name) > 1})
    for name in duplicate_fences:
        errors.append(f"DUPLICATE fence for '// md:{name}' in {companion}")
    for fence in fences:
        if fence.marker_name is None:
            errors.append(f"ORPHAN rust fence without a leading '// md:' marker in {companion}")
            continue
        block = by_name.get(fence.marker_name)
        if block is None:
            errors.append(f"ORPHAN fence '// md:{fence.marker_name}' has no source block in {source}")
        elif block.is_container:
            errors.append(f"CONTAINER '// md:{fence.marker_name}' must not have a rust fence in {companion}")
        elif fence.content != block.code:
            errors.append(
                f"STALE/TRUNCATED fence '// md:{fence.marker_name}' in {companion}; "
                f"source block starts at {source}:{block.start_line}"
            )
    for block in blocks:
        count = fence_names.count(block.name)
        if not block.is_container and count == 0:
            errors.append(f"MISSING fence for '// md:{block.name}' in {companion}")
        if block.is_container and count == 0 and f"// md:{block.name}" not in text:
            errors.append(f"MISSING container section for '// md:{block.name}' in {companion}")
    expected_order = [block.name for block in blocks if not block.is_container]
    actual_order = [name for name in fence_names if name in by_name and not by_name[name].is_container]
    if actual_order != expected_order and not duplicate_fences:
        errors.append(f"REORDERED rust fences in {companion}; fences must follow source leaf order")
    return errors


def _resolve_sources(root: Path, requested: Sequence[str]) -> list[Path]:
    if not requested:
        return iter_sources(root)
    sources: set[Path] = set()
    for raw in requested:
        path = (root / raw).resolve() if not Path(raw).is_absolute() else Path(raw).resolve()
        if path.suffix == ".md" and path.with_suffix(".rs").is_file():
            path = path.with_suffix(".rs")
        if path.is_dir():
            sources.update(p for p in path.rglob("*.rs") if not any(part in IGNORED_DIRS for part in p.parts))
        elif path.suffix == ".rs" and path.is_file():
            sources.add(path)
        else:
            raise CompanionError(f"not a Rust source, companion, or directory: {raw}")
    return sorted(sources, key=lambda p: p.relative_to(root).as_posix())


def sync(root: Path, requested: Sequence[str], check: bool) -> tuple[int, int, list[str]]:
    changed = 0
    checked = 0
    errors: list[str] = []
    for source in _resolve_sources(root, requested):
        companion = source.with_suffix(".md")
        if not companion.is_file():
            errors.append(f"MISSING companion doc: {companion}")
            continue
        checked += 1
        blocks = {block.name: block for block in parse_source(source)}
        text, newline = _read(companion)
        fences = parse_fences(text)
        replacements: list[tuple[int, int, str]] = []
        for fence in fences:
            if fence.marker_name and fence.marker_name in blocks and blocks[fence.marker_name].code is not None:
                expected = blocks[fence.marker_name].code
                if fence.content != expected:
                    replacements.append((fence.content_start, fence.content_end, expected or ""))
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
                _write(companion, text, newline)
    return checked, changed, errors


def _section(text: str, heading: str) -> str:
    pattern = re.compile(rf"^\*\*{re.escape(heading)}\*\*.*?$", re.MULTILINE)
    match = pattern.search(text)
    if not match:
        return ""
    rest = text[match.end() :]
    boundary = re.search(r"^(?:\*\*|#{1,6}\s|---\s*$)", rest, re.MULTILINE)
    return rest[: boundary.start()] if boundary else rest


def _bullets(section: str) -> list[str]:
    values = []
    for line in section.splitlines():
        if line.startswith("- "):
            values.append(line[2:].strip())
    return values


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
        companion = source.with_suffix(".md")
        if not companion.is_file():
            continue
        source_rel = source.relative_to(root).as_posix()
        companion_rel = companion.relative_to(root).as_posix()
        text, _ = _read(companion)
        blocks = parse_source(source)
        deps_section = _section(text, "Direct dependencies")
        dependents_section = _section(text, "Direct dependents")
        dependencies = []
        for bullet in _bullets(deps_section):
            dep = _path_from_bullet(bullet, source, root)
            if dep:
                dependencies.append(
                    {
                        "path": dep,
                        "origin": "EXTRACTED" if "(EXTRACTED)" in bullet else "INFERRED",
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
                        "origin": "EXTRACTED" if "(EXTRACTED)" in bullet else "INFERRED",
                    }
                )
        invariants = [
            {
                "origin": "EXTRACTED" if "(EXTRACTED)" in bullet else "INFERRED",
                "value": bullet,
            }
            for bullet in _bullets(_section(text, "Invariants"))
        ]
        contract_lines = []
        prose = re.sub(r"^```rust\s*$.*?^```\s*$", "", text, flags=re.MULTILINE | re.DOTALL)
        for line in prose.splitlines():
            lower = line.lower()
            explicit_cross_repo = (
                "keeplin-srv" in lower
                or "cross-repo" in lower
                or "keeplin client" in lower
                or "desktop client" in lower
            )
            if explicit_cross_repo:
                cleaned = re.sub(r"\s+", " ", line.strip(" -"))
                if cleaned and cleaned not in contract_lines:
                    contract_lines.append(cleaned)
        title = next((line[2:].strip() for line in text.splitlines() if line.startswith("# ")), companion_rel)
        draft.append(
            {
                "source": {"origin": "EXTRACTED", "value": source_rel},
                "companion": {"origin": "EXTRACTED", "value": companion_rel},
                "purpose": {"origin": "EXTRACTED", "value": title},
                "markers": {"origin": "EXTRACTED", "value": [block.name for block in blocks]},
                "invariants": {"origin": "INFERRED", "value": invariants},
                "cross_repo_contracts": {"origin": "INFERRED", "value": contract_lines[:8]},
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
    return json.dumps(build_manifest(root), ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def write_or_check_manifest(root: Path, output: Path, check: bool) -> list[str]:
    expected = manifest_text(root)
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

    sync_parser = sub.add_parser("sync", help="verify or synchronize Rust fences")
    sync_parser.add_argument("--check", action="store_true", help="report drift without writing")
    sync_parser.add_argument("paths", nargs="*")

    manifest_parser = sub.add_parser("manifest", help="generate the context manifest")
    manifest_parser.add_argument("--check", action="store_true", help="report a stale manifest without writing")
    manifest_parser.add_argument("--output", default="docs/context-manifest.json")

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
