// Generates the monochrome potato icons (PNG, no dependencies besides node:zlib).
// Run: node packages/app/phone/tools/make-icons.mjs
import { deflateSync } from 'node:zlib';
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const out = join(dirname(fileURLToPath(import.meta.url)), '..');

const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}
function png(w, h, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    rgba.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))
  ]);
}

// Shapes are described in a 512 x 512 space (same as icon.svg).
function inRoundRect(x, y, r) { // 0..512 square with corner radius r
  const cx = Math.min(Math.max(x, r), 512 - r), cy = Math.min(Math.max(y, r), 512 - r);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
}
function inPotato(x, y) { // lumpy ellipse
  const dx = (x - 256) / 150, dy = (y - 256) / 138;
  const a = Math.atan2(dy, dx);
  return dx * dx + dy * dy <= 1 + 0.06 * Math.sin(3 * a + 0.7) + 0.04 * Math.sin(5 * a);
}
const circle = (x, y, cx, cy, r) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
function smile(x, y) {
  const dx = x - 256, dy = y - 262, d = Math.hypot(dx, dy);
  return dy > 6 && d > 36 && d < 50 && Math.abs(dx) < 44;
}

function sample(x, y, rounded) {
  if (rounded && !inRoundRect(x, y, 112)) return [0, 0, 0, 0];
  if (circle(x, y, 206, 236, 16) || circle(x, y, 306, 236, 16) || smile(x, y)) return [28, 28, 28, 255];
  if (circle(x, y, 176, 176, 9) || circle(x, y, 338, 338, 9)) return [199, 199, 199, 255];
  if (inPotato(x, y)) return [229, 229, 229, 255];
  return [28, 28, 28, 255];
}

function render(size, rounded) {
  const ss = 3; // supersampling
  const buf = Buffer.alloc(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < ss; sy++) {
        for (let sx = 0; sx < ss; sx++) {
          const x = ((px + (sx + 0.5) / ss) / size) * 512, y = ((py + (sy + 0.5) / ss) / size) * 512;
          const c = sample(x, y, rounded);
          r += c[0] * c[3]; g += c[1] * c[3]; b += c[2] * c[3]; a += c[3];
        }
      }
      const o = (py * size + px) * 4;
      if (a > 0) { buf[o] = Math.round(r / a); buf[o + 1] = Math.round(g / a); buf[o + 2] = Math.round(b / a); }
      buf[o + 3] = Math.round(a / (ss * ss));
    }
  }
  return png(size, size, buf);
}

writeFileSync(join(out, 'icon-192.png'), render(192, true));
writeFileSync(join(out, 'icon-512.png'), render(512, true));
writeFileSync(join(out, 'apple-touch-icon.png'), render(180, false));
console.log('icons written to', out);
