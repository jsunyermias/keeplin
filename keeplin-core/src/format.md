# `format.rs` — the hard format limits shared by client and server

Self-contained companion for `keeplin-core/src/format.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this
file must be able to understand and modify the module without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section here;
grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use thiserror::Error;

use crate::error::StorageError;
```

**What it does** — The **single source of truth** for keeplin's three hard format limits
(issue keeplin#130) and for the wire codes the collaborative channel uses to reject an
operation that breaks one. The module is deliberately I/O-free and dependency-light: it
holds constants plus total, pure predicates, so both sides of the system can call it on
every edit without cost, and so a small model can reason about the whole format contract by
reading one file.

| Limit | Constant | Value |
|---|---|---|
| bytes per line | `MAX_LINE_BYTES` | 2¹² = 4096 |
| lines per note | `MAX_LINES_PER_NOTE` | 2¹⁶ = 65 536 |
| notes per notebook | `MAX_NOTES_PER_NOTEBOOK` | 2²⁴ = 16 777 216 |

The limits are a **shared wire/format contract**: `keeplin-srv` imports these very constants
(it pins `keeplin-core` to an exact git `rev`) instead of redefining them, so the server can
never drift from the client. Before this module existed the server carried its own
`MAX_LINE_LEN = 10_000` / `MAX_LINES_PER_NOTE = 100_000` and the client knew nothing: an
oversized edit looked saved locally, was dropped by the server, and never reached the user's
other devices. See "The format contract" below for the invariants that closed that hole.

**Dependencies** —
- `thiserror::Error` — derives `Display`/`std::error::Error` for `LimitViolation`; expects
  the derive to keep interpolating the module's `const`s in the `#[error(...)]` strings
  (Rust's implicit format-argument capture resolves them from module scope), so the message
  always quotes the limit that was breached.
- `crate::error::StorageError` — the target of the `From` conversion; expects the
  `TooLarge(String)` variant to keep existing and to keep being mapped by the daemon to
  HTTP 413 / gRPC `OUT_OF_RANGE`.

**Used by** — `collab/state.rs` (`NoteLines::diff_body` validates before emitting ops),
`collab/mod.rs` (`CollabBackend::create_note`/`update_note` validate before the local write;
`handle_server_msg` uses `is_limit_code`), `ordering.rs` (`place_new_note` gates the
notes-per-notebook cap), `keeplin-daemon/src/rest.rs` and `keeplin-daemon/src/server.rs`
(entry-point validation), and `keeplin-srv` (`crates/keeplin-srv/src/collab.rs`,
`crates/keeplin-srv/src/http.rs`, `crates/keeplin-srv/tests/core_compat.rs`).

**Repeated context** — Project premise: clean breaks, no migrations. Lowering the line limit
from 10 000 to 4096 bytes and the line count from 100 000 to 65 536 is a deliberate hard
break with no data migration path; `PROTOCOL_VERSION` (`compat.rs`) is unchanged because the
message shapes did not break — only the values a server will accept.

---

## MAX_LINE_BYTES

**Identification** — `pub const MAX_LINE_BYTES: usize = 1 << 12;`; marker
`// md:MAX_LINE_BYTES`.

**Code** — complete and verbatim:

```rust
// md:MAX_LINE_BYTES
pub const MAX_LINE_BYTES: usize = 1 << 12;
```

**What it does** — 4096 **UTF-8 bytes** is the largest line either side will accept. Bytes,
not `char`s: `str::len()` is O(1) on both sides and cannot disagree, whereas a `chars()`
count would have to be recomputed identically by every implementation. A line of 4096 bytes
is accepted; 4097 is rejected. Written as `1 << 12` so the "exact power of two" property is
visible in the source, not just asserted in a test.

**Dependencies** — none.

**Used by** — `check_line`; `keeplin-srv`'s `collab.rs` (`content.len() > MAX_LINE_BYTES` on
`Insert`/`Update`); the boundary tests below.

**Repeated context** — a "line" never contains `\n`; the collab channel rejects embedded
newlines separately (`bad_content`), so byte length and line identity stay independent.

---

## MAX_LINES_PER_NOTE

**Identification** — `pub const MAX_LINES_PER_NOTE: usize = 1 << 16;`; marker
`// md:MAX_LINES_PER_NOTE`.

**Code** — complete and verbatim:

```rust
// md:MAX_LINES_PER_NOTE
pub const MAX_LINES_PER_NOTE: usize = 1 << 16;
```

**What it does** — 65 536 **live** lines is the largest note either side will accept. "Live"
means not tombstoned: the client counts the lines of the materialised body, the server counts
the note's rows with `deleted_at IS NULL`. Counting the raw order vector instead would make
the two disagree on a note with many deleted lines, which is exactly the silent divergence
this issue exists to remove.

**Dependencies** — none.

**Used by** — `check_line_count`, `check_body`; `keeplin-srv`'s `collab.rs` (live-line count
before an `Insert`); the boundary tests below.

