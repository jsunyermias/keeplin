// Send one prompt to OpenAI and print the reply, matching the interface of the
// browser drivers so the review workflow can treat all four reviewers alike.
//
//   node codex.js "your prompt" [model]
//   node codex.js @prompt.txt [model]
//   node codex.js --list                 # what this key can actually reach
//
// Reads OPENAI_API_KEY. OPENAI_BASE_URL overrides the endpoint for a gateway or
// a compatible provider.
const fs = require('fs');

const KEY = process.env.OPENAI_API_KEY;
const BASE = (process.env.OPENAI_BASE_URL || 'https://api.openai.com/v1').replace(/\/$/, '');
const arg = process.argv[2];
// Model ids move faster than any list embedded here, so the default is
// configuration rather than a guess baked into the code. Run --list first.
const MODEL = process.argv[3] || process.env.OPENAI_MODEL;

async function call(path, init) {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${KEY}`,
      'Content-Type': 'application/json',
      ...(init && init.headers),
    },
  });
  const body = await res.text();
  if (!res.ok) {
    let detail = body.slice(0, 400);
    try {
      detail = JSON.parse(body).error.message;
    } catch {
      /* keep the raw body */
    }
    throw new Error(`HTTP ${res.status} from ${path}: ${detail}`);
  }
  return JSON.parse(body);
}

(async () => {
  if (!KEY) throw new Error('OPENAI_API_KEY is not set');

  if (arg === '--list') {
    const { data } = await call('/models', { method: 'GET' });
    for (const m of data.map((d) => d.id).sort()) console.log(m);
    return;
  }

  if (!arg) throw new Error('usage: node codex.js "prompt"|@file [model] | --list');
  if (!MODEL) {
    throw new Error('no model given — pass one as argv[2] or set OPENAI_MODEL (see --list)');
  }

  const prompt = arg.startsWith('@') ? fs.readFileSync(arg.slice(1), 'utf8') : arg;

  const out = await call('/chat/completions', {
    method: 'POST',
    body: JSON.stringify({
      model: MODEL,
      messages: [{ role: 'user', content: prompt }],
      temperature: 0.2,
    }),
  });

  const choice = out.choices && out.choices[0];
  console.log('model:', out.model || MODEL);
  console.log('--- reply ---');
  console.log((choice && choice.message && choice.message.content) || '(empty response)');
  if (choice && choice.finish_reason && choice.finish_reason !== 'stop') {
    console.log(`note: finish_reason=${choice.finish_reason}; the reply may be cut off`);
  }
})().catch((e) => {
  console.error('ERROR:', e.message.split('\n')[0]);
  process.exit(1);
});
