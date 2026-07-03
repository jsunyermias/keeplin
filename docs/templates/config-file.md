<!--
  TEMPLATE: companion doc for a non-Rust artifact — a build script, config file, schema, CI
  workflow, or shell script (`build.rs`, `keeplin.proto`, `config.toml`, `ci.yml`, `*.sh`).
  Named `<file>.md` next to it. Delete comments and unused sections before committing.
-->
# `{{path/to/file}}` — {{what it configures / generates}}

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

## Related files

- `{{path}}` — {{one-line reason}}