**Repeated context** — the collab channel is line-addressed: a note's body is `order`-joined
line content, so "lines per note" is a real structural bound, not a proxy for byte size.

---

## MAX_NOTES_PER_NOTEBOOK

**Identification** — `pub const MAX_NOTES_PER_NOTEBOOK: usize = 1 << 24;`; marker
`// md:MAX_NOTES_PER_NOTEBOOK`.

**Code** — complete and verbatim:

```rust
// md:MAX_NOTES_PER_NOTEBOOK
pub const MAX_NOTES_PER_NOTEBOOK: usize = 1 << 24;
```

**What it does** — 16 777 216 live notes is the largest a single notebook may hold. A
sanity ceiling rather than an everyday constraint: it is checked when a note is **created in**
or **moved into** a notebook, against the destination's live-note count. The Inbox
(`ordering::INBOX_ID`, the nil UUID) is a notebook like any other and is subject to the same
cap.

**Dependencies** — none.

**Used by** — `check_notebook_capacity`; `ordering::place_new_note` (which
`reconcile_notebook_move` routes every move through); `keeplin-srv`'s `http.rs` (the
`PATCH /api/notes/:id` move path); the boundary tests below.

**Repeated context** — `sort_key` bands (`ordering.rs`): pinned keys live in `1..=999` and the
normal band starts at `Note::DEFAULT_SORT_KEY`. The cap counts notes, not keys, so it is
independent of the banding.

---

## CODE_LINE_TOO_LONG

**Identification** — `pub const CODE_LINE_TOO_LONG: &str = "too_long";`; marker
`// md:CODE_LINE_TOO_LONG`.

**Code** — complete and verbatim:

```rust
// md:CODE_LINE_TOO_LONG
pub const CODE_LINE_TOO_LONG: &str = "too_long";
```

**What it does** — The `CollabServerMsg::Error { code }` string the server sends when a line
exceeds `MAX_LINE_BYTES`. The literal `"too_long"` is the code keeplin-srv has always sent;
it is lifted into keeplin-core unchanged so both repositories reference one definition
instead of two string literals that can silently diverge.

**Dependencies** — none.

**Used by** — `LimitViolation::code`, `is_limit_code`; `keeplin-srv`'s `collab.rs` when
rejecting an `Insert`/`Update`.

**Repeated context** — collab error codes are lowercase snake_case, stable, and machine-read
by the client — they are API, not log text.

---

## CODE_TOO_MANY_LINES

**Identification** — `pub const CODE_TOO_MANY_LINES: &str = "too_many_lines";`; marker
`// md:CODE_TOO_MANY_LINES`.

**Code** — complete and verbatim:

```rust
// md:CODE_TOO_MANY_LINES
pub const CODE_TOO_MANY_LINES: &str = "too_many_lines";
```

**What it does** — The error code for a note that would exceed `MAX_LINES_PER_NOTE` live
lines. Also the code keeplin-srv already used, now defined once here.

**Dependencies** — none.

**Used by** — `LimitViolation::code`, `is_limit_code`; `keeplin-srv`'s `collab.rs` when
rejecting an `Insert` into a full note.

**Repeated context** — see `CODE_LINE_TOO_LONG`.

---

## CODE_NOTEBOOK_FULL

**Identification** — `pub const CODE_NOTEBOOK_FULL: &str = "notebook_full";`; marker
`// md:CODE_NOTEBOOK_FULL`.

**Code** — complete and verbatim:

```rust
// md:CODE_NOTEBOOK_FULL
pub const CODE_NOTEBOOK_FULL: &str = "notebook_full";
```

**What it does** — The code for a notebook at `MAX_NOTES_PER_NOTEBOOK`. New in this issue —
no notes-per-notebook cap existed before. Unlike the other two it never travels on the collab
WebSocket (notebook membership is REST, not a line op): it surfaces as the `AppError::
QuotaExceeded` payload on keeplin-srv's `PATCH /api/notes/:id` and inside
`StorageError::TooLarge` on the client.

**Dependencies** — none.

**Used by** — `LimitViolation::code`, `is_limit_code`; keeplin-srv's note-move handler.

**Repeated context** — see `CODE_LINE_TOO_LONG`.

---

## LimitViolation

**Identification** — enum deriving `Debug, Clone, PartialEq, Eq, Error`; marker
`// md:LimitViolation`.

**Code** — complete and verbatim:

