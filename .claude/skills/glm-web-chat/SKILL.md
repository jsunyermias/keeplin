---
name: glm-web-chat
description: Drive the Z.ai web chat (chat.z.ai, by Zhipu AI) from a headless Chromium to send prompts to GLM-5.2 and read the replies back, authenticating by importing the user's browser cookies. Use this whenever the user wants to ask GLM or Z.ai something, compare an answer against GLM, reuse a Z.ai subscription from a container, or mentions GLM-5.2, GLM-5.1, GLM-5-Turbo, GLM-4.7, Zhipu, or z.ai — even when they don't say "browser" or "Playwright". GLM-4.7 works with no account at all, so reach for this even when the user has no Z.ai login.
---

# Z.ai web chat (GLM)

Send prompts to GLM by driving chat.z.ai in a headless browser.

## What works without an account

Anonymous chat works — no login, no cookies, nothing to ask the user for. But
the picker's GLM-5 entries are **disabled** for anonymous visitors; only
GLM-4.7 is selectable. If GLM-4.7 answers the user's question, take that path
and skip the whole auth conversation.

`glm.js` refuses rather than downgrading when a gated model is requested without
a session, because a silent downgrade would mean attributing GLM-4.7's answer to
GLM-5.2.

## Setup

The container ships Chromium but not the Playwright package. Never run
`playwright install` — the browsers are already on disk.

```bash
cd "$WORKDIR"
npm init -y >/dev/null
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright
```

Copy this skill's `scripts/` next to that `node_modules` and run from there;
the scripts read and write `zai-state.json` relative to the working directory.

`scripts/browser.js` handles two environment quirks that otherwise cost an hour
of debugging — read its header comments before changing any launch flag.

## Getting a session for GLM-5.2

**Cookies are the only workable route.** The alternatives are closed:

- **Google** — Google detects the automated browser and rejects at
  `/v3/signin/rejected` right after the email, before any password. No
  credential gets past it; do not ask the user for one.
- **GitHub** — `github.com` is blocked by the egress policy in Claude Code
  containers: Chromium reports `ERR_CERT_AUTHORITY_INVALID` and curl gets a 403
  from the proxy. This is an organisation policy denial. Report it; do not try
  to route around it.
- **Email + password** — the form carries a "Click to start verification"
  challenge. Expect a slider captcha, and do not attempt to defeat it.

Ask the user to export cookies from a browser where they are already signed in
(Cookie-Editor → Export → JSON), then convert:

```bash
node scripts/import-cookies.js zai-export.json zai-state.json z.ai
```

The converter exists because extension exports and Playwright's `storageState`
disagree in ways that fail silently — `sameSite` spellings, float expiry
timestamps, session cookies. It reports how many cookies survived, which domains
they cover, and warns about already-expired ones. A session that loads but does
not authenticate usually means the export was taken while logged out.

Tell the user those cookies are live credentials for their Z.ai account, that
they live in an ephemeral container, and that signing out of Z.ai invalidates
them.

## Sending prompts

```bash
node scripts/glm.js "your prompt"                  # GLM-5.2, needs a session
node scripts/glm.js "your prompt" GLM-4.7          # works anonymously
node scripts/glm.js "your prompt" GLM-5-Turbo
```

Models seen in the picker: `GLM-5.2` (flagship, "NEW"), `GLM-5.1`,
`GLM-5-Turbo`, `GLM-5V-Turbo` (vision), `GLM-4.7` (classic). The script prints
the model the top bar reports and aborts if it disagrees with what was asked.

Output is the transcript minus the page chrome present before sending. It
includes a **`Thought Process`** block ahead of the answer — separate it when
quoting GLM to the user, since presenting reasoning as the answer misrepresents
it.

## Constraints worth stating up front

- **Each run starts a new chat.** No multi-turn continuity between invocations.
- **Completion is inferred, not signalled.** The driver stops when the page text
  holds still for ~6 seconds. Long agentic or coding runs will be cut short —
  pass a larger `quietChecks`/`maxPolls` to `waitForReply` and warn the user the
  wait will be minutes.
- **The session is not durable.** `zai-state.json` holds live cookies in an
  ephemeral container; expect to re-import in a new session.
- **Signed-in and signed-out are different pages.** Signing in adds a sidebar
  that shifts the layout, turns the composer into a real `textarea`, and can
  open a release-announcement modal that swallows every click beneath it. The
  driver dismisses overlays first and locates controls by their own geometry
  rather than fixed coordinates, so both layouts work — but a screenshot is the
  fastest way to see which one you are on when something misbehaves.

## When something breaks

| Symptom | Cause |
| --- | --- |
| `ERR_CONNECTION_RESET` on every URL | Chromium's TLS 1.3 handshake dies at the egress proxy. `browser.js` caps at TLS 1.2. Never fix this by disabling certificate verification. |
| Nothing leaves the browser, but `curl` works | Chromium ignores `HTTPS_PROXY`; it must be passed to `chromium.launch({ proxy })`. |
| `not signed in — GLM-5.2 is gated` | Working as intended: import cookies, or fall back to GLM-4.7. |
| `"GLM-5.2" not selectable` | Cookies loaded but the account lacks access, or the entry was renamed. Check `zai-chat.png`. |
| `no model label found` | The names in `KNOWN` are stale. Match them exactly — Playwright normalises whitespace for exact strings but not for regexes, so a regex over these padded labels finds nothing. |
| `model selection did not take` | A modal reopened over the picker, or the top bar changed. Check `zai-chat.png` before trusting any reply. |
