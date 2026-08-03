// Strict argument parsing shared by every command in this skill.
//
// Each of them used to filter argv with `!a.startsWith('--')` and keep what was
// left. That silently swallows anything unrecognised, which turns a typo into a
// different operation: `apply-files.js repo reply --chek` reads as a request to
// check, and performs the real write instead, because --chek vanishes and CHECK
// stays false. The same shape let a flag's value be mistaken for the positional
// argument — `gate.js --cycle 1 --roles r.json review.md` tried to judge a file
// called "1".
//
// So: declared options only, exact arity, no duplicates, nothing left over.
// Anything else is a usage error, and a usage error must never proceed.
//
//   parse(argv, {
//     name: 'apply-files.js',
//     positionals: ['repo-path', 'reply-file'],
//     flags: { check: { boolean: true } },
//     options: { pr: { integer: true }, dir: {}, area: { repeat: true } },
//   })
//
// Returns { positional: [...], check: bool, pr: 7, area: [...] }, or throws.
function parse(argv, spec) {
  const {
    name, positionals = [], optionalPositionals = [], flags = {}, options = {}, usage,
  } = spec;
  const out = { positional: [] };
  const seen = new Set();

  const fail = (msg) => {
    const line = usage
      || `${name} ${positionals.map((p) => `<${p}>`).join(' ')}`.trim();
    throw new Error(`${msg}\nusage: node ${line}`);
  };

  for (const [key, o] of Object.entries(options)) {
    if (o.repeat) out[camel(key)] = [];
  }
  for (const key of Object.keys(flags)) out[camel(key)] = false;

  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--') {
      // Everything after -- is positional, so a path that begins with dashes
      // can still be passed.
      out.positional.push(...argv.slice(i + 1));
      break;
    }
    if (!a.startsWith('--')) {
      out.positional.push(a);
      continue;
    }

    const key = a.slice(2);
    if (Object.prototype.hasOwnProperty.call(flags, key)) {
      if (seen.has(key)) fail(`--${key} given more than once`);
      seen.add(key);
      out[camel(key)] = true;
      continue;
    }
    if (Object.prototype.hasOwnProperty.call(options, key)) {
      const o = options[key];
      if (seen.has(key) && !o.repeat) fail(`--${key} given more than once`);
      seen.add(key);
      const value = argv[i + 1];
      // A following flag is not a value. Treating it as one is how `--pr` at
      // the end of a line became "no pull request at all" and published a
      // review with nothing tying it to the change under review.
      if (value === undefined || value.startsWith('--')) fail(`--${key} needs a value`);
      i += 1;
      if (o.integer) {
        if (!/^\d+$/.test(value) || Number(value) < 1) {
          fail(`--${key} must be an integer >= 1, got "${value}"`);
        }
        out[camel(key)] = Number(value);
      } else if (o.repeat) {
        out[camel(key)].push(value);
      } else {
        out[camel(key)] = value;
      }
      continue;
    }
    // Unknown. This is the case the old parsers dropped on the floor.
    fail(`unknown option "${a}"`);
  }

  if (out.positional.length < positionals.length) {
    fail(`missing ${positionals[out.positional.length]}`);
  }
  const most = positionals.length + optionalPositionals.length;
  if (out.positional.length > most) {
    fail(`unexpected argument "${out.positional[most]}"`);
  }
  return out;
}

const camel = (s) => s.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

module.exports = { parse };