```rust
// md:LimitViolation
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LimitViolation {
    #[error("line of {bytes} bytes exceeds the format limit of {MAX_LINE_BYTES} bytes")]
    LineTooLong { bytes: usize },

    #[error("note of {lines} lines exceeds the format limit of {MAX_LINES_PER_NOTE} lines")]
    TooManyLines { lines: usize },

    #[error(
        "notebook already holds {notes} notes, the format limit of {MAX_NOTES_PER_NOTEBOOK} notes"
    )]
    NotebookFull { notes: usize },
}
```

**What it does** — One variant per limit, each carrying the **observed** magnitude so the
message can quote both what was attempted and what is allowed. `PartialEq, Eq` exist so tests
can assert the exact violation rather than merely "some error". `Clone` lets a caller keep the
violation after converting a copy into a `StorageError`. This is the crate's only
limit-specific error type; everything user-facing goes through the `From` conversion below.

**Dependencies** —
- `thiserror::Error` — generates `Display` and `std::error::Error`; expects the `#[error]`
  format strings to keep resolving `MAX_LINE_BYTES`, `MAX_LINES_PER_NOTE` and
  `MAX_NOTES_PER_NOTEBOOK` from module scope. Renaming a constant without editing the
  attribute is a compile error, which is the intended coupling.

**Used by** — every `check_*` function's `Err`; `NoteLines::diff_body`'s error type;
`collab/mod.rs` log lines; the tests below.

**Repeated context** — error convention of the crate: `StorageError` is the type that crosses
module boundaries; a module-local error type is fine as long as it converts into it.

---

## impl LimitViolation

**Identification** — inherent impl; marker `// md:impl LimitViolation`. One method.

**Code** — container: members documented as sub-blocks below: fn code.

**What it does** — Attaches the wire code to the violation, so a caller never has to
re-derive which string belongs to which limit.

**Dependencies** — the three `CODE_*` constants.

**Used by** — `ordering.rs`'s test, `collab/state.rs`'s tests, and any caller that has to put
a violation on the wire.

**Repeated context** — none.

### fn code

**Identification** — `pub fn code(&self) -> &'static str`; marker
`// md:impl LimitViolation > fn code`.

**Code** — complete and verbatim:

```rust
    // md:impl LimitViolation > fn code
    pub fn code(&self) -> &'static str {
        match self {
            LimitViolation::LineTooLong { .. } => CODE_LINE_TOO_LONG,
            LimitViolation::TooManyLines { .. } => CODE_TOO_MANY_LINES,
            LimitViolation::NotebookFull { .. } => CODE_NOTEBOOK_FULL,
        }
    }
```

**What it does** — Total mapping from variant to wire code. The `match` is exhaustive with no
catch-all arm on purpose: adding a fourth limit will not compile until its code is defined
and mapped here.

**Dependencies** — `CODE_LINE_TOO_LONG`, `CODE_TOO_MANY_LINES`, `CODE_NOTEBOOK_FULL` — expects
each to stay the exact string keeplin-srv puts in `CollabServerMsg::Error { code }`; if one
drifts, the client stops recognising the rejection and silently degrades to a warning.

**Used by** — tests in `ordering.rs` and `collab/state.rs`; available to any server-side
adapter that reports a violation.

**Repeated context** — none.

---

## impl From LimitViolation for StorageError

**Identification** — trait impl `impl From<LimitViolation> for StorageError`; marker
`// md:impl From LimitViolation for StorageError`.

**Code** — complete and verbatim:

```rust
// md:impl From LimitViolation for StorageError
impl From<LimitViolation> for StorageError {
    fn from(violation: LimitViolation) -> Self {
        StorageError::TooLarge(violation.to_string())
    }
}
```

**What it does** — The single bridge from this module's error type to the crate-wide one, so
`?` propagates a violation out of any `Result<_, StorageError>` function. Every limit becomes
`StorageError::TooLarge`, which the daemon maps to **HTTP 413 Payload Too Large**
(`rest.rs`) and **gRPC `OUT_OF_RANGE`** (`server.rs`). The rendered `Display` text is kept as
the payload so the user sees which limit was breached and by how much.

**Dependencies** — `StorageError::TooLarge` — expects the variant to exist and to keep its
dedicated status mapping; if it were folded back into `InvalidInput` the API would report 400
and callers could no longer distinguish "malformed" from "too big".

**Used by** — `ordering::place_new_note` (`?`), `collab/mod.rs`'s `create_note`/`update_note`
(`?`), `rest.rs` and `server.rs` (`map_err(StorageError::from)`).

**Repeated context** — the daemon's HTTP status mapping lives in `rest.rs`'s
`impl IntoResponse for ApiError`; the gRPC mapping in `server.rs`'s `storage_err`. Both must
carry a `TooLarge` arm.

---

## fn is_limit_code

