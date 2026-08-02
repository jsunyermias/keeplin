// Shared browser setup.
//
// Two environment quirks are handled here rather than in the drivers, because
// getting either wrong fails far from its cause:
//
//   1. The Chromium build number changes between container images, so the path
//      is discovered rather than hardcoded.
//   2. Claude Code's egress proxy resets Chromium's TLS 1.3 handshake — the
//      CONNECT tunnel opens, then the socket dies mid-ClientHello, surfacing as
//      ERR_CONNECTION_RESET on *every* URL. Capping at TLS 1.2 avoids it.
//      Certificate verification stays on; never "fix" this by ignoring cert
//      errors.
const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const BROWSER_ROOT = '/opt/pw-browsers';

function resolveChromium() {
  if (process.env.CHROMIUM_PATH) return process.env.CHROMIUM_PATH;
  const found = [];
  for (const dir of fs.readdirSync(BROWSER_ROOT)) {
    const p = path.join(BROWSER_ROOT, dir, 'chrome-linux', 'chrome');
    if (fs.existsSync(p)) found.push(p);
  }
  if (!found.length) {
    throw new Error(
      `no chromium under ${BROWSER_ROOT} — set CHROMIUM_PATH, or check that this ` +
      'environment ships a browser (do not run "playwright install")'
    );
  }
  found.sort((a, b) => (a.includes('headless') ? 1 : 0) - (b.includes('headless') ? 1 : 0));
  return found[0];
}

async function launch() {
  return chromium.launch({
    executablePath: resolveChromium(),
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--ssl-version-max=tls1.2'],
    // Chromium does not read HTTPS_PROXY on its own.
    proxy: process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY } : undefined,
  });
}

// Fixed viewport: some controls are reached by coordinate.
const CONTEXT = {
  userAgent:
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  viewport: { width: 1280, height: 900 },
  locale: 'es-ES',
};

// Wait for the transcript to stop growing. Completion is inferred, not
// signalled, so two phases are needed: first wait for generation to actually
// begin, then wait for it to settle. Collapsing these into one quiet-window
// check is the subtle bug — on a large prompt the model can sit silent for
// longer than the window, and the page looks "settled" before a single token
// has appeared, so the reply is reported as empty.
async function waitForReply(page, {
  quietChecks = 4,
  maxPolls = 150,
  interval = 2000,
  startTimeoutMs = 120000,
} = {}) {
  const baseline = await page.locator('body').innerText().catch(() => '');
  const deadline = Date.now() + startTimeoutMs;
  let started = false;
  while (Date.now() < deadline) {
    await page.waitForTimeout(interval);
    const text = await page.locator('body').innerText().catch(() => '');
    if (text !== baseline) { started = true; break; }
  }
  if (!started) return { text: baseline, complete: false, started: false };

  let last = '';
  let quiet = 0;
  for (let i = 0; i < maxPolls; i++) {
    await page.waitForTimeout(interval);
    const text = await page.locator('body').innerText().catch(() => '');
    const busy = /Thinking\.\.\.|Pensando|Generating|Stop generating|Detener/i.test(text);
    if (text === last && !busy) {
      if (++quiet >= quietChecks) return { text: last, complete: true, started: true };
    } else {
      quiet = 0;
      last = text;
    }
  }
  return { text: last, complete: false, started: true };
}

// Recover just the model's answer. Set subtraction alone is wrong for long
// prompts: a review legitimately repeats file names and code that also appear
// in the prompt, and those lines would vanish. The page lays the conversation
// out as [chrome][echoed prompt][reply], so anchor on the prompt's own last
// line and take what follows; fall back to subtraction when the UI collapses
// the message and that anchor is not on the page.
const SENTINEL = '[[FIN-PROMPT]]';

function newLines(before, after) {
  const lines = after.split('\n');
  {
    const tail = SENTINEL;
    const at = lines.map((l) => l.trim()).lastIndexOf(tail);
    if (at !== -1 && at < lines.length - 1) {
      return lines.slice(at + 1).filter((l) => l.trim() && !before.has(l));
    }
  }
  return lines.filter((l) => l.trim() && !before.has(l));
}

// A prompt given as "@path" is read from disk. Review prompts carry a whole
// diff, which is far past what fits comfortably on a command line.
function readPrompt(arg) {
  if (!arg) return null;
  return arg.startsWith('@') ? fs.readFileSync(arg.slice(1), 'utf8') : arg;
}

// fill() sets the value in one shot and still fires the events these editors
// listen for — verified on both a real textarea and a contenteditable. Typing
// character by character would take twenty minutes for a large diff.
async function enterPrompt(page, locator, text) {
  await locator.click();
  // The sentinel marks where the echoed prompt ends and the reply begins.
  const payload = `${text}\n\n${SENTINEL}`;
  try {
    await locator.fill(payload);
  } catch {
    // Custom editors occasionally reject fill(); typing always works.
    await page.keyboard.type(payload, { delay: 0 });
  }
  await page.waitForTimeout(600);
}

module.exports = {
  launch, resolveChromium, CONTEXT, waitForReply, newLines, readPrompt, enterPrompt, SENTINEL,
};
