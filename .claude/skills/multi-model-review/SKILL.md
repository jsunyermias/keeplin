---
name: multi-model-review
description: Run a pull request through independent reviewers from several model families — Qwen and GLM always, plus whichever of Kimi or Codex did not implement it — moving diffs and reviews as files so almost none of it passes through the orchestrating agent's context. Use this whenever a Keeplin pull request needs the independent review AGENTS.md requires, when the user asks for a second or third opinion on a diff, mentions a review rotation or ping-pong between implementer and reviewers, or asks who should review a change.
---

# Multi-model pull request review

Drive a review cycle across four model families, keeping the orchestrator out of
the data path.

## Why it is built this way

`AGENTS.md` requires that a change be reviewed by a family other than the one
that implemented it. This skill makes that mechanical: two reviewers are fixed
(Qwen, GLM) and the third alternates with the implementer, so whoever wrote the
change never sits in judgement of it.

The order matters. Qwen and GLM run in parallel and blind to each other — two
independent readings, not one plus an echo. The rotating third runs afterwards
with both reviews attached and adjudicates: it confirms, refutes or declines
each prior finding against the diff, resolves contradictions, and adds what
both missed. Prior findings are input to it, never authority — the same rule
AGENTS.md applies to the author's own explanation.

The second constraint is cost. A review moves a whole diff and several long
replies; if those pass through the orchestrating agent they dominate its
context. So every hop is file-to-file, and the only thing ever read back is one
verdict line per reviewer.

| Step | Context cost |
| --- | --- |
| Collect diff and touched files | none — local `git` |
| Send to a reviewer, capture the reply | none — `ask.js` redirects to a file |
| Hand reviews back to the implementer | none — file paths, not contents |
| Decide whether the cycle continues | one line per reviewer |
| Mirror the PR body and issue | once per PR, via the GitHub MCP tools |

That last row is unavoidable: `api.github.com` is refused by the egress proxy in
these containers (403), so only the MCP tools reach GitHub. `git fetch` of
`refs/pull/<n>/head` does work, which is why the diff costs nothing.

## Prerequisites

A workspace holding `node_modules` (with `playwright`), the browser sessions,
and — for Codex — a ChatGPT login:

```bash
cd "$WORKSPACE"
npm init -y >/dev/null
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright @openai/codex
```

Sessions come from the sibling skills: `kimi-web-chat` (SMS login),
`glm-web-chat` and `qwen-web-chat` (cookie import). Codex uses
`~/.codex/auth.json`, written by `codex login` on a machine with a browser and
copied here; `codex login status` must print `Logged in using ChatGPT`.

An OpenAI API key is *not* an alternative: a ChatGPT subscription does not fund
the API, which answers 429 on every model. There is deliberately no API-key
driver in this skill — one existed and was removed, because it contradicted the
documented route and, unlike `codex exec`, ran the model without a read-only
sandbox against the very tree it was judging.

## Running a cycle

