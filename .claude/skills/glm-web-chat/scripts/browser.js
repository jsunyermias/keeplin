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
// signalled, so slow modes need a longer quiet window and ceiling.
async function waitForReply(page, { quietChecks = 3, maxPolls = 60, interval = 2000 } = {}) {
  let last = '';
  let quiet = 0;
  for (let i = 0; i < maxPolls; i++) {
    await page.waitForTimeout(interval);
    const text = await page.locator('body').innerText().catch(() => '');
    if (text === last) {
      if (++quiet >= quietChecks) return { text: last, complete: true };
    } else {
      quiet = 0;
      last = text;
    }
  }
  return { text: last, complete: false };
}

// Subtract the page chrome captured before sending, so the account's sidebar
// and chat history stay out of the output.
function newLines(before, after) {
  return after.split('\n').filter((l) => l.trim() && !before.has(l));
}

module.exports = { launch, resolveChromium, CONTEXT, waitForReply, newLines };
