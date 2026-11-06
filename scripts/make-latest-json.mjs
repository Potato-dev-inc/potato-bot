#!/usr/bin/env node
/**
 * After a signed release build: writes latest.json (what the app's updater reads) next to the MSI to upload.
 *
 *   node scripts/make-latest-json.mjs <bundle-msi-folder> <out-folder>
 *
 * Reads `Potato Bot_<version>_x64_en-US.msi` and its `.sig` (made by `tauri build` when createUpdaterArtifacts is on and
 * TAURI_SIGNING_PRIVATE_KEY is set), copies the MSI to `Potato-Bot_<version>_x64_en-US.msi` (the name to upload to the
 * GitHub release v<version>) and writes `latest.json` with the version, the notes from CHANGELOG.md, the signature and
 * the asset URL. The private key is never read here: only the finished .sig file is.
 */
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const [bundleDir, outDir] = process.argv.slice(2);
if (!bundleDir || !outDir) {
  console.error('usage: node scripts/make-latest-json.mjs <bundle-msi-folder> <out-folder>');
  process.exit(2);
}
const conf = JSON.parse(readFileSync(join(root, 'packages/app/src-tauri/tauri.conf.json'), 'utf8'));
const version = conf.version;
const built = join(bundleDir, `Potato Bot_${version}_x64_en-US.msi`);
const sig = `${built}.sig`;
if (!existsSync(built) || !existsSync(sig)) {
  console.error(`missing ${built} or its .sig (was TAURI_SIGNING_PRIVATE_KEY set for the build?)`);
  process.exit(1);
}
const asset = `Potato-Bot_${version}_x64_en-US.msi`;
const changelog = readFileSync(join(root, 'CHANGELOG.md'), 'utf8');
const section = new RegExp(`^## ${version.replace(/\./g, '\\.')}\\s*\\n([\\s\\S]*?)(?=^## |$(?![\\s\\S]))`, 'm').exec(changelog);
const notes = (section?.[1] ?? '').trim();

mkdirSync(outDir, { recursive: true });
copyFileSync(built, join(outDir, asset));
const latest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature: readFileSync(sig, 'utf8').trim(),
      url: `https://github.com/Potato-dev-inc/potato-bot/releases/download/v${version}/${asset}`,
    },
  },
};
writeFileSync(join(outDir, 'latest.json'), JSON.stringify(latest, null, 2) + '\n');
console.error(`[make-latest-json] ${asset} + latest.json for ${version} written to ${outDir}`);