```bash
# 1. Gather the change (free). This comes first: --dir below must exist, and
#    the package it writes is where the assignment lives.
node scripts/collect.js /path/to/repo 197 work/pr197

# 2. Who does what. The implementer is fixed for the whole pull request; only
#    the pull request number decides it, so it can be reconstructed later.
#    --dir is required — without it the assignment would follow the operator's
#    current directory instead of the review, and two runs from two places
#    would produce two different implementers for the same pull request.
#
#    NAME THE IMPLEMENTER. The default is a guess from the pull request number,
#    and a guess that is wrong records a false implementer — which is how this
#    pull request ended up reviewed with the independence check switched off.
#    Any name works, in or out of the rotation:
node scripts/roles.js 4242 kimi   --dir work/pr4242   # -> adjudicator codex
node scripts/roles.js 4242 claude --dir work/pr4242   # outside the rotation:
                                                      #    both seats are free,
                                                      #    parity picks one

# 3. Mirror the objective into work/pr197/meta.md using the GitHub MCP tools,
#    then assemble the prompt
node scripts/build-prompt.js work/pr197 docs/prompts/0.C-prompt-revision-seguridad.md work/pr197/meta.md

# 4. The two blind reviewers. Run them on the same prompt; neither sees the
#    other, so their findings are two readings rather than one and an echo.
#    --pr binds each reply to this pull request, so a roles file from another
#    one cannot be used to close this cycle.
node scripts/ask.js qwen work/pr197/prompt.txt work/pr197/review-qwen.md --pr 197
node scripts/ask.js glm  work/pr197/prompt.txt work/pr197/review-glm.md  --pr 197

# 5. The adjudicator, with both prior reviews attached
node scripts/build-prompt.js work/pr197 docs/prompts/0.C-prompt-revision-seguridad.md \
  work/pr197/meta.md --prior work/pr197/review-qwen.md work/pr197/review-glm.md \
  --out prompt-adjudicator.txt
# roles.js said adjudicator: kimi for this pull request. Use what it returned,
# never a name copied from an example — sending this to the implementer would
# hand it its own work to judge. The gate checks this in step 7, but only if
# you pass it --roles, so the name here still has to be right.
node scripts/ask.js kimi work/pr197/prompt-adjudicator.txt work/pr197/review-kimi.md --pr 197

# 6. Read only the verdicts. One line each; never open the files to decide.
grep -m1 -h "VEREDICTO" work/pr197/review-qwen.md work/pr197/review-glm.md \
  work/pr197/review-kimi.md

# 7. The gate decides whether the cycle closes. This step is not optional:
#    "everyone said SIN HALLAZGOS" is not the criterion, the gate is.
#    --roles is not optional either. Without it the gate never checks who
#    replied, so a clean verdict from the implementer closes the cycle — the
#    protection exists, and omitting the flag is what switches it off.
node scripts/gate.js work/pr197/review-kimi.md \
  --roles work/pr197/roles-pr197.json \
  --cycle 1 --max-cycles 5
```

`gate.js` is the only thing that ends a review, and its exit code is the whole
contract:

| Exit | Meaning | What to do |
| --- | --- | --- |
| 0 | Cleared: verdict `SIN HALLAZGOS` **and** the phrase as the sole final line | Hand the pull request to the final reviewer |
| 1 | Not cleared | Send the review files back to the implementer by path and run another cycle |
| 2 | Cycle cap reached, or misuse | Stop and involve the maintainer rather than looping |

Both signals are required because either alone is forgeable. The phrase appears
in this repository's own diff and in the prior reviews attached to the
adjudicator's prompt, and these UIs echo prompts back into replies — so it is
required exactly once in the whole reply and as the last non-empty line.
Anything else is a quotation, and the gate says so. This is not hypothetical:
the first real run produced `VEREDICTO: BLOQUEANTE` with the phrase present, and
a phrase-only gate would have advanced a blocked change.

## Size is the real constraint

The browser reviewers accept a large paste — `fill()` moves 100 KB in
milliseconds — but they do not necessarily *answer* one. A 104 KB prompt
produced no reply at all from GLM; the same review at 24 KB worked. Codex, going
over the API, handled the full 104 KB.

### The adjudicator needs the model with the most headroom

Its prompt is the largest of all — the change, the project contract and both
blind reviews. Kimi was asked to adjudicate at 95 KB and again at 99 KB with the
poll ceiling raised to 300, and both replies were cut off before the verdict
line. That is a capacity limit, not a flake: the reasoning it emits ahead of the
answer grows with the prompt until the answer never arrives. Codex, going over
the API, adjudicated 117 KB without trouble.

So give adjudication to Codex whenever the assembled prompt is over roughly
70 KB, and use a browser family for the blind reviews, which are smaller. When
the pull request's implementer *is* Codex, split the review by area instead —
never hand it its own work. `roles.js` decides who may review; the size decides
whether that reviewer can, and the two have to agree before the cycle starts.

The gate catches this either way: a truncated reply has no verdict line and is
reported as a failed reviewer, never as a clean review.

So keep the browser prompts small. Drop the whole-file section with
`--no-files` — the byte cap alone will not do it, and when a change rewrites
what it touches the diff and the post-change text are nearly the same bytes
twice:

```bash
node scripts/build-prompt.js work/pr197 <checklist> work/pr197/meta.md --no-files
```

And split a large PR by area, running the rotation per area:

```bash
node scripts/collect.js /path/to/repo 197 work/pr197-mmr \
  --area .claude/skills/multi-model-review
```

Everything downstream treats that like any other package, and the prompt tells
the reviewer its view is partial so it does not report the rest of the pull
request as missing. Do the split with `--area` rather than by hand: a hand-made
package once reached three reviewers with an empty `files/`, and nothing in the
pipeline noticed. `collect.info` records the scope, so a later reader can see
what each round did and did not look at — and a pull request is only reviewed
once every area has been.