**Identification** — `pub fn is_limit_code(code: &str) -> bool`; marker
`// md:fn is_limit_code`.

**Code** — complete and verbatim:

```rust
// md:fn is_limit_code
pub fn is_limit_code(code: &str) -> bool {
    matches!(
        code,
        CODE_LINE_TOO_LONG | CODE_TOO_MANY_LINES | CODE_NOTEBOOK_FULL
    )
}
```

**What it does** — The client's inverse of `LimitViolation::code`: given a code string from a
`CollabServerMsg::Error`, says whether it is a format-limit rejection. This is what lets
`handle_server_msg` treat a limit rejection specially — dropping the cached note state and
forcing a rejoin so the server snapshot overwrites the divergent local body — while every
other code (`forbidden`, `not_found`, `bad_content`, …) stays a plain warning.

**Dependencies** — the three `CODE_*` constants; expects them to be the same strings the
server sends.

**Used by** — `collab/mod.rs`'s `handle_server_msg`; the mapping test below.

**Repeated context** — codes are matched exactly; the client never pattern-matches on the
human-readable `message`, which is free text and may change.

---

## fn check_line

**Identification** — `pub fn check_line(content: &str) -> Result<(), LimitViolation>`; marker
`// md:fn check_line`.

**Code** — complete and verbatim:

```rust
// md:fn check_line
pub fn check_line(content: &str) -> Result<(), LimitViolation> {
    if content.len() > MAX_LINE_BYTES {
        return Err(LimitViolation::LineTooLong {
            bytes: content.len(),
        });
    }
    Ok(())
}
```

**What it does** — Accepts a line of exactly `MAX_LINE_BYTES` bytes, rejects one byte more.
`str::len()` is the **UTF-8 byte length**, so a 2048-character string of two-byte characters
sits exactly at the limit and one more character overflows it by two — deliberate, and
asserted in `line_length_is_counted_in_utf8_bytes`. Total: no panics, no allocation.

**Dependencies** — `MAX_LINE_BYTES`, `LimitViolation::LineTooLong`.

**Used by** — `check_body`; available directly to callers validating a single line.

**Repeated context** — keeplin-srv applies the identical `content.len() > MAX_LINE_BYTES` test
on `Insert` and `Update`, importing this constant.

---

## fn check_line_count

**Identification** — `pub fn check_line_count(lines: usize) -> Result<(), LimitViolation>`;
marker `// md:fn check_line_count`.

**Code** — complete and verbatim:

```rust
// md:fn check_line_count
pub fn check_line_count(lines: usize) -> Result<(), LimitViolation> {
    if lines > MAX_LINES_PER_NOTE {
        return Err(LimitViolation::TooManyLines { lines });
    }
    Ok(())
}
```

**What it does** — Takes the **resulting** live-line count and accepts it up to and including
`MAX_LINES_PER_NOTE` (`>` , not `>=`). Contrast with `check_notebook_capacity`, which takes the
count *before* the insertion and therefore uses `>=`: the asymmetry is deliberate and matches
how each caller has the number to hand.

**Dependencies** — `MAX_LINES_PER_NOTE`, `LimitViolation::TooManyLines`.

**Used by** — `check_body`; keeplin-srv passes its own live-line count here in spirit (it
compares against the same constant before an `Insert`).

**Repeated context** — see `MAX_LINES_PER_NOTE` on why the count is of live lines.

---

## fn line_count

**Identification** — `pub fn line_count(body: &str) -> usize`; marker `// md:fn line_count`.

**Code** — complete and verbatim:

```rust
// md:fn line_count
pub fn line_count(body: &str) -> usize {
    if body.is_empty() {
        0
    } else {
        body.split('\n').count()
    }
}
```

**What it does** — Counts the lines a body materialises into, using exactly the rule
`NoteLines::diff_body` uses: an empty body is **zero** lines (not one), and otherwise the
count is the number of `\n`-separated segments — so `"a\n"` is two lines, the second empty.
Keeping this rule in one function is what makes "the client accepted it" and "the check
passed" mean the same thing.

**Dependencies** — none.

**Used by** — the tests below; exported for callers that need the count without the check.

**Repeated context** — `NoteLines::materialize` joins live line contents with `\n`, and
`diff_body` splits on `\n` with the same empty-body special case; the three must agree.

---

## fn check_body

**Identification** — `pub fn check_body(body: &str) -> Result<(), LimitViolation>`; marker
`// md:fn check_body`.

**Code** — complete and verbatim:

```rust
// md:fn check_body
pub fn check_body(body: &str) -> Result<(), LimitViolation> {
    if body.is_empty() {
        return Ok(());
    }
    let mut lines = 0usize;
    for line in body.split('\n') {
        lines += 1;
        check_line(line)?;
    }
    check_line_count(lines)
}
```

