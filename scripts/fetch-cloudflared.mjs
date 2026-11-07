#!/usr/bin/env node
/**
 * Release build step: puts the official cloudflared (win-x64) inside the app, so the phone's "away from home"
 * tunnel works without the person installing anything.
 *
 *   https://github.com/cloudflare/cloudflared/releases/download/<VERSION>/cloudflared-windows-amd64.exe
 *     -> checked against the hard-coded sha256 below
 *     -> cached outside the repo (%LOCALAPPDATA%\potato-bot-build-cache, or PM_BUILD_CACHE)
 *     -> packages/app/src-tauri/resources/bin/cloudflared.exe              (gitignored, never committed)
 *     -> packages/app/src-tauri/resources/legal/LICENSES/cloudflared.txt   (Apache-2.0 text from that tag)
 *
 * Version and hash are pinned here: change them together, on purpose, never by accident.
 * Where the hash came from: the `digest` field GitHub shows for this release asset
 * (GET https://api.github.com/repos/cloudflare/cloudflared/releases/tags/2026.10.7, asset
 * cloudflared-windows-amd64.exe, size 55390512), read on 2026-11-07 and then confirmed by hashing the download.
 * cloudflared publishes no NOTICE file at this tag; its LICENSE (Apache-2.0) is the only licence file in the repo root.
 * PM_SKIP_CLOUDFLARED=1 skips the step for a build that is not for release.
 */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync, copyFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const CLOUDFLARED_VERSION = '2026.10.7';
export const CLOUDFLARED_SHA256 = '2493dbc168923bb3e59e5e7716f71deced92992eea9acd6e9d74f84ac7751343';
export const CLOUDFLARED_SIZE = 55390512;

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const res = join(root, 'packages/app/src-tauri/resources');
const assetName = 'cloudflared-windows-amd64.exe';

if (process.env.PM_SKIP_CLOUDFLARED === '1') {
  console.error('[fetch-cloudflared] PM_SKIP_CLOUDFLARED=1: skipped');
  process.exit(0);
}

const cacheRoot = process.env.PM_BUILD_CACHE || join(process.env.LOCALAPPDATA || join(homedir(), '.cache'), 'potato-bot-build-cache');
const cache = join(cacheRoot, `cloudflared-${CLOUDFLARED_VERSION}`);
const sha = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

async function get(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return Buffer.from(await r.arrayBuffer());
}

mkdirSync(cache, { recursive: true });
const exe = join(cache, assetName);
if (!existsSync(exe) || sha(exe) !== CLOUDFLARED_SHA256) {
  console.error(`[fetch-cloudflared] downloading ${assetName} ${CLOUDFLARED_VERSION}`);
  const bytes = await get(`https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/${assetName}`);
  writeFileSync(`${exe}.part`, bytes);
  const got = sha(`${exe}.part`);
  if (got !== CLOUDFLARED_SHA256) {
    rmSync(`${exe}.part`, { force: true });
    throw new Error(`${assetName}: sha256 ${got} does not match the pinned ${CLOUDFLARED_SHA256}; refusing to stage it`);
  }
  renameSync(`${exe}.part`, exe);
}

const licence = join(cache, 'LICENSE');
if (!existsSync(licence)) {
  const text = await get(`https://raw.githubusercontent.com/cloudflare/cloudflared/${CLOUDFLARED_VERSION}/LICENSE`);
  if (!text.toString('utf8').includes('Apache License')) throw new Error('the cloudflared LICENSE is not the Apache-2.0 text');
  writeFileSync(licence, text);
}

mkdirSync(join(res, 'bin'), { recursive: true });
copyFileSync(exe, join(res, 'bin', 'cloudflared.exe'));
mkdirSync(join(res, 'legal', 'LICENSES'), { recursive: true });
copyFileSync(licence, join(res, 'legal', 'LICENSES', 'cloudflared.txt'));
console.error(`[fetch-cloudflared] cloudflared ${CLOUDFLARED_VERSION} win-x64 staged (sha256 ${CLOUDFLARED_SHA256.slice(0, 12)}...)`);
