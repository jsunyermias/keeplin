// Shared browser setup for the Kimi web-chat scripts.
//
// Two environment quirks are handled here rather than in each script, because
// getting either wrong produces a confusing failure far from its cause:
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
  const candidates = [];
  for (const dir of fs.readdirSync(BROWSER_ROOT)) {
    const p = path.join(BROWSER_ROOT, dir, 'chrome-linux', 'chrome');
    if (fs.existsSync(p)) candidates.push(p);
  }
  if (!candidates.length) {
    throw new Error(
      `no chromium under ${BROWSER_ROOT} — set CHROMIUM_PATH, or check that this ` +
      'environment ships a browser (do not run "playwright install")'
    );
  }
  // Prefer full chromium over headless_shell: the shell build lacks some UI.
  candidates.sort((a, b) => (a.includes('headless') ? 1 : 0) - (b.includes('headless') ? 1 : 0));
  return candidates[0];
}

async function launch() {
  return chromium.launch({
    executablePath: resolveChromium(),
    args: [
      '--no-sandbox',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      '--ssl-version-max=tls1.2',
    ],
    // Chromium does not read HTTPS_PROXY on its own.
    proxy: process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY } : undefined,
  });
}

// Fixed viewport keeps the rendered layout predictable between runs.
const CONTEXT = {
  // Clipboard access is what makes exact code recovery possible.
  permissions: ['clipboard-read', 'clipboard-write'],
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

// Generation is in progress exactly while the composer offers a stop control.
// That is an explicit signal from the page, unlike inferring completion from
// the text going quiet — which mistakes a pause for thought for an answer.
async function waitForGeneration(page, { startMs = 15000, maxMs = 360000 } = {}) {
  const stop = () => page.locator('[class*="stop" i], button:has(svg[class*="stop" i])');
  const startDeadline = Date.now() + startMs;
  while (Date.now() < startDeadline) {
    if (await stop().count()) break;
    await page.waitForTimeout(1000);
  }
  const deadline = Date.now() + maxMs;
  while (Date.now() < deadline) {
    await page.waitForTimeout(2000);
    if (!(await stop().count())) return true;
  }
  return false;
}

// Recover code exactly, by clicking each code block's copy icon and reading the
// clipboard. Reading the rendered page cannot be used for this: innerText drops
// blank lines and, in long blocks, whole lines — Z.ai renders code in
// CodeMirror, which virtualises rows, so the page never holds them all.
//
// Every one of these sites mounts a virtualising editor rather than a plain
// <pre>: Z.ai uses CodeMirror, Qwen uses Monaco. Both keep only the visible
// rows in the DOM, which is precisely why the page cannot be read for code.
//
// The icon is neither a button nor accessibly named, and each site places it
// differently, so candidates are found by proximity to the block's top-right
// corner and tried in turn — the clipboard changing is the only reliable way
// to tell the copy control from the download button beside it.
const CODE_ROOTS = [
  'pre',                    // Kimi and anything rendering plain markdown
  '.cm-scroller',           // CodeMirror (Z.ai)
  '.monaco-editor',         // Monaco (Qwen)
  '[class*="code-block" i]',
].join(', ');

async function copyCodeBlocks(page, { rootSelector = CODE_ROOTS } = {}) {
  const roots = page.locator(rootSelector);
  const n = await roots.count();
  const out = [];
  for (let i = 0; i < n; i++) {
    const root = roots.nth(i);
    if (!(await root.isVisible().catch(() => false))) continue;

    // Several icons can sit together — Qwen puts copy beside download — so
    // gather the candidates rather than guessing which is which, and try them
    // until the clipboard actually changes.
    const spots = await root.evaluate((el) => {
      const r = el.getBoundingClientRect();
      if (r.width < 80 || r.height < 20) return [];
      return [...document.querySelectorAll('*')]
        .map((e) => e.getBoundingClientRect())
        .filter((b) =>
          b.width > 8 && b.width < 44 && b.height > 8 && b.height < 44 &&
          b.left > r.right - 90 && b.right < r.right + 30 &&
          b.bottom > r.top - 70 && b.top < r.top + 24
        )
        .sort((a, z) => z.top - a.top || z.left - a.left)
        .slice(0, 6)
        .map((b) => ({ x: Math.round(b.left + b.width / 2), y: Math.round(b.top + b.height / 2) }));
    }).catch(() => []);
    if (!spots.length) continue;

    await root.hover().catch(() => {});
    await page.waitForTimeout(400);

    for (const spot of spots) {
      const sentinel = `__SIN_COPIA_${i}_${spot.x}_${spot.y}__`;
      await page.evaluate((v) => navigator.clipboard.writeText(v), sentinel);
      await page.mouse.click(spot.x, spot.y);
      await page.waitForTimeout(1000);
      const text = await page.evaluate(() => navigator.clipboard.readText().catch(() => ''));
      // An unchanged clipboard means that icon was not the copy control.
      if (text && text !== sentinel) {
        if (!out.includes(text)) out.push(text);
        break;
      }
    }
  }
  return out;
}

// A prompt given as "@path" is read from disk. Review prompts carry a whole
// diff, which is far past what fits comfortably on a command line.
function readPrompt(arg) {
  if (!arg) return null;
  return arg.startsWith('@') ? fs.readFileSync(arg.slice(1), 'utf8') : arg;
}

// fill() sets the value in one shot and still fires the events this editor
// listens for — verified against Kimi's contenteditable composer. Typing
// character by character would take twenty minutes for a large diff.
async function enterPrompt(page, locator, text) {
  await locator.click();
  // The sentinel marks where the echoed prompt ends and the reply begins.
  const payload = `${text}\n\n${SENTINEL}`;
  try {
    await locator.fill(payload);
  } catch {
    await page.keyboard.type(payload, { delay: 0 });
  }
  await page.waitForTimeout(600);
}

module.exports = {
  launch, resolveChromium, CONTEXT, waitForReply, waitForGeneration, newLines,
  copyCodeBlocks, readPrompt, enterPrompt, SENTINEL,
};
