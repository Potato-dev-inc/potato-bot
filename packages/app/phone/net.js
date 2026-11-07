// Networking helpers: hello proof, candidate race, small errors. No secrets are ever logged.
export const STORE_KEY = 'potato.phone.v1';
export const HELLO_TIMEOUT_MS = 2500;
const enc = new TextEncoder();

export class ApiError extends Error {
  constructor(status, code, message) { super(message); this.name = 'ApiError'; this.status = status; this.code = code; }
}
export class NetError extends Error {
  constructor() { super('network'); this.name = 'NetError'; }
}

export function toHex(bytes) {
  let s = '';
  for (const b of bytes) s += b.toString(16).padStart(2, '0');
  return s;
}
export function b64ToBytes(str) {
  let x = String(str).trim().replace(/-/g, '+').replace(/_/g, '/');
  while (x.length % 4) x += '=';
  const bin = atob(x);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
export function randomNonce(crypto) {
  const a = new Uint8Array(16);
  crypto.getRandomValues(a);
  return toHex(a);
}
// Pure JS SHA-256 / HMAC-SHA256, used when crypto.subtle is missing (plain http pages are not secure contexts).
const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
]);
export function sha256(data) {
  const len = data.length;
  const total = ((len + 9 + 63) >> 6) << 6;
  const buf = new Uint8Array(total);
  buf.set(data);
  buf[len] = 0x80;
  const dv = new DataView(buf.buffer);
  dv.setUint32(total - 8, Math.floor((len * 8) / 4294967296));
  dv.setUint32(total - 4, (len * 8) >>> 0);
  const H = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  const w = new Uint32Array(64);
  const rotr = (x, n) => (x >>> n) | (x << (32 - n));
  for (let off = 0; off < total; off += 64) {
    for (let i = 0; i < 16; i++) w[i] = dv.getUint32(off + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, h] = H;
    for (let i = 0; i < 64; i++) {
      const S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const t1 = (h + S1 + ch + K[i] + w[i]) >>> 0;
      const S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const mj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (S0 + mj) >>> 0;
      h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    H[0] += a; H[1] += b; H[2] += c; H[3] += d; H[4] += e; H[5] += f; H[6] += g; H[7] += h;
  }
  const out = new Uint8Array(32);
  const odv = new DataView(out.buffer);
  for (let i = 0; i < 8; i++) odv.setUint32(i * 4, H[i]);
  return out;
}
export function hmacSha256(key, msg) {
  let k = key.length > 64 ? sha256(key) : key;
  const ipad = new Uint8Array(64 + msg.length);
  const opad = new Uint8Array(96);
  for (let i = 0; i < 64; i++) {
    const kb = i < k.length ? k[i] : 0;
    ipad[i] = kb ^ 0x36;
    opad[i] = kb ^ 0x5c;
  }
  ipad.set(msg, 64);
  opad.set(sha256(ipad), 64);
  return sha256(opad);
}
const hasSubtle = (crypto) => !!(crypto && crypto.subtle && typeof crypto.subtle.digest === 'function');

/** pair: SHA-256(UPPERCASE code utf8). device: SHA-256(raw bytes of base64 token). */
export async function helloKey(crypto, kind, secret) {
  const bytes = kind === 'pair' ? enc.encode(String(secret).toUpperCase()) : b64ToBytes(secret);
  if (!hasSubtle(crypto)) return sha256(bytes);
  return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
}
export async function helloProof(crypto, keyBytes, nonce) {
  const msg = enc.encode('potato-phone-hello|' + nonce);
  if (!hasSubtle(crypto)) return toHex(hmacSha256(keyBytes, msg));
  const k = await crypto.subtle.importKey('raw', keyBytes, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const sig = await crypto.subtle.sign('HMAC', k, msg);
  return toHex(new Uint8Array(sig));
}
export function normalizeOrigin(s) {
  try {
    const u = new URL(String(s));
    return u.protocol === 'http:' || u.protocol === 'https:' ? u.origin : null;
  } catch { return null; }
}
export function dedupeOrigins(list) {
  const seen = new Set();
  const out = [];
  for (const x of list) {
    const o = normalizeOrigin(x);
    if (o && !seen.has(o)) { seen.add(o); out.push(o); }
  }
  return out;
}

async function hello(deps, origin, kind, keyBytes, deviceId, ctrl) {
  const clear = deps.clearTimeout || ((id) => globalThis.clearTimeout(id));
  const timer = deps.setTimeout(() => ctrl.abort(), HELLO_TIMEOUT_MS);
  try {
    const nonce = randomNonce(deps.crypto);
    const body = { kind, nonce };
    if (kind === 'device' && deviceId) body.deviceId = deviceId;
    const res = await deps.fetch(origin + '/api/hello', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Potato-Phone': '1' },
      body: JSON.stringify(body),
      signal: ctrl.signal
    });
    if (!res.ok) throw new Error('bad status');
    const j = await res.json();
    const want = await helloProof(deps.crypto, keyBytes, nonce);
    if (!j || typeof j.proof !== 'string' || j.proof.toLowerCase() !== want) throw new Error('bad proof');
    return { origin, pcName: String(j.pcName || '') };
  } finally {
    try { clear(timer); } catch { /* ignore */ }
  }
}

/** Race all origins; first VERIFIED answer wins. Failures and wrong proofs are ignored. Resolves null if none. */
export function raceHello(deps, origins, kind, keyBytes, deviceId) {
  return new Promise((resolve) => {
    let left = origins.length;
    if (!left) return resolve(null);
    const ctrls = origins.map(() => new AbortController());
    let done = false;
    origins.forEach((o, i) => {
      hello(deps, o, kind, keyBytes, deviceId, ctrls[i]).then(
        (win) => {
          if (done) return;
          done = true;
          ctrls.forEach((c, j) => { if (j !== i) { try { c.abort(); } catch { /* ignore */ } } });
          resolve(win);
        },
        () => { if (--left === 0 && !done) { done = true; resolve(null); } }
      );
    });
  });
}
