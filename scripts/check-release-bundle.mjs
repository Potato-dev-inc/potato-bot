#!/usr/bin/env node
/**
 * Release check, run at build time (after the resources are staged): fails when a file the app needs at run time
 * would not be in the installer. It reads what the code needs (the sandbox's REQUIRED_FILES in install.rs, the
 * persona packs and recipes in the packages) and checks each is staged under src-tauri/resources AND matched by a
 * glob in tauri.conf.json `bundle.resources`. The 1.0.1 installer lacked the "pro" persona because the list named
 * packs one by one; this would have stopped that build.
 */
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const tauri = join(root, 'packages/app/src-tauri');
const res = join(tauri, 'resources');
const conf = JSON.parse(readFileSync(join(tauri, 'tauri.conf.json'), 'utf8'));
const patterns = (Array.isArray(conf.bundle.resources) ? conf.bundle.resources : Object.keys(conf.bundle.resources)).map((p) => {
  // A tiny glob: "**/*" is anything below, "*" is one path piece; every other character is literal.
  let re = '';
  for (let i = 0; i < p.length; ) {
    if (p.startsWith('**/*', i)) {
      re += '.*';
      i += 4;
    } else if (p[i] === '*') {
      re += '[^/]*';
      i += 1;
    } else {
      re += '[]\\^$.|?+(){}'.includes(p[i]) ? '\\' + p[i] : p[i];
      i += 1;
    }
  }
  return new RegExp(`^${re}$`);
});
const problems = [];
const need = (rel) => {
  const path = join(tauri, rel);
  if (!existsSync(path)) problems.push(`not staged: ${rel}`);
  else if (!patterns.some((re) => re.test(rel))) problems.push(`staged but no tauri.conf.json resources glob covers it: ${rel}`);
};
const walk = (dir) =>
  readdirSync(dir).flatMap((n) => (statSync(join(dir, n)).isDirectory() ? walk(join(dir, n)) : [join(dir, n)]));

// The brain.
need('resources/potato-brain/brain.mjs');
need('resources/potato-brain/persona/protocol.md');
const packs = join(root, 'packages/potato-brain/persona/packs');
for (const pack of readdirSync(packs)) {
  for (const f of readdirSync(join(packs, pack))) need(`resources/potato-brain/persona/packs/${pack}/${f}`);
}
// The sandbox: what the code requires, every staged file, and recipes that parse.
const install = readFileSync(join(tauri, 'src/potato/sandbox/install.rs'), 'utf8');
const block = /const REQUIRED_FILES[^=]*=\s*\[([^\]]*)\]/.exec(install);
if (!block) problems.push('could not read REQUIRED_FILES from install.rs');
for (const m of (block?.[1] ?? '').matchAll(/"([^"]+)"/g)) need(`resources/potato-sandbox/${m[1]}`);
need('resources/potato-sandbox/images.json');
need('resources/potato-sandbox/viewer/embed.html');
need('resources/potato-sandbox/viewer/novnc/LICENSE.txt');
need('resources/potato-sandbox/LICENSE-APACHE-2.0');
for (const f of walk(join(res, 'potato-sandbox'))) need(relative(tauri, f).split(sep).join('/'));
for (const f of readdirSync(join(res, 'potato-sandbox/recipes')).filter((n) => n.endsWith('.json'))) {
  try {
    JSON.parse(readFileSync(join(res, 'potato-sandbox/recipes', f), 'utf8'));
  } catch (e) {
    problems.push(`recipe does not parse: ${f}`);
  }
}
// Node.js, so nobody has to install it (fetch-node.mjs puts it here).
need('resources/node/node.exe');
// The licences.
for (const f of ['LICENSES/Node.txt', 'LICENSE', 'THIRD-PARTY-NOTICES.md', 'LICENSES/MIT-desktoplab.txt', 'LICENSES/MPL-2.0.txt', 'LICENSES/Apache-2.0.txt']) need(`resources/legal/${f}`);

if (problems.length) {
  console.error('[check-release-bundle] the installer would be missing files:\n  ' + problems.join('\n  '));
  process.exit(1);
}
console.error('[check-release-bundle] ok: every file the app needs is staged and covered by tauri.conf.json');