**What it does** — The whole-note gate: every line within `MAX_LINE_BYTES` **and** at most
`MAX_LINES_PER_NOTE` lines. One pass, counting while checking; the first over-long line short-
circuits, so a pathological body is rejected without walking all of it. An empty body is
trivially valid. This is the function every write path calls **before** touching local storage
or emitting an op, which is what turns a server rejection into an impossible state rather than
a silent divergence.

**Dependencies** — `check_line` (per-line bound), `check_line_count` (total bound); expects
both to stay total and allocation-free, since this runs on every note write.

**Used by** — `NoteLines::diff_body`; `CollabBackend::create_note`/`update_note`;
`keeplin-daemon`'s `rest.rs` (`create_note`, `update_note`) and `server.rs` (the gRPC
equivalents).

**Repeated context** — reject, never truncate: the project treats silent truncation of user
content as data loss, so every limit is a hard rejection with a propagated error.

---

## fn check_notebook_capacity

**Identification** —
`pub fn check_notebook_capacity(live_notes: usize) -> Result<(), LimitViolation>`; marker
`// md:fn check_notebook_capacity`.

**Code** — complete and verbatim:

```rust
// md:fn check_notebook_capacity
pub fn check_notebook_capacity(live_notes: usize) -> Result<(), LimitViolation> {
    if live_notes >= MAX_NOTES_PER_NOTEBOOK {
        return Err(LimitViolation::NotebookFull { notes: live_notes });
    }
    Ok(())
}
```

**What it does** — Answers "may one more note enter this notebook?". `live_notes` is the
destination's **current** live count, so the comparison is `>=`: at exactly
`MAX_NOTES_PER_NOTEBOOK` the notebook is full and the next note is refused.

**Dependencies** — `MAX_NOTES_PER_NOTEBOOK`, `LimitViolation::NotebookFull`.

**Used by** — `ordering::place_new_note`, which reads the count from
`NotebookSortProfile::live_notes` and therefore covers both creation and every
`reconcile_notebook_move`; keeplin-srv's `PATCH /api/notes/:id` move path.

**Repeated context** — the Inbox is `ordering::INBOX_ID` (the nil UUID) and is a normal
notebook for capacity purposes.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`. Six tests.

**Code** — container: members documented as sub-blocks below: fn
the_three_limits_are_exact_powers_of_two, fn line_length_is_counted_in_utf8_bytes, fn
line_count_boundary_accepts_the_limit_and_rejects_one_more, fn
line_count_matches_the_materialised_body, fn check_body_enforces_both_line_limits, fn
notebook_capacity_rejects_the_note_that_would_exceed_the_cap, fn
every_violation_maps_to_its_wire_code_and_to_too_large.

**What it does** — Pins the numeric values, the byte-counting decision and every boundary
(limit accepted, limit + 1 rejected) for all three limits. Because the constants live here,
these are the tests that would fail first if someone "rounded" a limit to a non-power of two
or switched line length to `chars`.

**Dependencies** — `super::*`.

**Used by** — CI (`cargo test --workspace`).

**Repeated context** — project test convention: pure logic in in-file `#[cfg(test)]` tests;
anything needing sockets or a filesystem in `keeplin-core/tests/`.

The explicit `imports` leaf below preserves the test-module dependency preamble
verbatim.

### imports

**Identification** — test-module dependencies; marker `// md:mod tests > imports`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > imports
    use super::*;
```

**What it does** — Brings the parent module API and test-only dependencies into
scope.

**Dependencies** —

- `super::*` — the items under test from the parent module; expects: the parent keeps them at module scope; a rename or a move into a submodule breaks these tests at compile time, which is the intended early signal.

**Used by** — every block of `mod tests` in this file: `fn the_three_limits_are_exact_powers_of_two`, `fn line_length_is_counted_in_utf8_bytes`, `fn line_count_boundary_accepts_the_limit_and_rejects_one_more`, `fn line_count_matches_the_materialised_body`, `fn check_body_enforces_both_line_limits`, `fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap`, `fn every_violation_maps_to_its_wire_code_and_to_too_large`. Nothing outside the module can use it: the preamble is private to `mod tests`.

**Repeated context** — This preamble is a leaf block, not scaffolding: only the `mod` declaration, its attributes and its braces are exempt from coverage, so these `use` lines carry their own marker and are verified verbatim against the source (template v2.5.0, RULE 6). Changing an import here without updating this fence fails `scripts/check-docs.sh`.

### fn the_three_limits_are_exact_powers_of_two

**Identification** — unit test; marker
`// md:mod tests > fn the_three_limits_are_exact_powers_of_two`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn the_three_limits_are_exact_powers_of_two
    #[test]
    fn the_three_limits_are_exact_powers_of_two() {
        assert_eq!(MAX_LINE_BYTES, 4096);
        assert_eq!(MAX_LINES_PER_NOTE, 65_536);
        assert_eq!(MAX_NOTES_PER_NOTEBOOK, 16_777_216);
        assert!(MAX_LINE_BYTES.is_power_of_two());
        assert!(MAX_LINES_PER_NOTE.is_power_of_two());
        assert!(MAX_NOTES_PER_NOTEBOOK.is_power_of_two());
    }
