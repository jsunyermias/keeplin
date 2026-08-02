// Send one prompt to Qwen Studio (chat.qwen.ai) and print the reply.
//
//   node qwen.js "your prompt" [model]
//   node qwen.js @prompt.txt [model]     # long prompts, e.g. a review diff
//   model: Qwen3.8-Max-Preview (default) | Qwen3.7-Max | Qwen3.7-Plus
//
// Unlike Z.ai there is no anonymous mode — the site requires a session for any
// send, so a missing qwen-state.json fails fast rather than degrading.
const fs = require('fs');
const { launch, CONTEXT, waitForReply, newLines, readPrompt, enterPrompt } = require('./browser');

const PROMPT = readPrompt(process.argv[2]);
const MODEL = process.argv[3] || 'Qwen3.8-Max-Preview';
const STATE = 'qwen-state.json';

(async () => {
  if (!PROMPT) throw new Error('usage: node qwen.js "prompt" [model]');
  if (!fs.existsSync(STATE)) {
    throw new Error(`no ${STATE} — import cookies with import-cookies.js first`);
  }

  const browser = await launch();
  const ctx = await browser.newContext({ ...CONTEXT, storageState: STATE });
  const page = await ctx.newPage();

  await page.goto('https://chat.qwen.ai', { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(9000);

  if (/Iniciar sesi.n|Sign in|Crear una cuenta/i.test(await page.locator('body').innerText())) {
    throw new Error('session expired or cookies rejected — re-export and re-import');
  }

  // The header shows the active model and opens the picker when clicked.
  const header = page.locator('text=/^Qwen[0-9.]+-(Max-Preview|Max|Plus)$/').first();
  const current = (await header.innerText().catch(() => '')).trim();
  if (current !== MODEL) {
    await header.click({ timeout: 20000 });
    await page.waitForTimeout(3000);
    await page.getByText(MODEL, { exact: true }).first().click({ timeout: 15000 });
    await page.waitForTimeout(2500);
  }

  const active = (await header.innerText().catch(() => '')).trim();
  console.log('model:', active || 'UNKNOWN');
  if (active !== MODEL) {
    throw new Error(`model selection did not take: header reads "${active}", wanted "${MODEL}"`);
  }

  const before = new Set((await page.locator('body').innerText()).split('\n'));
  const box = page.locator('textarea').first();
  await box.waitFor({ state: 'visible', timeout: 30000 });
  await enterPrompt(page, box, PROMPT);
  await page.keyboard.press('Enter');

  const { text, complete, started } = await waitForReply(page, {
    quietChecks: Number(process.env.QUIET_CHECKS || 4),
    maxPolls: Number(process.env.MAX_POLLS || 150),
  });
  if (!started) throw new Error('the model never began replying — prompt may exceed what this UI accepts');
  if (!complete) console.log('note: hit the poll ceiling; reply may be truncated');

  console.log('url:', page.url());
  console.log('--- reply ---');
  console.log(newLines(before, text).join('\n'));

  await page.screenshot({ path: 'qwen-chat.png' });
  await ctx.storageState({ path: STATE });
  await browser.close();
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
