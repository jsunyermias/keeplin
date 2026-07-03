<!--
  TEMPLATE: a cross-cutting design / policy document that lives at the repository root
  (like ARCHITECTURE.md or SECURITY.md). Use for a concept that spans crates and that a
  reader should be able to understand standalone. Delete comments and unused sections
  before committing.
-->
# {{Title}} — {{one-line framing}}

<!-- One or two sentences telling the reader what this document is and how to read it
     relative to the others (e.g. "read this first"; "the threat model lives here"). Some
     repetition with per-file companions is fine — this page should stand alone. -->
{{What this document covers and how it relates to the other docs.}}

---

## {{1. The concept / the model}}

<!-- Establish the mental model before the details: the core idea, the invariant, the
     vocabulary the rest of the doc uses. -->
{{The central idea, stated once, clearly.}}

## {{2. How it works across the system}}

<!-- Walk the concept through the components it touches. A table comparing the two backends
     / the two modes / the trust boundaries scans well. -->
| {{axis}} | {{option A}} | {{option B}} |
|----------|--------------|--------------|
| {{property}} | {{…}} | {{…}} |

## {{3. Guarantees and non-guarantees}}

<!-- What the design promises and, just as important, what it explicitly does NOT. This is
     where a security doc puts its threat model and a sync doc puts its delivery semantics. -->
- **Guarantees:** {{what always holds}}
- **Does not guarantee:** {{what is out of scope, and the mitigation if any}}

## {{4. Operational implications}}

<!-- What an operator/user must do or know: setup requirements, backup guidance, footguns
     the software detects, configuration that changes the tradeoff. Delete if not a policy
     doc. -->
- {{A thing the operator must do, and what breaks if they don't.}}

## Trade-offs & rejected alternatives

<!-- The decisions that shaped this design and the roads not taken. -->
- {{Decision, and the alternative rejected, and why.}}

## Related documents

- `ARCHITECTURE.md` — the one-page mental model
- `{{path/to/companion.md}}` — {{the module that implements this}}