```

**What it does** — Asserts the decimal values (4096 / 65 536 / 16 777 216) and the
power-of-two property that issue #130 makes an acceptance criterion. Cheap, but it is the
guard that makes an accidental edit of a `1 << n` visible.

**Dependencies** — the three limit constants.

**Used by** — CI only.

**Repeated context** — none.

### fn line_length_is_counted_in_utf8_bytes

**Identification** — unit test; marker
`// md:mod tests > fn line_length_is_counted_in_utf8_bytes`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn line_length_is_counted_in_utf8_bytes
    #[test]
    fn line_length_is_counted_in_utf8_bytes() {
        assert!(check_line(&"a".repeat(MAX_LINE_BYTES)).is_ok());
        assert_eq!(
            check_line(&"a".repeat(MAX_LINE_BYTES + 1)),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 1
            })
        );
        let two_byte_chars = "é".repeat(MAX_LINE_BYTES / 2);
        assert_eq!(two_byte_chars.chars().count(), MAX_LINE_BYTES / 2);
        assert!(check_line(&two_byte_chars).is_ok());
        let one_char_over = "é".repeat(MAX_LINE_BYTES / 2 + 1);
        assert_eq!(
            check_line(&one_char_over),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 2
            })
        );
    }
```

**What it does** — The ASCII boundary (4096 ok, 4097 rejected) plus the decision that settles
the "bytes or chars?" question: 2048 two-byte characters are exactly at the limit, and 2049
overflow it by **two bytes**, not one. A `chars`-based implementation would accept the second
case, so this test fails loudly if the counting rule is ever changed on one side only.

**Dependencies** — `check_line`, `LimitViolation::LineTooLong`, `MAX_LINE_BYTES`.

**Used by** — CI only.

**Repeated context** — keeplin-srv counts bytes too (`content.len()`); the cross-repo test
`crates/keeplin-srv/tests/core_compat.rs` re-asserts the shared constant.

### fn line_count_boundary_accepts_the_limit_and_rejects_one_more

**Identification** — unit test; marker
`// md:mod tests > fn line_count_boundary_accepts_the_limit_and_rejects_one_more`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn line_count_boundary_accepts_the_limit_and_rejects_one_more
    #[test]
    fn line_count_boundary_accepts_the_limit_and_rejects_one_more() {
        assert!(check_line_count(MAX_LINES_PER_NOTE).is_ok());
        assert_eq!(
            check_line_count(MAX_LINES_PER_NOTE + 1),
            Err(LimitViolation::TooManyLines {
                lines: MAX_LINES_PER_NOTE + 1
            })
        );
    }
```

**What it does** — Fixes the inclusive edge: 65 536 lines pass, 65 537 fail, and the returned
violation reports the observed count.

**Dependencies** — `check_line_count`, `LimitViolation::TooManyLines`, `MAX_LINES_PER_NOTE`.

**Used by** — CI only.

**Repeated context** — none.

### fn line_count_matches_the_materialised_body

**Identification** — unit test; marker
`// md:mod tests > fn line_count_matches_the_materialised_body`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn line_count_matches_the_materialised_body
    #[test]
    fn line_count_matches_the_materialised_body() {
        assert_eq!(line_count(""), 0);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("a\n"), 2);
    }
```

**What it does** — Pins the empty-body-is-zero-lines special case and the trailing-newline
case (`"a\n"` is two lines), the two places where a naive `matches('\n') + 1` would disagree
with `diff_body`.

**Dependencies** — `line_count`.

**Used by** — CI only.

**Repeated context** — `NoteLines::diff_body` uses the same `body.is_empty()` guard before
`split('\n')`; if one changes, both must.

### fn check_body_enforces_both_line_limits

**Identification** — unit test; marker
`// md:mod tests > fn check_body_enforces_both_line_limits`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn check_body_enforces_both_line_limits
    #[test]
    fn check_body_enforces_both_line_limits() {
        assert!(check_body("").is_ok());
        let at_line_limit = "x\n".repeat(MAX_LINES_PER_NOTE - 1) + "x";
        assert_eq!(line_count(&at_line_limit), MAX_LINES_PER_NOTE);
        assert!(check_body(&at_line_limit).is_ok());
        let over_line_limit = "x\n".repeat(MAX_LINES_PER_NOTE) + "x";
        assert_eq!(line_count(&over_line_limit), MAX_LINES_PER_NOTE + 1);
        assert_eq!(
            check_body(&over_line_limit),
            Err(LimitViolation::TooManyLines {
                lines: MAX_LINES_PER_NOTE + 1
            })
        );
        let long_line = format!("ok\n{}", "a".repeat(MAX_LINE_BYTES + 1));
        assert_eq!(
            check_body(&long_line),
            Err(LimitViolation::LineTooLong {
                bytes: MAX_LINE_BYTES + 1
            })
        );
    }
