---
name: kimi-web-chat
description: Drive the Kimi web chat (kimi.com, by Moonshot AI) from a headless Chromium so prompts can be sent to Kimi K3 and its replies read back, including the SMS login that authenticates the session. Use this whenever the user wants to ask Kimi something, compare an answer against Kimi or K3, log into kimi.com, reuse a Kimi subscription from a container, or mentions Kimi, K3, K3 Swarm, Kimi Code, or Moonshot's web chat — even when they don't say "browser" or "Playwright". Also use it when a Moonshot API key returns 429/insufficient balance or does not list a K3 model, since the web chat is the only route to K3.
---

# Kimi web chat

Send prompts to Kimi K3 by driving the real web app in a headless browser, using
the user's own logged-in subscription.

## When the browser is the right tool

Moonshot ships an HTTP API, and for anything programmatic it is the better
choice — streaming, sampling parameters, no UI to break. Reach for this skill
only when one of these is true:

- **The user needs K3 specifically.** The API has exposed `kimi-k2.6` and
  `kimi-k2.7-code`; K3 has been a web-product-only model. Check
  `GET https://api.moonshot.ai/v1/models` before assuming — if a K3 id appears
  there, use the API instead and skip all of this.
- **The user has a chat subscription but no API balance.** These bill
  separately. A valid key returning `exceeded_current_quota_error` means the
  subscription cannot fund API calls, and the browser is the way to use what
  they already pay for.

Say which route you are taking and why, so the user can redirect you cheaply.

## Setup

The container ships Chromium but not the Playwright package. Never run
`playwright install` — the browsers are already on disk.

```bash
cd "$WORKDIR"
npm init -y >/dev/null
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright
```

Copy this skill's `scripts/` next to that `node_modules`, and run everything
from that directory: the scripts read and write `kimi-state.json`, `otp.txt`,
and `login.log` relative to the working directory.

`scripts/kimi-lib.js` handles two environment quirks that otherwise cost an hour
of debugging — read its header comments before changing any launch flag.

## Logging in

The login needs a human in the loop, so it spans two turns: one to request the
code, one to deliver it. Plan for that rather than trying to do it in a single
tool call.

**Only SMS works.** Google OAuth is a dead end — Google detects the automated
browser and rejects at `/v3/signin/rejected` right after the email is submitted,
before any password. Do not ask the user for Google credentials; no credential
gets past that wall. If SMS is also blocked, fall back to asking the user to
export cookies from a browser where they are already signed in, and load that
JSON as Playwright `storageState`.

**1. Ask for the phone number** with its country code, and the country's name as
it appears in the site's picker (`Spain`, not `España` — the list is English).

**2. Launch in the background.** The script blocks up to 15 minutes waiting for
the code; a foreground call would hang the turn.

```bash
rm -f login.log otp.txt
node scripts/login-sms.js "+34" "612345678" "Spain"   # run_in_background
```

**3. Wait for the send to resolve**, then confirm it actually went out. The log
records `countdown_visible=true` when the Send button turns into a countdown,
which is the site's own confirmation. `false` means the number never took or a
captcha appeared — read `sms-2-after-send.png` before telling the user anything.

```bash
until grep -qE "after Send|ERROR" login.log 2>/dev/null; do sleep 2; done
cat login.log
```

**4. Ask the user for the code**, then hand it over. Write atomically so the
poller never reads a half-written file:

```bash
printf '123456' > otp.tmp && mv otp.tmp otp.txt
```

**5. Confirm.** Look for `logged in: true` and `saved session to
kimi-state.json`. On failure, `sms-3-after-login.png` shows what the site said.

Tell the user the SMS originates from a datacenter IP, so a "new location"
warning from Kimi is expected and not an intruder.

## Sending prompts

```bash
node scripts/kimi.js "your prompt"            # K3 by default
node scripts/kimi.js "your prompt" Instant    # fast, shallow
node scripts/kimi.js "your prompt" "K3 Swarm" # batch/search agent
```

The site always reopens on **Instant**, so the script selects the model on every
run and prints what it ended up on (`model: K3 High`). If that line reads
`UNKNOWN`, the picker moved — do not present the reply as coming from K3 until
you have re-checked, because a silently-downgraded model is worse than an error.

Output is the transcript minus the page chrome that was present before sending.
It still contains Kimi's **`Think` block** — its internal reasoning — ahead of
the final answer. Separate them when quoting Kimi to the user; presenting
reasoning as the answer misrepresents it.

## Constraints worth stating up front

Mention these when they matter to what the user is asking for, rather than
letting them discover the limit mid-task:

- **Each run starts a new chat.** There is no multi-turn continuity between
  invocations. A real back-and-forth needs the script extended to reopen the
  previous chat URL instead of starting from the home page.
- **Completion is inferred, not signalled.** The script stops when the page text
  stops changing for ~6 seconds. That fits ordinary chat. Deep Research and
  K3 Swarm think far longer and will be cut off — raise `QUIET_CHECKS` and
  `MAX_POLLS` in `kimi.js`, and warn the user the wait will be minutes.
- **The session is not durable.** `kimi-state.json` holds live session cookies
  and lives in an ephemeral container. It grants full account access while it
  exists; treat it as a credential and expect to redo the SMS in a new session.
- **The account's data is visible.** Driving a logged-in browser exposes the
  user's own chat history and account name in page text. The script subtracts
  the pre-existing chrome to keep it out of the output, but screenshots still
  capture it.

## When something breaks

| Symptom | Cause |
| --- | --- |
| `ERR_CONNECTION_RESET` on every URL | Chromium's TLS 1.3 handshake dies at the egress proxy. `kimi-lib.js` caps at TLS 1.2. Never fix this by disabling certificate verification. |
| Nothing leaves the browser, but `curl` works | Chromium ignores `HTTPS_PROXY`; it must be passed to `chromium.launch({ proxy })`. |
| `no chromium under /opt/pw-browsers` | The build directory is versioned. The resolver globs for it; set `CHROMIUM_PATH` to override. |
| SMS never arrives | The country reverted to `+1`. The code is a searchable dropdown, and typing in it clears the phone field, so country goes first — `login-sms.js` verifies the number before sending. |
| Reply is the login wall | Session expired or the container recycled. Redo the SMS login. |

If a slider captcha appears after Send (the form carries a hidden
`NECaptchaValidate` field), the SMS route is closed for that attempt. Do not try
to defeat it — switch to the cookie-export fallback and tell the user why.
