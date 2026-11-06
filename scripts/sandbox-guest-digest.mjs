// The digest of the sandbox guest files, the same one the app computes in GuestFiles::digest (install.rs):
// sha256 of a ustar stream of the shipped files, with Windows line endings taken out.
// A prebuilt image is used only when its manifest entry carries this digest, so the two must stay byte for byte equal.
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const GUEST_FOLDERS = ['desktop', 'control', 'egress', 'page-browser'];
const shipped = (name) => !(name.endsWith('.md') || name.startsWith('test_') || name.endsWith('.pyc'));

function collect(dir, prefix, out) {
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) collect(path, `${prefix}/${name}`, out);
    else if (shipped(name)) out.push([`${prefix}/${name}`, path]);
  }
}

function stripCr(bytes) {
  const out = Buffer.alloc(bytes.length);
  let n = 0;
  for (let i = 0; i < bytes.length; i++) {
    if (bytes[i] === 13 && bytes[i + 1] === 10) continue;
    out[n++] = bytes[i];
  }
  return out.subarray(0, n);
}

function tarHeader(name, size) {
  const header = Buffer.alloc(512);
  const octal = (start, end, value) => header.write(value.toString(8).padStart(end - start - 1, '0'), start, 'latin1');
  header.write(name, 0, 'latin1');
  const executable = !(name.endsWith('.json') || name.endsWith('.conf') || name.endsWith('.desktop'));
  octal(100, 108, executable ? 0o755 : 0o644);
  octal(108, 116, 0);
  octal(116, 124, 0);
  octal(124, 136, size);
  octal(136, 148, 0);
  header.write('        ', 148, 'latin1');
  header[156] = 0x30;
  header.write('ustar\0', 257, 'latin1');
  header.write('00', 263, 'latin1');
  const sum = header.reduce((total, byte) => total + byte, 0);
  header.write(sum.toString(8).padStart(6, '0'), 148, 'latin1');
  header[154] = 0;
  header[155] = 0x20;
  return header;
}

/** `root` holds guest/ and recipes/ (packages/potato-sandbox, or its staged copy). */
export function sandboxGuestDigest(root) {
  const files = [];
  for (const folder of GUEST_FOLDERS) collect(join(root, 'guest', folder), folder, files);
  collect(join(root, 'recipes'), 'recipes', files);
  files.sort((a, b) => Buffer.compare(Buffer.from(a[0]), Buffer.from(b[0])));
  const hash = createHash('sha256');
  for (const [name, path] of files) {
    const data = stripCr(readFileSync(path));
    hash.update(tarHeader(name, data.length));
    hash.update(data);
    hash.update(Buffer.alloc((512 - (data.length % 512)) % 512));
  }
  hash.update(Buffer.alloc(1024));
  return hash.digest('hex');
}
