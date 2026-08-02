// Convert a browser cookie export into a Playwright storageState file.
//
//   node import-cookies.js <export.json> <out-state.json> [domain-filter]
//
// Cookie-Editor (and most extensions) emit a flat array whose field names and
// value encodings differ from what Playwright expects. The mismatches are all
// silent — a wrong sameSite or a float expiry loads without error and simply
// fails to authenticate — so each one is normalised explicitly here.
const fs = require('fs');

const [SRC, OUT, FILTER] = process.argv.slice(2);

// Extensions use Chrome's wire values; Playwright wants the RFC spellings.
const SAME_SITE = {
  no_restriction: 'None',
  unspecified: 'Lax',
  lax: 'Lax',
  strict: 'Strict',
  none: 'None',
};

function normalise(c) {
  const domain = c.domain || c.host || '';
  const sameSiteKey = String(c.sameSite || 'unspecified').toLowerCase();
  let sameSite = SAME_SITE[sameSiteKey] || 'Lax';
  // A cookie marked SameSite=None must also be Secure or Chromium drops it.
  if (sameSite === 'None' && !c.secure) sameSite = 'Lax';

  // Session cookies carry no expiry; Playwright encodes that as -1. Real
  // expiries arrive as float seconds and must be integers.
  const expires = c.session || c.expirationDate == null
    ? -1
    : Math.floor(Number(c.expirationDate));

  return {
    name: c.name,
    value: c.value,
    domain,
    path: c.path || '/',
    expires,
    httpOnly: !!c.httpOnly,
    secure: !!c.secure,
    sameSite,
  };
}

(() => {
  if (!SRC || !OUT) throw new Error('usage: node import-cookies.js <export.json> <out.json> [domain]');

  const raw = JSON.parse(fs.readFileSync(SRC, 'utf8'));
  // Accept a bare array, a Playwright-shaped object, or an extension wrapper.
  const list = Array.isArray(raw) ? raw : raw.cookies || raw.data || [];
  if (!list.length) throw new Error('no cookies found in export');

  let cookies = list.map(normalise).filter((c) => c.name && c.domain);
  if (FILTER) cookies = cookies.filter((c) => c.domain.includes(FILTER));
  if (!cookies.length) throw new Error(`no cookies left after filtering on "${FILTER}"`);

  const now = Math.floor(Date.now() / 1000);
  const expired = cookies.filter((c) => c.expires !== -1 && c.expires < now);
  if (expired.length) {
    console.log(`warning: ${expired.length} cookie(s) already expired: ` +
      expired.map((c) => c.name).join(', '));
  }

  fs.writeFileSync(OUT, JSON.stringify({ cookies, origins: [] }, null, 2));
  console.log(`wrote ${OUT}: ${cookies.length} cookies`);
  console.log('domains:', [...new Set(cookies.map((c) => c.domain))].join(', '));
  console.log('session-only:', cookies.filter((c) => c.expires === -1).length);
})();
