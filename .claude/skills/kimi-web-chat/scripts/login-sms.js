// SMS login for the Kimi web chat. Run this in the BACKGROUND: it blocks for
// up to 15 minutes waiting for a human to supply the verification code.
//
//   node login-sms.js <country-code> <phone> <country-name-as-listed>
//   e.g. node login-sms.js +34 612345678 Spain
//
// Progress is appended to login.log and screenshots are written alongside it,
// so the flow can be inspected from another turn while the browser stays alive.
// The script polls otp.txt; write the code there once the user reports it.
const fs = require('fs');
const { launch, CONTEXT } = require('./kimi-lib');

const CC = process.argv[2] || '+34';
const PHONE = process.argv[3];
const COUNTRY = process.argv[4] || 'Spain';
const OTP_FILE = 'otp.txt';
const STATE = 'kimi-state.json';

const log = (...a) => {
  const line = `[${new Date().toISOString()}] ${a.join(' ')}`;
  console.log(line);
  fs.appendFileSync('login.log', line + '\n');
};

(async () => {
  if (!PHONE) throw new Error('missing phone number');
  if (fs.existsSync(OTP_FILE)) fs.unlinkSync(OTP_FILE);

  const browser = await launch();
  const ctx = await browser.newContext(CONTEXT);
  const page = await ctx.newPage();

  log('opening kimi.com');
  await page.goto('https://www.kimi.com', { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(5000);

  // The chat UI renders for anonymous visitors; the login wall only appears on
  // send, so a throwaway message is the way to reach it.
  const box = page.locator('div[contenteditable="true"], textarea').first();
  await box.click();
  await box.type('hola', { delay: 20 });
  await page.keyboard.press('Enter');
  await page.waitForTimeout(5000);

  const ccInput = page.locator('input[placeholder="+1"]').first();
  const telInput = page.locator('input[type="tel"]').first();
  const codeInput = page.locator('input[placeholder="Verification code"]').first();

  await telInput.waitFor({ state: 'visible', timeout: 30000 });

  // The country code is a searchable dropdown, not a free-text field. Typing
  // into it also clears the phone input, so the country must be picked first
  // or the number silently reverts to +1 and no SMS ever arrives.
  await ccInput.click();
  await page.waitForTimeout(800);
  await ccInput.fill(CC.replace('+', ''));
  await page.waitForTimeout(1500);
  await page.getByText(COUNTRY, { exact: true }).first().click({ timeout: 15000 });
  await page.waitForTimeout(1000);
  log(`selected country ${COUNTRY}`);

  await telInput.fill(PHONE);
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'sms-1-filled.png' });

  const shownTel = await telInput.inputValue().catch(() => '');
  log(`form reads tel=${JSON.stringify(shownTel)}`);
  if (!shownTel) throw new Error('phone field empty after fill — aborting before Send');

  await page.getByRole('button', { name: 'Send', exact: true }).first().click();
  log('clicked Send');
  await page.waitForTimeout(6000);
  await page.screenshot({ path: 'sms-2-after-send.png' });

  // A countdown replacing the Send button ("117 s") means the SMS went out.
  const afterSend = await page.locator('body').innerText().catch(() => '');
  const sent = /\d+\s*s\b/.test(afterSend);
  log(`after Send: countdown_visible=${sent}`);
  log('page tail: ' + JSON.stringify(afterSend.slice(-400)));
  if (!sent) log('WARNING: no countdown detected — check sms-2-after-send.png for a captcha');

  log(`waiting for ${OTP_FILE} (up to 15 min)`);
  let code = null;
  for (let i = 0; i < 180; i++) {
    if (fs.existsSync(OTP_FILE)) {
      code = fs.readFileSync(OTP_FILE, 'utf8').trim();
      if (code) break;
    }
    await page.waitForTimeout(5000);
  }
  if (!code) throw new Error('no OTP supplied in time');
  log('got code, submitting');

  await codeInput.fill(code);
  await page.waitForTimeout(400);
  await page.getByRole('button', { name: 'Log In', exact: true }).last().click();
  await page.waitForTimeout(10000);
  await page.screenshot({ path: 'sms-3-after-login.png' });

  const text = await page.locator('body').innerText().catch(() => '');
  const loggedIn = !/Log in to sync chat history/i.test(text);
  log('logged in: ' + loggedIn);
  if (!loggedIn) log('page tail: ' + JSON.stringify(text.slice(-400)));

  if (loggedIn) {
    await ctx.storageState({ path: STATE });
    log('saved session to ' + STATE);
  }
  await browser.close();
  log('done');
})().catch((e) => {
  log('ERROR: ' + e.message.split('\n')[0]);
  process.exit(1);
});
