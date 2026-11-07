#!/usr/bin/env node
/**
 * Ensure Tauri bundle resource globs exist before `tauri dev` / `tauri build`.
 * `potato-brain` and `potato-sandbox` are gitignored and produced from their packages.
 */
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brainInputsHash, readStamp, sandboxInputsHash } from './resource-stamp.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const brainDir = join(root, 'packages/app/src-tauri/resources/potato-brain');
const sandboxDir = join(root, 'packages/app/src-tauri/resources/potato-sandbox');

function ensurePotatoBrain() {
  const brain = join(brainDir, 'brain.mjs');
  const persona = join(brainDir, 'persona');
  if (existsSync(brain) && existsSync(persona) && readStamp(brainDir) === brainInputsHash(root)) {
    return;
  }
  const result = spawnSync(process.execPath, [join(root, 'scripts', 'bundle-potato-brain.mjs')], {
    cwd: root,
    stdio: 'inherit',
  });
  if (result.status === 0) return;
  mkdirSync(persona, { recursive: true });
  writeFileSync(
    brain,
    "console.error('potato-brain stub: bundle it with node scripts/bundle-potato-brain.mjs');\nprocess.exit(2);\n",
    'utf-8',
  );
  console.error('[ensure-tauri-resources] potato-brain bundle failed - wrote a stub for tauri dev');
}

function ensurePotatoSandbox() {
  const ready =
    existsSync(join(sandboxDir, 'guest')) &&
    existsSync(join(sandboxDir, 'recipes', 'base.json')) &&
    existsSync(join(sandboxDir, 'viewer', 'embed.html')) &&
    existsSync(join(sandboxDir, 'viewer', 'novnc')) &&
    readStamp(sandboxDir) === sandboxInputsHash(root);
  if (ready) return;
  const result = spawnSync(process.execPath, [join(root, 'scripts', 'bundle-potato-sandbox.mjs')], {
    cwd: root,
    stdio: 'inherit',
  });
  if (result.status === 0) return;
  mkdirSync(join(sandboxDir, 'guest'), { recursive: true });
  mkdirSync(join(sandboxDir, 'recipes'), { recursive: true });
  mkdirSync(join(sandboxDir, 'viewer', 'novnc'), { recursive: true });
  for (const part of ['guest', 'recipes', 'viewer', 'viewer/novnc']) {
    writeFileSync(join(sandboxDir, part, 'stub.txt'), 'stub\n', 'utf-8');
  }
  console.error('[ensure-tauri-resources] potato-sandbox staging failed - wrote a stub for tauri dev');
}

/** The licence texts travel with the app: LICENSE, THIRD-PARTY-NOTICES.md and LICENSES/ go to resources/legal/. */
function ensureLegal() {
  const dest = join(root, 'packages/app/src-tauri/resources/legal');
  rmSync(dest, { recursive: true, force: true });
  mkdirSync(dest, { recursive: true });
  for (const name of ['LICENSE', 'THIRD-PARTY-NOTICES.md']) cpSync(join(root, name), join(dest, name));
  cpSync(join(root, 'LICENSES'), join(dest, 'LICENSES'), { recursive: true });
}

/** The phone web app (packages/app/phone) is served by the phone server from resources/phone/. Plain files, no build; the tools folder stays out. */
function ensurePhone() {
  const src = join(root, 'packages/app/phone');
  const dest = join(root, 'packages/app/src-tauri/resources/phone');
  rmSync(dest, { recursive: true, force: true });
  mkdirSync(dest, { recursive: true });
  cpSync(src, dest, { recursive: true, filter: (p) => !p.split(/[\\/]/).includes('tools') });
}

export function ensureTauriResources() {
  ensurePhone();
  ensureLegal();
  ensurePotatoBrain();
  ensurePotatoSandbox();
}

const invokedDirectly =
  process.argv[1] &&
  fileURLToPath(import.meta.url).replace(/\\/g, '/') === process.argv[1].replace(/\\/g, '/');

if (invokedDirectly) ensureTauriResources();
