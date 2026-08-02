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

// The viewport is fixed because the model picker is clicked by coordinate.
const CONTEXT = {
  userAgent:
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  viewport: { width: 1280, height: 900 },
  locale: 'es-ES',
};

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
  try {
    await locator.fill(text);
  } catch {
    await page.keyboard.type(text, { delay: 0 });
  }
  await page.waitForTimeout(600);
}

module.exports = { launch, resolveChromium, CONTEXT, readPrompt, enterPrompt };
