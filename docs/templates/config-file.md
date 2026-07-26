<!--
  TEMPLATE: companion doc for a non-Rust artifact — a build script, config file, schema, CI
  workflow, or shell script (`build.rs`, `keeplin.proto`, `config.toml`, `ci.yml`, `*.sh`).
  Named `<file>.md` next to it. Delete comments and unused sections before committing.
-->
# `{{path/to/file}}` — {{what it configures / generates}}

## Mechanical fidelity for supported non-Rust files

Use this section only for formats currently enforced by `scripts/companion_tool.py`:

- Shell (`*.sh`, plus extensionless files with a shell shebang) uses unique
  `# md:<name>` source markers. A block runs from its marker to the next marker or EOF.
  Its companion has exactly one `bash` fence whose first content line is the same marker.
  Only marker-led `bash` fences are fidelity blocks; ordinary illustrative `bash` fences
  remain prose. Before the first marker, only an optional UTF-8 BOM, a first-line shebang
  and blank lines are allowed.
- SQL migrations never receive markers and must never be edited to satisfy documentation:
  `sqlx::migrate!` validates applied-migration checksums. Each `*.sql` must have a sibling
  companion named by replacing `.sql` with `.md`, containing exactly one `sql` fence with
  the complete file verbatim. Comparison normalizes line endings to LF and nothing else;
  synchronization may rewrite only that companion fence and never the migration.

Unsupported formats are ignored rather than guessed. Python and Proto are tracked for a
later bounded migration; TOML, YAML, dotenv files and Dockerfiles remain descriptive
companions until explicitly added to the mechanical contract.

## Purpose

<!-- What this artifact is and what depends on it. When does it run / who reads it? -->
{{What the file does and who consumes it (the compiler, the daemon at startup, CI, an
operator). Note when it runs — build time, startup, in CI.}}

## What it {{generates | defines | runs}}

<!-- The concrete output or contract: the generated code, the schema's messages/RPCs, the
     config keys, the CI steps. Use a table or a fenced snippet copied from the file. -->
{{The concrete contract — generated types, message/RPC list, config keys with defaults, or
pipeline steps.}}

## Configuration / key reference

<!-- For config/schema files: the fields, with defaults and meaning. For a build script:
     the options passed. Delete the form that does not apply. -->
| Key / option | Default | Meaning |
|--------------|---------|---------|
| `{{key}}` | `{{default}}` | {{what it controls}} |

## Notes & gotchas

<!-- The non-obvious operational facts: required external tools, backward-compatibility
     rules (e.g. "never renumber a proto field"), ordering constraints, secrets handling. -->
- {{A required tool / environment fact (e.g. "needs `protoc` on PATH").}}
- {{A compatibility rule that must not be broken (e.g. "add new proto fields with new tags;
  never reuse or renumber; old peers ignore unknown fields").}}

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). This file is LAYER 2;
CI publishes LAYER 1 as `knowledge-graph-<commit SHA>`, and `graphify update .` creates the
same ignored `graphify-out/` layout locally. Download or generate the graph for this exact
commit before refreshing EXTRACTED relationships; local Graphify is never required to use
this companion.

<!-- Data source: CI artifact or local graphify-out/graph.json from this exact commit.
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. Never
     present inference as fact. -->
**Nodes/edges this file contributes**

- `{{Entity}}` — {{what it is; the relationships the graph holds for it, labelled EXTRACTED/INFERRED}}

**Direct dependencies** (what this file uses; one line each on what it is and why it matters here)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Direct dependents** (who breaks if this file changes; one line each)

- `{{path}}` — {{ONE-LINE summary}} (EXTRACTED|INFERRED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- {{invariant}}

## Related files

- `{{path}}` — {{one-line reason}}
