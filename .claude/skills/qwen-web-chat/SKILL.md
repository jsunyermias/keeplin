---
name: qwen-web-chat
description: Drive Qwen Studio (chat.qwen.ai, by Alibaba) from a headless Chromium to send prompts to Qwen3.8-Max-Preview or Qwen3.7-Max and read the replies back, authenticating by importing the user's browser cookies. Use this whenever the user wants to ask Qwen something, compare an answer against Qwen, reuse a Qwen subscription from a container, or mentions Qwen, Qwen3.8, Qwen3.7, Max-Preview, Tongyi, or Alibaba's chat — even when they don't say "browser" or "Playwright".
---

# Qwen Studio web chat

Send prompts to Qwen's flagship models by driving chat.qwen.ai in a headless
browser.

## There is no anonymous mode

The site renders a composer for signed-out visitors, but any send bounces to
`/auth`. A session is required for everything, so `qwen.js` fails fast when
`qwen-state.json` is missing rather than producing a confusing empty reply.

## Setup

The container ships Chromium but not the Playwright package. Never run
`playwright install` — the browsers are already on disk.

```bash
cd "$WORKDIR"
npm init -y >/dev/null
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright
```

Copy this skill's `scripts/` next to that `node_modules` and run from there;
the scripts read and write `qwen-state.json` relative to the working directory.

`scripts/browser.js` handles two environment quirks that otherwise cost an hour
of debugging — read its header comments before changing any launch flag.

## Getting a session

Qwen offers email + password, Google, and GitHub. Prefer **cookies**:

- **Cookies** — no password crosses the conversation, and nothing to defeat.
- **Email + password** — the form is a plain one, so this can work if the user
  insists. Say plainly that the password will sit in the transcript and should
  be rotated afterwards.
- **Google** — closed. Google detects the automated browser and rejects at
  `/v3/signin/rejected` right after the email, before any password.
- **GitHub** — closed. `github.com` is blocked by the egress policy in Claude
  Code containers: Chromium reports `ERR_CERT_AUTHORITY_INVALID` and curl gets a
  403 from the proxy. That is an organisation policy denial; report it rather
  than routing around it.

Ask the user to export cookies from a browser where they are already signed in
(Cookie-Editor → Export → JSON), then convert:

```bash
node scripts/import-cookies.js qwen-export.json qwen-state.json qwen.ai
```

The converter exists because extension exports and Playwright's `storageState`
disagree in ways that fail silently — `sameSite` spellings, float expiry
timestamps, session cookies. It reports how many cookies survived, which domains
they cover, and warns about already-expired ones. A session that loads but does
not authenticate usually means the export was taken while logged out.

Alibaba accounts often span several domains. If the import authenticates in the
browser but the site still bounces to `/auth`, re-export without the domain
filter so sibling domains are carried over too.

Tell the user those cookies are live credentials for their Alibaba account —
a broader blast radius than the chat alone — that they live in an ephemeral
container, and that signing out invalidates them.

## Sending prompts

```bash
node scripts/qwen.js "your prompt"                       # Qwen3.8-Max-Preview
node scripts/qwen.js "your prompt" Qwen3.7-Max
node scripts/qwen.js "your prompt" Qwen3.7-Plus
```

Models seen in the picker: `Qwen3.8-Max-Preview` (preview of the 3.8 flagship),
`Qwen3.7-Max` (text only, no vision), `Qwen3.7-Plus` (the default). The script
prints the model the header reports and aborts if it disagrees with what was
asked, so a silent fallback never gets attributed to the wrong model.

The UI language follows the browser locale, which `browser.js` pins to `es-ES`.
Set it to `en-US` if you would rather match selectors against English labels.

## Constraints worth stating up front

- **Each run starts a new chat.** No multi-turn continuity between invocations.
- **Completion is inferred, not signalled.** The driver stops when the page text
  holds still for ~6 seconds. Deep-thinking or image-generating runs will be cut
  short — pass a larger `quietChecks`/`maxPolls` to `waitForReply` and warn the
  user the wait will be minutes.
- **The session is not durable.** `qwen-state.json` holds live cookies in an
  ephemeral container; expect to re-import in a new session.
- **The account's data is visible.** Driving a logged-in browser exposes the
  user's own chat history in page text. The driver subtracts the pre-existing
  chrome to keep it out of the output, but screenshots still capture it.

## When something breaks

| Symptom | Cause |
| --- | --- |
| `ERR_CONNECTION_RESET` on every URL | Chromium's TLS 1.3 handshake dies at the egress proxy. `browser.js` caps at TLS 1.2. Never fix this by disabling certificate verification. |
| Nothing leaves the browser, but `curl` works | Chromium ignores `HTTPS_PROXY`; it must be passed to `chromium.launch({ proxy })`. |
| `session expired or cookies rejected` | The export was stale or taken while logged out. Re-export; consider dropping the domain filter. |
| `model selection did not take` | The picker moved or the label was renamed. Re-derive from `qwen-chat.png` before trusting any reply. |
