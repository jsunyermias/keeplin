<!--
  TEMPLATE (v2.5.0, mechanically verified): companion doc for a source module (`foo.rs` -> `foo.md`,
  same directory). Use for every `.rs` file — library modules, binaries, build scripts and
  integration tests (`tests/*.rs`); for test files each `#[test] fn` is simply a block.
  This v2 supersedes the old table-based `source-module.md` / `test-file.md` format.

  WHAT CHANGED vs v2.4.0: the container preamble is now covered mechanically, and the
  v2.3 exemption for a test module's `use …;` imports is WITHDRAWN. Scaffolding is only
  the container's declaration, its attributes and its braces; anything else between a
  container marker and its first child marker — imports, consts, associated types,
  nested items — must carry its own marker and become a leaf block. The canonical case,
  `mod tests`' import preamble, is now the block `// md:mod tests > imports`.
  `sync-companion-code --check` reports the violation as UNCOVERED, so the previous
  wording did not merely under-specify the rule: it described code that CI now rejects.

  WHAT CHANGED vs v2.3.1: RULE 7 is now enforced by `sync-companion-code --check`.
  Every Rust fence maps to exactly one source leaf and must match it after LF/CRLF
  normalization only. Stale, truncated, orphan, reordered and duplicate fences fail CI.
  The same tool can synchronize existing fence bodies without changing authored prose.

  WHAT CHANGED vs v2.3: RULE 6 gains the CONTAINER PREAMBLE IS SCAFFOLDING paragraph —
  it declares (does not change) the convention the repo already follows: a container's
  own structural lines (the `impl`/`mod` declaration and its attributes, a test module's
  `use …;` imports, and closing braces) are scaffolding, not block content, and need no
  fence. No companion content changes are required by this amendment.

  WHAT CHANGED vs v2.2: the uncommented-code convention is now contractual (new
  RULE 9, mechanically enforced by check-docs.sh check 7): the .rs carries no
  comments other than `// md:` markers, so fences never contain doc comments.

  WHAT CHANGED vs v2.1: enforcement is now real, not aspirational. The hardened
  `scripts/check-docs.sh` mechanically verifies the marker/checklist/fence contract
  (the v2.1 text claimed this before it was true). Grouped Coverage-checklist rows are
  now explicitly forbidden (one row per block). RULE 7 clarified: the check is repo-wide.

  WHAT CHANGED vs v1: documentation alone is no longer enough. Every block section MUST
  embed the block's code COMPLETE AND VERBATIM in a ```rust fence (subsection **Code**).
  The goal: a small model given ONLY this .md can safely rewrite the paired .rs without
  ever opening it.

  ══════════════════════════════ HARD RULES ══════════════════════════════
  CI (`scripts/check-docs.sh`) mechanically verifies: the companion exists, it has a
  `## Graph context` section, every `// md:` marker in the .rs appears here, no marker
  is duplicated in the .rs, the Coverage checklist has exactly one row per marker, and
  no elision pattern appears inside any ```rust fence, the .rs carries no comment lines
  other than `// md:` markers, every Rust fence is exactly faithful to its source leaf,
  and no unmarked item hides in a container's preamble.
  Line endings are normalized to LF; indentation, blank lines, attributes, signatures and
  bodies are not normalized. A file that violates any rule is an INCOMPLETE migration, no
  matter how good the prose is.

  RULE 1 — COMPLETE CODE, ALWAYS. Every leaf-block section embeds the block's code
  complete and verbatim in a ```rust fence: character-for-character as it appears in
  the .rs, including the `// md:` marker comment, all attributes (`#[derive(...)]`,
  `#[serde(...)]`, `#[test]`, `#[tokio::test]`, …) and the full body. (No doc
  comments — RULE 9 banishes them from the .rs, so none appear here either.)

  RULE 2 — NO ELISION, EVER. Forbidden inside code fences: `// ...`, `/* ... */`,
  `// snip`, `// rest unchanged`, `// (as before)`, omitted function bodies, collapsed
  match arms, elided struct fields, "signature only". There are no exceptions — CI
  greps for these patterns. If the code feels too long to embed, the block is too big:
  split it in the .rs into smaller blocks, each with its own marker — never shorten
  the code instead.

  RULE 3 — SIGNATURE ≠ CODE. The signature quoted in **Identification** never
  substitutes the fence. A section without its **Code** fence is unfinished work.

  RULE 4 — 1:1:1 CORRESPONDENCE. Each block in the .rs has exactly one `// md:` marker,
  exactly one section here (header chain = marker path), and exactly one row in the
  Coverage checklist. Nothing in the .rs without a section; no section without code.
  NEVER group blocks into one checklist row (no `| 5–17 |` ranges) — CI counts rows
  against markers and fails on grouped rows.

  RULE 5 — WHAT A BLOCK IS. A block is a section of code that implements one function
  or feature: the imports group; a type definition (struct/enum/trait/type alias);
  a free function; a const/static; a small impl documented as a unit; one method of a
  large impl; one test function; or a small group of helpers that only make sense
  together (e.g. two tiny regex builders for one grammar). Blocks follow source order.

  RULE 6 — CONTAINERS vs LEAVES. An `impl` or `mod` (e.g. `mod tests`) whose members
  are documented individually is a CONTAINER: its section has NO code fence; its
  **Code** subsection reads "Members documented as sub-blocks below: …". An impl with
  ≤ 3 short methods MAY instead be one leaf block with the whole impl in a single
  fence. Never both (no duplicated code), never neither.

  CONTAINER PREAMBLE IS SCAFFOLDING — AND SCAFFOLDING IS ONLY THIS. A container's own
  structural lines are not block content and carry no fence: the `impl`/`mod`/`trait`
  declaration with its generics and `where` clause, its attributes (`#[cfg(test)]`,
  `#[async_trait]`, …), and the braces. That list is exhaustive and mechanically
  enforced. EVERY other line between a container's marker and its first child marker is
  real code and needs its own marker and leaf section: imports, consts and statics,
  associated types, type aliases, nested `mod`/`impl` blocks, helper functions.
  `sync-companion-code --check` reports anything else as UNCOVERED and CI fails.
  The `mod tests` import preamble is the canonical example of what this now REQUIRES:
  mark it `// md:mod tests > imports` and embed it verbatim, one leaf like any other.
  (v2.4.0 and earlier exempted those `use …;` lines; that exemption is withdrawn.)
  A container's own code is never duplicated: only a block's OWN code is embedded, and
  a block's own `where` clause, associated types, and every line of a multi-line `use`
  statement live inside that block's fence, character-for-character (RULE 1).

  RULE 7 — FIDELITY IS MECHANICAL. Run `scripts/sync-companion-code --check`: every
  leaf marker must own exactly one fence with identical content, while containers own no
  fence and no unmarked code in their preamble. The comparison normalizes CRLF/LF line
  endings and nothing else. The check detects stale, truncated, reordered, missing,
  orphan, duplicate and uncovered blocks. To repair ordinary
  drift, run `scripts/sync-companion-code path/to/file.rs`; it updates only existing fence
  bodies and leaves all authored prose intact. `scripts/check-docs.sh` runs this repo-wide;
  do not mark the migration done until it passes clean.

  RULE 8 — DEPENDENCIES ARE CONTRACTS. The **Dependencies** subsection is a bullet
  list, one bullet per dependency, in this shape:

      - `symbol` — {what it is used for here}; expects: {the behaviour/contract this
        block relies on}.

  Name the specific methods/functions used, not just the owning type. The "expects"
  half is the load-bearing part: it is what breaks if the dependency changes, and
  what a reader modifying the DEPENDENCY must preserve — the two directions
  (Dependencies / Used by) exist so a change on either side can be evaluated without
  opening other files. Note silent-failure risks explicitly (e.g. "if X stops doing
  Y this degrades quietly, no compile error"). `—` stays valid for pure delegation.
  **Used by** keeps the call-site + purpose form; the contract those callers rely on
  is this block's own **What it does** — do not duplicate it per caller.

  RULE 9 — THE CODE IS UNCOMMENTED. The .rs carries NO explanatory comments:
  no `//` lines, no `///` doc comments, no `//!` module docs, no `/* */` blocks,
  no trailing comments. The only comment lines allowed are `// md:` markers. All
  explanation lives in this companion — if the code needs a comment, write it
  here instead. CI (check 7) greps for stray comment lines and fails.

  Delete every HTML comment and every section that does not apply before committing.
-->

# `{{path/to/module.rs}}` — {{one-line purpose}}

Self-contained companion for `{{path/to/module.rs}}`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only
this file must be able to understand and modify the module without opening anything else,
so project-wide conventions are deliberately re-explained here (hyper-redundancy is
intended).

**How to navigate**: every block in the `.rs` carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section here;
grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
{{the file's full use block, copied exactly, marker comment included}}
```

**What it does** — {{one paragraph: what this module is responsible for, the key type(s)
it defines, and where it sits in the system — e.g. its position in the decorator stack,
or "intentionally I/O-free so it is unit-testable in isolation". Then the module's
load-bearing mechanism at narrative level: the grammar, the on-disk/on-wire layout, the
state machine — whatever a reader must hold in their head before the block sections
below make sense. Long algorithms get their own ## Design notes section instead (see
below); keep this to what orients, not what exhausts.}}

**Dependencies** — {{one bullet per crate imported above (HARD RULE 8): what it provides
here and what the module relies on — e.g. "thiserror — `#[from]` generates the
auto-conversions every caller's `?` depends on; removing an attribute silently drops a
conversion path".}}

**Used by** — {{the files/modules that import this one, with the symbols they use}}.

**Repeated context** — {{project-wide conventions this module relies on, restated even
if documented elsewhere: serde naming, error type, encryption-at-rest rule, compatibility
surfaces… "none" is a valid answer.}}

---

<!--
  BLOCK SECTION SKELETON — repeat per block, in source order.
  Top-level blocks use `##`; members of a container use `###` under the container's `##`.
  Separate sections with `---`.

  ┌──────────────────────── EXAMPLE OF A FINISHED SECTION ───────────────────────┐
  │ (from keeplin-core/src/links.rs — this is the expected end state; delete)    │
  │                                                                              │
  │ ### fn parse                                                                 │
  │                                                                              │
  │ **Identification** — `pub fn parse(segment: &str) -> Self`; marker           │
  │ `// md:impl Reference > fn parse`.                                           │
  │                                                                              │
  │ **Code** — complete and verbatim:                                            │
  │                                                                              │
  │ ```rust                                                                      │
  │ // md:impl Reference > fn parse                                              │
  │ pub fn parse(segment: &str) -> Self {                                        │
  │     match Uuid::parse_str(segment) {                                         │
  │         Ok(id) => Reference::Id(id),                                         │
  │         Err(_) => Reference::Alias(segment.to_string()),                     │
  │     }                                                                        │
  │ }                                                                            │
  │ ```                                                                          │
  │                                                                              │
  │ **What it does** — A valid UUID becomes `Reference::Id`, anything else       │
  │ `Reference::Alias`. Total — never fails.                                     │
  │                                                                              │
  │ **Dependencies** —                                                           │
  │ - `Uuid::parse_str` — classifies the segment; expects it to be total:        │
  │   `Err` (never a panic) is what drives the `Alias` branch.                   │
  │                                                                              │
  │ **Used by** — `parse_link_ref` (every notebook/note segment).                │
  │                                                                              │
  │ **Repeated context** — none.                                                 │
  └──────────────────────────────────────────────────────────────────────────────┘
-->

## {{Block name — type / fn / impl / mod, as in the source}}

**Identification** — {{what it is (enum deriving … / inherent impl / unit test / …) and
its full signature}}; marker `// md:{{marker path}}`.

**Code** — complete and verbatim:

```rust
{{THE ENTIRE BLOCK, character-for-character from the .rs: marker comment, attributes,
full body. See HARD RULES 1–3 — no elision of any kind.}}
```

**What it does** — {{the contract: inputs, outputs, error cases, side effects, edge cases
deliberately handled, invariants maintained. What would break if this were wrong.}}

**Dependencies** — {{one bullet per dependency (HARD RULE 8):
`- \`symbol\` — {what it is used for here}; expects: {the behaviour/contract this block
relies on, including silent-failure risks}`. Name the specific methods used, not just the
type. `—` for pure delegation.}}

**Used by** — {{who calls this and for what — name the call sites, including tests}}.

**Repeated context** — {{project-wide conventions relevant to THIS block, restated.
"none" is valid.}}

---

<!--
  OPTIONAL — include only when the module has a load-bearing algorithm, protocol, or
  data layout that needs more narrative than the Overview holds (examples in this repo:
  "The note model — per-device logs + version-vector merge", "Atomic write pattern",
  "Database schema"). Name it after the mechanism, not "Design notes". Fold invariants
  and rejected alternatives here.
-->

## {{Module-specific mechanism — optional}}

{{The algorithm/layout/state machine, its invariants, and the alternatives rejected
and why.}}

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `{{Symbol}}` — {{defined here / relationship held by the graph}} (EXTRACTED|INFERRED)

**Direct dependencies** (files this one's symbols reference)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Direct dependents** (files whose symbols reference this one)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated
elsewhere)

- {{invariant}}

---

## Coverage checklist

<!-- MANDATORY, machine-checked. One row per block in the .rs, in source order — never
     group blocks into a single row. `scripts/check-docs.sh` counts these rows against
     the `// md:` markers in the .rs and fails on any mismatch. Every row corresponds
     to exactly one marker in the source and exactly one section above. -->

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports (`use …`) | `// md:Overview` |
| 2 | {{block}} | `// md:{{marker path}}` |