```

**What it does** — Exercises the composite gate on real bodies: empty is fine, a body of
exactly 65 536 lines passes, 65 537 fails with `TooManyLines`, and an over-long line **not in
first position** still fails with `LineTooLong` — proving the per-line check runs across the
whole body, not just its head.

**Dependencies** — `check_body`, `line_count`, both `LimitViolation` line variants, both line
constants.

**Used by** — CI only.

**Repeated context** — none.

### fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap

**Identification** — unit test; marker
`// md:mod tests > fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap
    #[test]
    fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap() {
        assert!(check_notebook_capacity(0).is_ok());
        assert!(check_notebook_capacity(MAX_NOTES_PER_NOTEBOOK - 1).is_ok());
        assert_eq!(
            check_notebook_capacity(MAX_NOTES_PER_NOTEBOOK),
            Err(LimitViolation::NotebookFull {
                notes: MAX_NOTES_PER_NOTEBOOK
            })
        );
    }
```

**What it does** — Pins the `>=` edge: an empty notebook and one holding 16 777 215 notes both
accept another; one already holding 16 777 216 does not. Materialising 2²⁴ notes to test this
through a backend would be absurd, so the boundary is asserted on the pure predicate and
`ordering.rs`'s test proves `place_new_note` feeds it the right number.

**Dependencies** — `check_notebook_capacity`, `LimitViolation::NotebookFull`,
`MAX_NOTES_PER_NOTEBOOK`.

**Used by** — CI only.

**Repeated context** — none.

### fn every_violation_maps_to_its_wire_code_and_to_too_large

**Identification** — unit test; marker
`// md:mod tests > fn every_violation_maps_to_its_wire_code_and_to_too_large`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn every_violation_maps_to_its_wire_code_and_to_too_large
    #[test]
    fn every_violation_maps_to_its_wire_code_and_to_too_large() {
        let violations = [
            LimitViolation::LineTooLong { bytes: 1 },
            LimitViolation::TooManyLines { lines: 1 },
            LimitViolation::NotebookFull { notes: 1 },
        ];
        for violation in violations {
            assert!(is_limit_code(violation.code()));
            let mapped: StorageError = violation.clone().into();
            assert!(matches!(mapped, StorageError::TooLarge(_)));
            assert_eq!(mapped.to_string(), format!("Too large: {violation}"));
        }
        assert!(!is_limit_code("bad_content"));
    }
