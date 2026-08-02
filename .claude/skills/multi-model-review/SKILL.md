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
copied here; `codex login status` must print `Logged in using ChatGPT`. An
OpenAI API key is *not* enough — a ChatGPT subscription does not fund the API,
which answers 429 on every model.

## Running a cycle

```bash
# 1. Who does what this cycle
node scripts/roles.js 1        # -> implementer kimi, reviewers qwen glm codex

# 2. Gather the change (free)
node scripts/collect.js /path/to/repo 197 work/pr197

# 3. Mirror the objective into work/pr197/meta.md using the GitHub MCP tools,
#    then assemble the prompt
node scripts/build-prompt.js work/pr197 docs/prompts/0.C-prompt-revision-seguridad.md work/pr197/meta.md

# 4. Ask each reviewer (nothing passes through your context)
node scripts/ask.js qwen  work/pr197/prompt.txt work/pr197/review-qwen.md
node scripts/ask.js glm   work/pr197/prompt.txt work/pr197/review-glm.md
node scripts/ask.js codex work/pr197/prompt.txt work/pr197/review-codex.md

# 5. Read only the verdicts
grep -m1 -h "VEREDICTO" work/pr197/review-*.md
```

Then hand the review files back to the implementer by path and start the next
cycle. Stop when every reviewer reports `SIN HALLAZGOS`, or when the maintainer
calls it.

## Size is the real constraint

The browser reviewers accept a large paste — `fill()` moves 100 KB in
milliseconds — but they do not necessarily *answer* one. A 104 KB prompt
produced no reply at all from GLM; the same review at 24 KB worked. Codex, going
over the API, handled the full 104 KB.

So keep the browser prompts small: prefer the diff alone, drop the whole-file
section (`build-prompt.js` already caps it), and split a large PR by area,
running the rotation per area. Give the oversized whole to Codex if you want one
reviewer with the complete picture.

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

## Recording the outcome

`AGENTS.md` wants the reviewing family recorded on the PR. The rotation is
derived from the cycle number precisely so it can be reconstructed later. When
the cycle closes, fill the PR's *Independent review* section with the families
that reviewed and a link or path to their findings — and do not tick those boxes
on the strength of your own re-reading of the diff.
