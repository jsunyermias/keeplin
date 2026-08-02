// Send one prompt to the Kimi web chat and print the reply.
//
//   node kimi.js "your prompt" [model]
//   node kimi.js @prompt.txt [model]     # long prompts, e.g. a review diff
//   model: K3 (default) | Instant | "K3 Swarm"
//
// Requires an authenticated kimi-state.json in the working directory; without
// it the send silently hits the login wall. Run login-sms.js first.
const fs = require('fs');
const { launch, CONTEXT, readPrompt, enterPrompt } = require('./kimi-lib');

const PROMPT = readPrompt(process.argv[2]);
const MODEL = process.argv[3] || 'K3';
const STATE = 'kimi-state.json';

// Kimi keeps thinking long after the visible text pauses, so "done" is inferred
// from the transcript going quiet. Deep Research and Swarm runs pause for much
// longer than this and will be cut short — raise both values for those.
const QUIET_CHECKS = 3;
const MAX_POLLS = 60;

(async () => {
  if (!PROMPT) throw new Error('usage: node kimi.js "prompt" [model]');
  if (!fs.existsSync(STATE)) throw new Error(`no ${STATE} — run login-sms.js first`);

  const browser = await launch();
  const ctx = await browser.newContext({ ...CONTEXT, storageState: STATE });
  const page = await ctx.newPage();

  await page.goto('https://www.kimi.com', { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(5000);

  if (/Log in to sync chat history/i.test(await page.locator('body').innerText())) {
    throw new Error('session expired — run login-sms.js again');
  }

  // The picker sits at the right edge of the composer and resets to "Instant"
  // on every fresh context, so K3 has to be chosen each run. It is clicked by
  // coordinate because the control exposes no stable accessible name; this is
  // the most brittle line in the skill and depends on the fixed viewport.
  await page.mouse.click(898, 465);
  await page.waitForTimeout(2000);
  await page.getByText(MODEL, { exact: true }).first().click({ timeout: 15000 });
  await page.waitForTimeout(1500);

  const selector = (await page.locator('body').innerText())
    .match(/(Instant|K3 Swarm|K3)\s+(High|Medium|Low)/);
  console.log('model:', selector
    ? selector[0].replace(/\s+/g, ' ')
    : 'UNKNOWN — verify before trusting the reply');

  const box = page.locator('div[contenteditable="true"], textarea').first();
  await box.waitFor({ state: 'visible', timeout: 30000 });
  await enterPrompt(page, box, PROMPT);

  // Snapshot the chrome (sidebar, nav, chat list) so it can be subtracted from
  // the result — reading document.body wholesale otherwise dumps the account's
  // entire chat history into the output.
  const before = new Set((await page.locator('body').innerText()).split('\n'));
  await page.keyboard.press('Enter');

  let last = '';
  let quiet = 0;
  for (let i = 0; i < MAX_POLLS; i++) {
    await page.waitForTimeout(2000);
    const text = await page.locator('body').innerText().catch(() => '');
    if (text === last) {
      if (++quiet >= QUIET_CHECKS) break;
    } else {
      quiet = 0;
      last = text;
    }
  }
  if (quiet < QUIET_CHECKS) console.log('note: hit the poll ceiling; reply may be truncated');

  const fresh = last.split('\n').filter((l) => l.trim() && !before.has(l));
  console.log('url:', page.url());
  console.log('--- reply ---');
  console.log(fresh.join('\n'));

  await page.screenshot({ path: 'kimi-chat.png' });
  await ctx.storageState({ path: STATE });
  await browser.close();
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