```

**What it does** — Closes the loop between the three surfaces: every variant's `code()` is
recognised by `is_limit_code` (so the client will react to it), every variant converts to
`StorageError::TooLarge` (so the daemon answers 413 / `OUT_OF_RANGE`), and the rendered
message survives the conversion. The negative case (`"bad_content"`, a real non-limit collab
code) proves `is_limit_code` is not accidentally total.

**Dependencies** — `LimitViolation::code`, `is_limit_code`, the `From` impl,
`StorageError::TooLarge`.

**Used by** — CI only.

**Repeated context** — none.

---

## The format contract

The three constants and their invariants, restated so a change to either repository can be
evaluated from this file alone:

1. **One definition.** The values live here and nowhere else. `keeplin-srv` imports
   `keeplin_core::format` rather than redefining; its `Cargo.toml` pins keeplin-core to an
   exact git `rev` (never a branch), so the server cannot drift from the client between
   releases.
2. **Same units on both sides.** Line length is UTF-8 **bytes**; line count is **live** lines
   (tombstones excluded); notebook capacity is **live** notes (soft-deleted excluded). A line
   the client accepts is never rejected by the server for a counting reason, and vice versa.
3. **Reject, never truncate.** Exceeding a limit is an error the user sees: HTTP 413 on the
   daemon's REST API, gRPC `OUT_OF_RANGE`, `CollabServerMsg::Error` on the collab socket. No
   path silently shortens content.
4. **Validate before writing.** The client checks *before* the local write and *before*
   emitting an op, so an edit that the server would reject never reaches local storage. The
   server still checks independently — it cannot trust a client — and when it does reject,
   the error now carries `note_id` so the client can drop its cached state and resynchronise
   instead of diverging.
5. **Clean break.** Lowering the line limit (10 000 → 4096) and the line count (100 000 →
   65 536) is not migrated: pre-existing oversized content is simply no longer editable
   through paths that revalidate it. `PROTOCOL_VERSION` is unchanged because no message shape
   broke.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every companion
because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the navigation
model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `MAX_LINE_BYTES` — defined here; read by `check_line` and by keeplin-srv (INFERRED)
- `MAX_LINES_PER_NOTE` — defined here; read by `check_line_count` and by keeplin-srv (INFERRED)
- `MAX_NOTES_PER_NOTEBOOK` — defined here; read by `check_notebook_capacity` (INFERRED)
- `LimitViolation` — defined here (EXTRACTED; file-local)
- `check_body()` — defined here; the crate's whole-note gate (EXTRACTED)
- `check_notebook_capacity()` — defined here (EXTRACTED)
- `is_limit_code()` — defined here (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/error.rs` — `StorageError::TooLarge` is the conversion target (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `keeplin-core/src/collab/state.rs` — `diff_body` calls `check_body` before mutating (INFERRED)
- `keeplin-core/src/collab/mod.rs` — write paths call `check_body`; `handle_server_msg` calls `is_limit_code` (INFERRED)
- `keeplin-core/src/ordering.rs` — `place_new_note` calls `check_notebook_capacity` (INFERRED)
- `keeplin-daemon/src/rest.rs` — `create_note`/`update_note` call `check_body` (INFERRED)
- `keeplin-daemon/src/server.rs` — the gRPC `create_note`/`update_note` call `check_body` (INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- The three limits are exact powers of two and are defined exactly once, in this file.
- Line length is measured in UTF-8 bytes on both sides of the wire; line and note counts
  exclude tombstones.
- Exceeding a limit always produces an error the user can see; no caller may truncate.
- Any change here is a cross-repo change: keeplin-srv's pinned `rev` must be bumped and its
  cross-repo compatibility test re-run.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | `MAX_LINE_BYTES` | `// md:MAX_LINE_BYTES` |
| 3 | `MAX_LINES_PER_NOTE` | `// md:MAX_LINES_PER_NOTE` |
| 4 | `MAX_NOTES_PER_NOTEBOOK` | `// md:MAX_NOTES_PER_NOTEBOOK` |
| 5 | `CODE_LINE_TOO_LONG` | `// md:CODE_LINE_TOO_LONG` |
| 6 | `CODE_TOO_MANY_LINES` | `// md:CODE_TOO_MANY_LINES` |
| 7 | `CODE_NOTEBOOK_FULL` | `// md:CODE_NOTEBOOK_FULL` |
| 8 | `enum LimitViolation` | `// md:LimitViolation` |
| 9 | `impl LimitViolation` | `// md:impl LimitViolation` |
| 10 | `fn code` | `// md:impl LimitViolation > fn code` |
| 11 | `impl From<LimitViolation> for StorageError` | `// md:impl From LimitViolation for StorageError` |
| 12 | `fn is_limit_code` | `// md:fn is_limit_code` |
| 13 | `fn check_line` | `// md:fn check_line` |
| 14 | `fn check_line_count` | `// md:fn check_line_count` |
| 15 | `fn line_count` | `// md:fn line_count` |
| 16 | `fn check_body` | `// md:fn check_body` |
| 17 | `fn check_notebook_capacity` | `// md:fn check_notebook_capacity` |
| 18 | `mod tests` | `// md:mod tests` |
| 19 | `imports` | `// md:mod tests > imports` |
| 20 | `fn the_three_limits_are_exact_powers_of_two` | `// md:mod tests > fn the_three_limits_are_exact_powers_of_two` |
| 21 | `fn line_length_is_counted_in_utf8_bytes` | `// md:mod tests > fn line_length_is_counted_in_utf8_bytes` |
| 22 | `fn line_count_boundary_accepts_the_limit_and_rejects_one_more` | `// md:mod tests > fn line_count_boundary_accepts_the_limit_and_rejects_one_more` |
| 23 | `fn line_count_matches_the_materialised_body` | `// md:mod tests > fn line_count_matches_the_materialised_body` |
| 24 | `fn check_body_enforces_both_line_limits` | `// md:mod tests > fn check_body_enforces_both_line_limits` |
| 25 | `fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap` | `// md:mod tests > fn notebook_capacity_rejects_the_note_that_would_exceed_the_cap` |
| 26 | `fn every_violation_maps_to_its_wire_code_and_to_too_large` | `// md:mod tests > fn every_violation_maps_to_its_wire_code_and_to_too_large` |
