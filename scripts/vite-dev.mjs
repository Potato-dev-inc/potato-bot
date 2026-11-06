#!/usr/bin/env node
/**
 * Tauri beforeDevCommand entry: build workspace deps, start Vite.
 */
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ensureWorkspaceBuilt } from './ensure-workspace-built.mjs';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const appDir = join(repoRoot, 'packages', 'app');

ensureWorkspaceBuilt(repoRoot);

const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const vite = spawn(npm, ['exec', 'vite'], {
  cwd: appDir,
  stdio: 'inherit',
  env: process.env,
  shell: true,
});

for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(signal, () => vite.kill(signal));
}

vite.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 0);
});
