# Documentation templates

Keeplin keeps its prose documentation **next to the code it describes**, in Markdown, so a
reader can open any file and find a companion that explains the *why* the source comments
cannot. This directory holds the templates for each kind of document, plus the conventions
that keep them consistent.

## The convention in one sentence

**Every source file `foo.rs` has a companion `foo.md` in the same directory**, and every
top-level concern (architecture, security) has a document at the repository root.

When you add a source file, add its companion. When you change what a file *does* (not just
how), update its companion in the same change — a stale companion is worse than none.

## Which template to use

| You are documenting… | Template | Lives at |
|----------------------|----------|----------|
| Any `.rs` file with logic — library/daemon module, binary, build script, **or an integration/unit test file** (`tests/*.rs`; each `#[test] fn` is a block) | [`source-module.md`](source-module.md) (v2.5.0, mechanically verified) | next to the `.rs`, same basename |
| A crate root (`lib.rs` / `main.rs` that mostly wires modules) | [`crate-root.md`](crate-root.md) | next to the root `.rs` |
| A build script, config, or schema (`build.rs`, `*.toml`, `*.proto`, `*.yml`, `*.sh`) | [`config-file.md`](config-file.md) | `<file>.md` next to it |
| A cross-cutting design or policy doc (architecture, security, threat model) | [`design-doc.md`](design-doc.md) | repository root |
| A high-risk architectural decision with alternatives and approval state | [`docs/adr/0000-template.md`](../adr/0000-template.md) | `docs/adr/NNNN-short-title.md` in the repository that owns the decision |

## House style

- **Explain the *why*, not the *what*.** The source already says what the code does; the
  companion says why it is shaped that way, what alternatives were rejected, and which
  invariants must not break.
- **Lead with a one-line title** matching the file: `` # `path/to/file.rs` — short purpose ``.
- **Open with `## Purpose`** (source/config) or `## What is tested` (tests): a short
  paragraph a newcomer can read in ten seconds.
- **Use tables** for method/field/variant inventories and test-case matrices — they scan far
  better than prose lists.
- **Fence real snippets** (`rust`, `sql`, `toml`, …) when a signature or SQL statement makes
  the point faster than a sentence. Keep them short and copied faithfully from the source.
- **Include `## Graph context`** (mandatory, CI-enforced): the file's nodes/edges in a
  Graphify graph generated from the same commit (a CI artifact or ignored local output), its direct
  dependencies and dependents each with a **one-line inline summary**, and the file's
  invariants restated. Label every relationship `EXTRACTED` (mechanically from the
  graph/AST) or `INFERRED` (your conclusion) — never present inference as fact.
- **Close with `## Related files`**: the handful of files a reader will jump to next, each
  with a one-line reason.
- **Keep it current.** Prefer describing behaviour and invariants (which age slowly) over
  line numbers or exact code (which age fast). After large refactors generate or download
  the same-commit graph and refresh affected `## Graph context` sections without committing
  `graphify-out/`.
- **Verify embedded code mechanically.** Run `scripts/sync-companion-code --check`; CRLF/LF
  is the only normalization, so every other character in each Rust fence must match source.
  A container's preamble may hold only its declaration, attributes and braces — imports,
  consts and nested items there need their own marker, or the check fails with UNCOVERED.
- **Build bounded context explicitly.** `scripts/context-pack <source> --list --profile
  understand` estimates the minimal pack; profiles add only manifest-declared context.
- **Be hyper self-contained; redundancy is intentional.** A small model given ONLY this
  `.md` must be able to work on the paired `.rs` safely, without reading any other file.
  Restate the invariants and summarise every referenced file inline — never "deduplicate"
  that redundancy away. Links to `ARCHITECTURE.md` / `SECURITY.md` are for *deeper*
  treatment, not a substitute for the inline summary.

## The two-layer navigation model

Agents working on this repo have two layers:

1. **LAYER 1 — discovery (the graph)**: `graphify query "<question>"` /
   `graphify path "A" "B"` / `graphify explain "X"` against a downloaded CI artifact or
   ignored local `graphify-out/graph.json` route you to the right files without reading the
   whole repo. If no graph is present, proceed directly to LAYER 2.
2. **LAYER 2 — work (these companions)**: once routed, read the companion `.md`, not the
   raw `.rs`, whenever possible; each companion is contractually self-contained for safe
   editing of its paired file.

## Placeholders in the templates

Text in `{{double braces}}` is a fill-in. Lines in `<!-- HTML comments -->` are guidance for
the author and must be deleted before committing. Delete any section that does not apply
rather than leaving it empty.
