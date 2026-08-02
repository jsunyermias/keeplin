// Send one prompt to the Z.ai web chat and print the reply.
//
//   node glm.js "your prompt" [model]
//   model: GLM-5.2 (default) | GLM-5.1 | GLM-5-Turbo | GLM-5V-Turbo | GLM-4.7
//
// Anonymous visitors can chat, but only on GLM-4.7 — every GLM-5 entry in the
// picker is disabled until a session is loaded. A missing or stale
// zai-state.json therefore does not error, it silently downgrades the model,
// which is why the active model is read back rather than assumed.
const fs = require('fs');
const { launch, CONTEXT, waitForReply, newLines } = require('./browser');

const PROMPT = process.argv[2];
const MODEL = process.argv[3] || 'GLM-5.2';
const STATE = 'zai-state.json';

// Signing in adds a sidebar that shifts everything right, so nothing is located
// by fixed coordinate — geometry is read from the elements themselves.
const geometry = (locator) =>
  locator.evaluateAll((els) =>
    els.map((e) => {
      const r = e.getBoundingClientRect();
      return {
        text: (e.textContent || '').trim(),
        cx: Math.round(r.x + r.width / 2),
        cy: Math.round(r.y + r.height / 2),
        visible: !!e.offsetParent && r.width > 0,
      };
    }).filter((o) => o.visible)
  );

// A release-announcement modal can be waiting on load and swallows every click
// underneath it.
async function dismissOverlays(page) {
  for (const sel of ['button:has-text("Next")', '[aria-label="Close"]', 'button:has-text("Skip")']) {
    const el = page.locator(sel).first();
    if (await el.count() && await el.isVisible().catch(() => false)) {
      await el.click().catch(() => {});
      await page.waitForTimeout(1500);
    }
  }
  await page.keyboard.press('Escape').catch(() => {});
  await page.waitForTimeout(1200);
}

(async () => {
  if (!PROMPT) throw new Error('usage: node glm.js "prompt" [model]');

  const browser = await launch();
  const ctx = await browser.newContext({
    ...CONTEXT,
    storageState: fs.existsSync(STATE) ? STATE : undefined,
  });
  const page = await ctx.newPage();

  await page.goto('https://chat.z.ai', { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(9000);
  await dismissOverlays(page);

  const signedIn = !/Sign in/i.test(await page.locator('body').innerText());
  if (!signedIn && MODEL !== 'GLM-4.7') {
    throw new Error(
      `not signed in — ${MODEL} is gated, only GLM-4.7 is available anonymously. ` +
      'Import cookies with import-cookies.js first.'
    );
  }

  // Exact string matching is used rather than a regex because Playwright only
  // normalises surrounding whitespace for the string form, and these labels
  // carry a chevron and padding.
  const KNOWN = ['GLM-5.2', 'GLM-5.1', 'GLM-5-Turbo', 'GLM-5V-Turbo', 'GLM-4.7'];
  const names = KNOWN.includes(MODEL) ? KNOWN : [MODEL, ...KNOWN];

  // The topmost label is the header; any lower one is a menu entry.
  const labels = async () => {
    const out = [];
    for (const n of names) {
      for (const g of await geometry(page.getByText(n, { exact: true }))) {
        out.push({ ...g, text: n });
      }
    }
    return out;
  };
  const header = async () => {
    const all = await labels();
    return all.sort((a, b) => a.cy - b.cy)[0] || null;
  };

  let active = await header();
  if (!active) throw new Error('no model label found — the top bar changed');

  if (active.text !== MODEL) {
    await page.mouse.click(active.cx, active.cy);
    await page.waitForTimeout(3000);

    const option = (await labels())
      .filter((o) => o.text === MODEL && o.cy > active.cy + 20)
      .sort((a, b) => a.cy - b.cy)[0];
    if (!option) throw new Error(`"${MODEL}" not selectable — gated behind sign-in, or renamed`);

    await page.mouse.click(option.cx, option.cy);
    await page.waitForTimeout(2500);
    active = await header();
  }

  console.log('model:', active ? active.text : 'UNKNOWN');
  if (!active || active.text !== MODEL) {
    throw new Error(
      `model selection did not take: header reads "${active && active.text}", wanted "${MODEL}"`
    );
  }

  const before = new Set((await page.locator('body').innerText()).split('\n'));

  // Signed in the composer is a real textarea; anonymously it is a custom
  // editor that exposes none, so fall back to clicking where it renders.
  const box = page.locator('textarea').first();
  if (await box.count() && await box.isVisible().catch(() => false)) {
    await box.click();
  } else {
    await page.mouse.click(668, 385);
  }
  await page.waitForTimeout(800);
  await page.keyboard.type(PROMPT, { delay: 25 });
  await page.waitForTimeout(500);
  await page.keyboard.press('Enter');

  const { text, complete } = await waitForReply(page);
  if (!complete) console.log('note: hit the poll ceiling; reply may be truncated');

  console.log('url:', page.url());
  console.log('--- reply ---');
  console.log(newLines(before, text).join('\n'));

  await page.screenshot({ path: 'zai-chat.png' });
  if (fs.existsSync(STATE)) await ctx.storageState({ path: STATE });
  await browser.close();
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
