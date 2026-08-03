# 0010 — End-to-end encrypted collaborative editing: threat model and v1 ambition

- Status: proposed
- Date: 2026-07-27
- Decision owners: maintainer
- Scope: cross-repo
- Issue: [keeplin#143](https://github.com/jsunyermias/keeplin/issues/143)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none
- Superseded by: none

> **Draft prepared by Claude. `proposed` does not authorize implementation.** Every
> recommendation below is a recommendation. Four decisions are explicitly the maintainer's and
> are marked **DECISION** where they occur. The ADR also asks for a human security review of the
> design before any cryptographic core is written; that request is itself part of the proposal.

## Context and problem

Verified on `keeplin@d32bb04` and `keeplin-srv@1d58bd2`.

Collaborative editing today is **not** end-to-end encrypted. `keeplin-srv` receives line
operations in the clear, materializes them into its own projections, relays them to subscribers,
and stores the result. The server is therefore a full-content reader by construction, and
`SECURITY.md` describes it as such.

[keeplin#142](https://github.com/jsunyermias/keeplin/issues/142) proposes to make the server
blind: it would relay opaque operations it cannot read.
[keeplin-srv#72](https://github.com/jsunyermias/keeplin-srv/issues/72) is the server side of the
same change. Neither can start without a durable decision about what "blind" means, what the
system still guarantees when the server is hostile, and how ambitious the first version is —
which is what this ADR exists to settle.

A durable decision is needed because the cryptographic core is the part of this project where a
subtle mistake is both catastrophic and hard to reverse: ciphertext written under a broken scheme
stays broken on disk. The project also develops through rotating AI models, so a decision that
lives only in a pull request discussion will not survive the next context window.

Two facts constrain the answer and are easy to miss:

- **A pending decision already depends on this one.** [keeplin#154](https://github.com/jsunyermias/keeplin/issues/154) (`orden-17`) proposes encryption that can be disabled per note or per notebook. That is in direct tension with a blind server, and [keeplin#162](https://github.com/jsunyermias/keeplin/issues/162) records the conflict as unresolved and assigns its resolution here.
- **Server-side materialization is scheduled work.** [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) (`orden-07`) makes the server's projection of note content atomic and retryable. A blind server cannot project content it cannot read. These two pieces of planned work contradict each other and the contradiction is not currently written down anywhere.

## Forces and requirements

- **Confidentiality against the operator.** Self-hosted or not, the operator must not be able to read note content. This is the product reason the epic exists.
- **Convergence must survive opacity.** Concurrent edits must still converge without the server reading content.
- **The wire contract is shared and versioned.** `keeplin-core::collab::protocol` is the single source of truth; `keeplin-srv` imports it. `compatible_with` requires exact `PROTOCOL_VERSION` equality and `core_compat.rs` asserts both sides agree, so any format change here is a lockstep, hard-cut change.
- **No hand-rolled cryptography.** Vetted primitives and vetted implementations only.
- **The project has no cryptographer.** Rotating models implement; review independence is procedural, not expert. The design must be conservative enough to survive that.
- **Recovery must remain possible.** A user who loses a device must not lose their notes; a scheme with no recovery path is not shippable even if it is cryptographically ideal.
- **Pre-release freedom.** There is no installed base to migrate, so a clean break is available now and will not be later. This argues for deciding the format before first release, not after.

## Threat model

### Assets

Note and line content; note titles and structure where they carry meaning; the content
encryption keys; device identity keys; the membership list of a shared note.

### Trust boundaries

The client (`keeplin-core` / `keeplin-daemon`) is trusted. The transport is untrusted. The
server (`keeplin-srv`) is **untrusted for confidentiality** and **partially trusted for
availability**: it is the only thing that can deliver operations, so it can always refuse to.

### Adversaries and what is claimed

| Adversary | Capability | Claim |
|---|---|---|
| Passive server or operator | Reads everything stored and relayed | **Cannot recover content.** Primary goal. |
| Network attacker | Reads and modifies transport | Subsumed by the above; TLS remains, but the guarantee does not rest on it. |
| Active server | Forges, reorders, drops, replays operations | Forgery and cross-context replay are **detected and rejected** (AEAD + signature + bound associated data). Reordering within a note is **detected** through version vectors. Dropping is **not prevented** — see below. |
| Compromised participant device | Holds valid keys | Out of scope. A participant can always leak what they can read. |

### Accepted leakage — explicit, not incidental

The server necessarily learns: which accounts exist and which devices they own; that a note
exists and which devices subscribe to it; the number and approximate size of encrypted lines; the
timing and frequency of edits; the social graph implied by sharing. Content length leaks through
ciphertext length unless padded, and **v1 does not pad**.

### Non-goals

Availability against a hostile server. Metadata privacy. Protection against a malicious
participant. Deniability. Post-compromise security in v1 (see the rotation decision below).

## Options considered

**A. Keep the current behavior.** The server reads content and keeps server-side features. Zero
work, zero risk of a crypto mistake, and the product promise of a private notes system is not
met. Rejected as a durable end state; it is the status quo the epic exists to replace.

**B. Static per-note content key, no rotation (recommended for v1).** One content encryption key
per note, wrapped to each participant device's public key. Adding a participant wraps the
existing key to them. Removing a participant removes their wrap, but they retain any key material
they already had, so **removal does not protect past or future content of that note until the
note is re-keyed**. Cheap, auditable, implementable with well-understood primitives. Its weakness
is exactly the revocation story, and that weakness collides with
[keeplin-srv#76](https://github.com/jsunyermias/keeplin-srv/issues/76) (`orden-10`), which makes
device revocation take effect immediately on live sockets: after this ADR, that issue would
enforce *access* revocation while *cryptographic* revocation stays incomplete. That gap must be
documented in `SECURITY.md` as a non-guarantee, not glossed.

**C. Group key agreement with rotation from day one (MLS).** Solves revocation and gives
post-compromise security. Costs: a much larger protocol surface, a dependency on an MLS
implementation, group-state management across devices and reconnections, and a design that this
project cannot currently review to the standard it deserves. Recommended as v2, with the v1
format designed so it does not have to be undone.

**D. Encrypt at rest on the server with server-held keys.** Protects against disk theft and
nothing else. It does not meet the stated goal and is easy to mistake for it; recorded here so
that mistake is on the record as considered and rejected.

## Decision and justification

Stated as recommendations. **`proposed` does not authorize implementation.**

1. **The primary guarantee is confidentiality against a passive server**, with integrity and
   authenticity against an active one, and **no availability guarantee**. A hostile server can
   always withhold operations; clients detect the gap through version vectors and surface it,
   rather than silently converging on a truncated history.

2. **DECISION — v1 ambition.** Recommend option **B**: static per-note content key, no rotation,
   with the revocation limit written into `SECURITY.md` as an explicit non-guarantee, and MLS
   deferred to v2. The alternative is C, which blocks the epic behind a dependency the project
   cannot yet review.

3. **Key schema.** A random content encryption key per note. Per-device identity keypair
   generated on the device; the private key never leaves it and is never sent to the server. The
   content key is wrapped to each participant device's public key; the server stores only opaque
   wraps. **DECISION — recovery**: with no key escrow, losing every device holding a note's key
   loses the note irrecoverably. Recommend an explicit user-held recovery key at account
   creation, since the alternative is data loss reported as a bug.

4. **Primitives.** libsodium through a vetted Rust binding: XChaCha20-Poly1305 for content AEAD,
   sealed boxes for wrapping, Ed25519 for operation signatures. No primitive is implemented in
   this repository. For v2, an existing MLS implementation rather than a bespoke group scheme.

5. **Operation format.** Each line operation carries ciphertext, nonce, and a signature by the
   authoring device. The associated data binds note id, line id, operation sequence and device
   id, so the server cannot move a valid operation to another note, another line, or another
   position without detection. Types live in `keeplin-core::collab::protocol`; `keeplin-srv`
   imports them. This is a breaking wire change and bumps `PROTOCOL_VERSION` in lockstep.

6. **Server features given up.** Server-side materialization of content, server-side search, and
   any server-side re-encryption. **This directly contradicts `orden-07`
   (`keeplin-srv#75`)**, which is scheduled to make that materialization atomic. Recommend that
   `orden-07` proceed as scoped — the sync journal still needs atomic application, and metadata
   projection survives — but that its content-projection surface be marked as removed on E2EE
   arrival, so the work is not built twice.

7. **Convergence.** LWW per field with tiebreak `(timestamp, device_id)`, version vectors, and
   line-id ordering read no content, so they survive opaque operations unchanged. Two points do
   not, and both are server-side: content materialization (item 6) and any conflict resolution
   the server performs on content. The client-supplied `timestamp` remains the LWW input; signing
   it prevents a hostile *server* from altering it, and does not prevent a malicious
   *participant* from choosing it. That is inherent to LWW and is a non-goal, not a defect.

8. **DECISION — granular encryption (resolves the conflict in keeplin#162).** Recommend that
   `keeplin#154` be reframed: encryption is a per-note property fixed at creation and visible in
   metadata, not a switch that can be flipped on an existing note. A note created unencrypted can
   use server-side features; an encrypted note cannot, ever. Flipping an encrypted note to
   plaintext would require the client to re-upload content the server was never supposed to see,
   and flipping the other way leaves plaintext history on the server. Either the eight-case
   cross-device matrix in `keeplin#154 §3.4` is redesigned on that basis, or this ADR's blind
   server is not what gets built.

## Consequences and risks

Server-side search over encrypted notes disappears and no client-side replacement exists yet;
that is a product regression to plan for, not a detail. Revocation is incomplete until v2, and
`orden-10` will make that gap look closed when it is not. Without a recovery key, device loss is
data loss. Ciphertext length leaks content length. A v2 migration to MLS will have to re-key
existing notes, so v1 must at minimum version its key schema so that re-keying is expressible.

Residual risk that no process here removes: this design has been reviewed by no cryptographer.
**Recommend an explicit human security review of this ADR before any cryptographic code is
written**, and treating that review as a merge condition of the first implementation issue rather
than a nice-to-have.

## Compatibility, migration, and rollback

The operation format is a shared wire surface, so this is a hard cut: `PROTOCOL_VERSION` bumps on
both sides in the same pair of pull requests, `core_compat.rs` covers every new message in both
directions, and `keeplin-srv` moves to a new immutable `keeplin-core` pin. Partially upgraded
deployments do not interoperate by design; `compatible_with` requires exact equality and refuses
the connection, which is the correct failure.

Pre-release, no stored data needs migrating, which is the strongest practical argument for
deciding the format now. Rollback after ciphertext exists is not a code revert: content written
under this scheme is unreadable to an older client. Any rollback plan is therefore a data
decision, not a deployment one, and must be stated before the first release that writes
ciphertext.

Sequencing against the roadmap: this ADR is `orden-16`, and it must be accepted before `orden-25`
(`keeplin#142`) and `orden-26` (`keeplin-srv#72`). It should also land after `orden-09`, which
already bumps `PROTOCOL_VERSION` once; two hard cuts are worse than one, and combining them is
worth evaluating explicitly.

## Verification plan

- Convergence: concurrent edits from three devices converge to identical plaintext with the
  server holding only ciphertext, including reconnection and out-of-order delivery.
- Server blindness: an integration test asserting the server's stored rows and relayed frames
  contain no plaintext for an encrypted note, and that no code path materializes its content.
- Active adversary: an operation replayed into a different note, a different line, or a different
  position is rejected; a forged signature is rejected; a dropped operation is detected by the
  receiving client rather than silently accepted.
- Cross-repo: `core_compat.rs` round-trips every new message against the real `keeplin-core`
  types in both directions, and asserts the lockstep `PROTOCOL_VERSION`.
- Key handling: wrapping and unwrapping across device addition and removal, and an explicit test
  asserting the documented revocation limit rather than a wished-for one.
- Recovery: restoring a note on a fresh device from the recovery key, and the negative case where
  no key exists.
- Failure injection: the drills of `keeplin-srv#79` (`orden-15`) extended to the encrypted path.

## Equivalent decision in the other repository

Canonical here, in `keeplin`, because `keeplin-core` owns the shared model, wire and format
contracts. `keeplin-srv` receives a linking record in its own `docs/adr/` that points at this file
and states the server-side consequences: no content materialization, no server-side search, opaque
relay only. The two ADRs link each other rather than duplicating the decision. Implementation
lands as a paired set of pull requests with a single `PROTOCOL_VERSION` bump and a new immutable
`keeplin-core` pin on the server side.

## Sub-issues this ADR would unlock

Listed so `keeplin#142` can be split once this is accepted, per that issue's acceptance criteria:
device identity and key storage on the client; content-key wrapping and sharing; encrypted
operation format in `keeplin-core::collab::protocol`; the server's opaque relay path
(`keeplin-srv#72`); protocol bump and contract tests; recovery key; and the `SECURITY.md` rewrite
that states the real guarantees, including the non-guarantees named above.
