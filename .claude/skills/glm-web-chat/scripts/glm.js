// Send one prompt to the Z.ai web chat and print the reply.
//
//   node glm.js "your prompt" [model]
//   node glm.js @prompt.txt [model]      # long prompts, e.g. a review diff
//   model: GLM-5.2 (default) | GLM-5.1 | GLM-5-Turbo | GLM-5V-Turbo | GLM-4.7
//
// Anonymous visitors can chat, but only on GLM-4.7 — every GLM-5 entry in the
// picker is disabled until a session is loaded. Left alone the site would
// quietly answer as GLM-4.7 instead, so a missing or stale zai-state.json is
// refused outright for any gated model, and the active model is read back from
// the page afterwards rather than assumed.
const fs = require('fs');
const {
  launch, CONTEXT, waitForReply, waitForGeneration, newLines, copyCodeBlocks,
  readPrompt, enterPrompt,
} = require('./browser');

const PROMPT = readPrompt(process.argv[2]);
const MODEL = (process.argv[3] || '').replace(/^--.*/, '') || 'GLM-5.2';
// Prose survives the page renderer; code does not, so code is lifted from the
// clipboard instead. See copyCodeBlocks in browser.js for why.
const CODE_MODE = process.argv.includes('--code');
const STATE = 'zai-state.json';

// Signing in adds a sidebar that shifts everything right, so the model picker
// is located from the elements' own geometry rather than by fixed coordinate.
// The one exception is the signed-out composer, which exposes no element to
// measure; see the fallback further down.
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
    await enterPrompt(page, box, PROMPT);
  } else {
    // Signed out the composer is a custom editor exposing no textarea.
    await page.mouse.click(668, 385);
    await page.waitForTimeout(800);
    await page.keyboard.type(PROMPT, { delay: 0 });
    await page.waitForTimeout(600);
  }
  await page.keyboard.press('Enter');

  if (CODE_MODE) {
    const finished = await waitForGeneration(page);
    if (finished === false) console.log('note: generation did not finish within the ceiling');
    if (finished === null) {
      // No stop control on this site; fall back to the quiet heuristic.
      await waitForReply(page, { quietChecks: 4, maxPolls: 150 });
    }
    await page.waitForTimeout(2500);
    const blocks = await copyCodeBlocks(page);
    if (!blocks.length) {
      throw new Error('no code block could be copied — the reply had none, or the copy control moved');
    }
    console.log('url:', page.url());
    for (const [i, b] of blocks.entries()) {
      console.log(`--- code ${i} ---`);
      console.log(b);
    }
  } else {
    const { text, complete, started } = await waitForReply(page, {
      quietChecks: Number(process.env.QUIET_CHECKS || 4),
      maxPolls: Number(process.env.MAX_POLLS || 150),
    });
    if (!started) throw new Error('the model never began replying — prompt may exceed what this UI accepts');
    if (!complete) console.log('note: hit the poll ceiling; reply may be truncated');

    const reply = newLines(before, text).join('\n').trim();
    // An empty reply is a failure, not a review with nothing to say. Left
    // alone it exits 0 and the caller records a successful run that carries
    // no verdict — silence dressed as success is the worst outcome here.
    if (!reply) {
      throw new Error(
        'the reply came back empty — the send did not take, or the page stalled. Rerun; ' +
        `the conversation was ${page.url()}`
      );
    }
    console.log('url:', page.url());
    console.log('--- reply ---');
    console.log(reply);
  }

  await page.screenshot({ path: 'zai-chat.png' });
  if (fs.existsSync(STATE)) await ctx.storageState({ path: STATE });
  await browser.close();
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
