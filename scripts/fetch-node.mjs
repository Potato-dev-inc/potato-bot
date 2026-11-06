#!/usr/bin/env node
/**
 * Release build step: puts the official Node.js (win-x64) inside the app, so a person never has to install Node.
 *
 *   https://nodejs.org/dist/v<NODE_VERSION>/node-v<NODE_VERSION>-win-x64.zip
 *     -> checked against https://nodejs.org/dist/v<NODE_VERSION>/SHASUMS256.txt
 *     -> cached outside the repo (%LOCALAPPDATA%\potato-bot-build-cache, or PM_BUILD_CACHE)
 *     -> packages/app/src-tauri/resources/node/node.exe           (gitignored, never committed)
 *     -> packages/app/src-tauri/resources/legal/LICENSES/Node.txt (Node's licence text, shipped in the legal folder)
 *
 * The version is pinned here: change NODE_VERSION on purpose, never by accident. Only Windows builds need it
 * (the app is Windows-only); PM_SKIP_NODE=1 skips it for a build that is not for release.
 */
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const NODE_VERSION = '22.20.0';
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const res = join(root, 'packages/app/src-tauri/resources');
const base = `https://nodejs.org/dist/v${NODE_VERSION}`;
const zipName = `node-v${NODE_VERSION}-win-x64.zip`;

if (process.env.PM_SKIP_NODE === '1') {
  console.error('[fetch-node] PM_SKIP_NODE=1: skipped');
  process.exit(0);
}

const cacheRoot = process.env.PM_BUILD_CACHE || join(process.env.LOCALAPPDATA || join(homedir(), '.cache'), 'potato-bot-build-cache');
const cache = join(cacheRoot, `node-v${NODE_VERSION}`);
const sha = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

async function get(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return Buffer.from(await r.arrayBuffer());
}

mkdirSync(cache, { recursive: true });
const zip = join(cache, zipName);
const sums = (await get(`${base}/SHASUMS256.txt`)).toString('utf8');
const line = sums.split(/\r?\n/).find((l) => l.trim().endsWith(`  ${zipName}`));
if (!line) throw new Error(`${zipName} is not in SHASUMS256.txt`);
const expected = line.trim().split(/\s+/)[0].toLowerCase();

if (!existsSync(zip) || sha(zip) !== expected) {
  console.error(`[fetch-node] downloading ${zipName}`);
  writeFileSync(`${zip}.part`, await get(`${base}/${zipName}`));
  if (sha(`${zip}.part`) !== expected) {
    rmSync(`${zip}.part`, { force: true });
    throw new Error(`${zipName}: sha256 does not match SHASUMS256.txt`);
  }
  renameSync(`${zip}.part`, zip);
}

const unpacked = join(cache, 'unpacked');
if (!existsSync(join(unpacked, `node-v${NODE_VERSION}-win-x64`, 'node.exe'))) {
  rmSync(unpacked, { recursive: true, force: true });
  mkdirSync(unpacked, { recursive: true });
  // Windows' own bsdtar (a GNU tar earlier on PATH reads "C:" as a host name).
  const tar = join(process.env.SystemRoot || 'C:/Windows', 'System32', 'tar.exe');
  const r = spawnSync(existsSync(tar) ? tar : 'tar', ['-xf', zip, '-C', unpacked], { stdio: 'inherit' });
  if (r.status !== 0) throw new Error('could not unpack the Node.js zip (tar -xf failed)');
}
const folder = join(unpacked, `node-v${NODE_VERSION}-win-x64`);
const nodeExe = join(folder, 'node.exe');

// Node's own SHASUMS file also lists the bare exe: check the unpacked one against it as well.
const exeLine = sums.split(/\r?\n/).find((l) => l.trim().endsWith('  win-x64/node.exe'));
if (exeLine && sha(nodeExe) !== exeLine.trim().split(/\s+/)[0].toLowerCase()) throw new Error('node.exe does not match SHASUMS256.txt');

mkdirSync(join(res, 'node'), { recursive: true });
cpSync(nodeExe, join(res, 'node', 'node.exe'));
mkdirSync(join(res, 'legal', 'LICENSES'), { recursive: true });
cpSync(join(folder, 'LICENSE'), join(res, 'legal', 'LICENSES', 'Node.txt'));
console.error(`[fetch-node] node ${NODE_VERSION} win-x64 staged (sha256 ${expected.slice(0, 12)}...)`);