Give the oversized whole to Codex if you want one reviewer with the complete
picture — unless Codex is this pull request's implementer, in which case that
would be self-review.

## Reading a reply is the fragile part

These UIs stream into a page rather than returning a value, so "the reply" has
to be carved out of the rendered conversation. Three failure modes were found
the hard way, and the drivers now handle each — but they are what to suspect
when output looks wrong:

- **Stopping before the model starts.** The quiet-window heuristic used to begin
  counting immediately, so a model that paused to think looked finished before
  emitting a token. `waitForReply` now waits for generation to *begin*, then for
  it to settle, and reports `started: false` rather than an empty reply.
- **Stopping mid-generation.** A visible `Thinking...` indicator keeps the page
  text static while tokens are still coming. It is now treated as busy.
- **Echoed prompt swallowing the reply.** The reply is anchored on a plain-text
  sentinel the driver appends, compared after trimming, because the UI pads the
  echoed line. Z.ai additionally collapses long messages behind
  `Show full message`, so the echo on the page is truncated mid-word.

Qwen's extraction is still imperfect: the verdict line comes through, but the
echoed prompt can trail it. Read the verdict, not the whole file, and open the
file only when a verdict warrants it.

## Chat models as implementers: use the clipboard, never the page

Codex implements natively — it is an agent with file access, so it edits the
tree and the orchestrator never sees the code. Kimi is a chat, and code has to
be carried out of it. **Read it from the clipboard, not from the page.**

Reading the rendered page does not work, and fails in a way that looks like
success. A unified diff loses blank context lines outright — deleted, not
blanked — so `git apply` rejects the hunk with nothing left to repair;
recounting the header and reducing required context both fail. Asking for whole
files is worse: a reply that should have carried a complete file came back
missing an entire function and a closing brace. `innerText` simply does not
contain every line of a long code block.

The copy icon each block renders solves it. `copyCodeBlocks` in `kimi-lib.js`
clicks it and reads `navigator.clipboard`, recovering the text byte for byte —
blank lines, braces and all. Two details make it reliable:

- The icon is not a `button` element and carries no accessible name, so it is
  reached from the card's own geometry, inset from its top-right corner.
- The clipboard is seeded with a sentinel before each click. An unchanged
  clipboard means the click missed, and that block is skipped rather than
  reported with the previous block's contents.

The context must hold `clipboard-read` and `clipboard-write`; `CONTEXT` in
`kimi-lib.js` already does.

```bash
node scripts/kimi.js @implement-prompt.txt K3 --code > reply.out
```

Ask for the complete file in a single code block. Exercised end to end: Kimi
produced a file, it was written without being read, and the result ran
correctly.

Completion is detected the same way, from the page rather than by inference:
generation is in progress exactly while the composer offers a stop control, so
`waitForGeneration` waits on that instead of on the text going quiet.

All three browser drivers now carry both, and each was verified end to end by
having it produce a file that was written without being read and then run. The
editors differ — Kimi renders plain `<pre>`, Z.ai uses CodeMirror, Qwen uses
Monaco — and the last two virtualise their rows, which is exactly why the page
cannot be read for code. Z.ai is the flakiest: roughly one run in two came back
with nothing copyable, so retry once before concluding anything.

## Verification

The guarantees above are covered by offline tests — no browser, no model:

```bash
node --test scripts/tests/contract.test.js
```

They pin the gate's behaviour on each outcome including the quoted-phrase and
repeated-phrase cases, that the adjudicator is never the implementer and that
the implementer does not change between cycles, that `apply-files.js` leaves a
Markdown file's inner fences intact and refuses to write outside the repository,
that `apply-patch.js` recovers a diff whose `@@` headers the model miscounted
and *refuses* one whose blank context lines the renderer deleted, and that
`collect.js` refuses when the project contract is missing. The two cases were
the wrong way round here until an adjudicator read the tests against the prose:
a lost context line is exactly what cannot be repaired mechanically, which is
why `apply-patch.js` stops rather than guessing.

## Recording the outcome

`AGENTS.md` wants the reviewing family recorded on the PR. The assignment is
derived from the pull request number, and holds for every cycle of that pull
request, precisely so it can be reconstructed later. When
the cycle closes, fill the PR's *Independent review* section with the families
that reviewed and a link or path to their findings — and do not tick those boxes
on the strength of your own re-reading of the diff.
