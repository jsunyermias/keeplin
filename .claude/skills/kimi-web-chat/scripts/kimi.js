// Send one prompt to the Kimi web chat and print the reply.
//
//   node kimi.js "your prompt" [model]
//   node kimi.js @prompt.txt [model]     # long prompts, e.g. a review diff
//   node kimi.js @prompt.txt K3 --code   # emit code blocks, recovered exactly
//   model: K3 (default) | Instant | "K3 Swarm"
//
// Requires an authenticated kimi-state.json in the working directory; without
// it the send silently hits the login wall. Run login-sms.js first.
const fs = require('fs');
const {
  launch, CONTEXT, waitForReply, waitForGeneration, newLines, copyCodeBlocks,
  readPrompt, enterPrompt,
} = require('./kimi-lib');

const PROMPT = readPrompt(process.argv[2]);
const MODEL = (process.argv[3] || 'K3').replace(/^--.*/, '') || 'K3';
// Prose survives the page renderer; code does not, so code is lifted from the
// clipboard instead. See copyCodeBlocks in kimi-lib.js for why.
const CODE_MODE = process.argv.includes('--code');
const STATE = 'kimi-state.json';

// Completion is inferred, not signalled. Deep Research and Swarm runs think far
// longer than the defaults allow and will be cut short — raise these for those.
const QUIET_CHECKS = Number(process.env.QUIET_CHECKS || 4);
const MAX_POLLS = Number(process.env.MAX_POLLS || 150);

(async () => {
  if (!PROMPT) throw new Error('usage: node kimi.js "prompt" [model]');
  if (!fs.existsSync(STATE)) throw new Error(`no ${STATE} — run login-sms.js first`);

  const browser = await launch();
  const ctx = await browser.newContext({ ...CONTEXT, storageState: STATE });
  const page = await ctx.newPage();

  await page.goto('https://www.kimi.com', { waitUntil: 'domcontentloaded', timeout: 60000 });
  // The composer and its model picker render late; 5s was not enough and the
  // run died claiming the UI had changed.
  await page.waitForTimeout(9000);

  if (/Log in to sync chat history/i.test(await page.locator('body').innerText())) {
    throw new Error('session expired — run login-sms.js again');
  }

  // The picker sits at the right edge of the composer and resets to "Instant"
  // on every fresh context, so the wanted model has to be chosen each run. Its
  // position shifts with the sidebar's state, so the control is located from
  // the elements' own geometry rather than by fixed coordinate: a hardcoded
  // point silently misses and the run dies on an unexplained click timeout.
  const KNOWN = ['K3 Swarm', 'K3', 'Instant'];
  const names = KNOWN.includes(MODEL) ? KNOWN : [MODEL, ...KNOWN];

  const labels = async () => {
    const out = [];
    for (const n of names) {
      const found = await page.getByText(n, { exact: true }).evaluateAll((els) =>
        els.map((e) => {
          const r = e.getBoundingClientRect();
          return {
            cx: Math.round(r.x + r.width / 2),
            cy: Math.round(r.y + r.height / 2),
            visible: !!e.offsetParent && r.width > 0,
          };
        }).filter((o) => o.visible)
      );
      for (const g of found) out.push({ ...g, text: n });
    }
    return out;
  };

  // With the menu closed, the only visible label is the active model.
  const active = async () => (await labels()).sort((a, b) => a.cy - b.cy)[0] || null;

  let current = await active();
  if (!current) throw new Error('no model label found on the composer — the UI changed');

  if (current.text !== MODEL) {
    await page.mouse.click(current.cx, current.cy);
    await page.waitForTimeout(2500);
    const option = (await labels())
      .filter((o) => o.text === MODEL && Math.abs(o.cy - current.cy) > 15)
      .sort((a, b) => a.cy - b.cy)[0];
    if (!option) throw new Error(`"${MODEL}" not selectable — renamed, or gated on this account`);
    await page.mouse.click(option.cx, option.cy);
    await page.waitForTimeout(2000);
    current = await active();
  }

  console.log('model:', current ? current.text : 'UNKNOWN');
  if (!current || current.text !== MODEL) {
    throw new Error(
      `model selection did not take: composer reads "${current && current.text}", wanted "${MODEL}"`
    );
  }

  const box = page.locator('div[contenteditable="true"], textarea').first();
  await box.waitFor({ state: 'visible', timeout: 30000 });
  await enterPrompt(page, box, PROMPT);

  // Snapshot the chrome (sidebar, nav, chat list) so it can be subtracted from
  // the result — reading document.body wholesale otherwise dumps the account's
  // entire chat history into the output.
  const before = new Set((await page.locator('body').innerText()).split('\n'));
  await page.keyboard.press('Enter');

  if (CODE_MODE) {
    const finished = await waitForGeneration(page);
    if (!finished) console.log('note: generation did not finish within the ceiling');
    await page.waitForTimeout(2000);
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
      quietChecks: QUIET_CHECKS,
      maxPolls: MAX_POLLS,
    });
    if (!started) throw new Error('the model never began replying — prompt may exceed what this UI accepts');
    if (!complete) console.log('note: hit the poll ceiling; reply may be truncated');

    console.log('url:', page.url());
    console.log('--- reply ---');
    console.log(newLines(before, text).join('\n'));
  }

  await page.screenshot({ path: 'kimi-chat.png' });
  await ctx.storageState({ path: STATE });
  await browser.close();
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
