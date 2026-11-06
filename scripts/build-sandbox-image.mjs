#!/usr/bin/env node
/**
 * Build a prebuilt sandbox distro image for one desktop (WSL2 Ubuntu 24.04), so the app can import it
 * instead of running every recipe on the maintainer's PC.
 *
 *   node scripts/build-sandbox-image.mjs --desktop light|zorin|mint --out <dir> [--dry-run]
 *
 * Steps, in order (each one is logged with the seconds it took):
 *   1. check zstd is on PATH (fails here, before any WSL work, when it is not)
 *   2. wsl --install Ubuntu-24.04 --name pm-sbx-img-<desktop>-<random> --location <work> --no-launch
 *   3. copy the guest files over stdin as a tar (guest/{desktop,control,egress,page-browser} and recipes/), as root
 *   4. copy the install-time wsl-install.conf to /etc/wsl.conf (the same file the app puts in place), then wsl --terminate
 *   5. run apply-recipe.py --image-build as root with the desktop's recipes
 *   6. read /etc/pm-sbx/image.json back, then wsl --terminate
 *   7. wsl --export <distro> <work>/export.tar, zstd -T0 -19 into <out>/<desktop>.tar.zst
 *   8. sha256 and size of the .tar.zst, written to <out>/<desktop>.json and printed
 *   finally: wsl --unregister of the temp distro (only a pm-sbx-img- name is ever unregistered), and the work folder removed.
 *
 * No shell (every command is an argv array), no new dependencies, no visible windows (windowsHide). --dry-run prints
 * the commands and changes nothing.
 */
import { spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { createReadStream, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sandboxGuestDigest } from './sandbox-guest-digest.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PKG = join(ROOT, 'packages/potato-sandbox');
const DISTRO_IMAGE = 'Ubuntu-24.04';
const NAME_PREFIX = 'pm-sbx-img-';
const GUEST_SHARE = '/usr/local/share/pm-sbx';
const GUEST_FOLDERS = ['desktop', 'control', 'egress', 'page-browser'];
const INSTALL_CONF = `${GUEST_SHARE}/desktop/wsl-install.conf`;
// The recipes baked into each prebuilt desktop image. Chrome is NOT here: the app installs it on the user's PC
// after import through the normal ApplyRecipes run, so the image stays free of it.
const RECIPE_SETS = {
  light: ['base', 'light'],
  zorin: ['base', 'xfce', 'zorin-look'],
  mint: ['base', 'mint'],
};

let dryRun = false;

function log(message) {
  console.log(`[build-sandbox-image] ${message}`);
}

function usage(message) {
  if (message) console.error(`[build-sandbox-image] ${message}`);
  console.error('usage: node scripts/build-sandbox-image.mjs --desktop light|zorin|mint --out <dir> [--extra-recipe <id>]... [--opt <key=value>]... [--dry-run]');
  process.exit(2);
}

function parseArgs(argv) {
  const args = { desktop: '', out: '', dryRun: false, extra: [], opts: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--dry-run') args.dryRun = true;
    else if (arg === '--desktop') args.desktop = argv[++i] ?? '';
    else if (arg === '--out') args.out = argv[++i] ?? '';
    // For an image kept on one's own PC only: Chrome's licence is not cleared for a hosted image.
    else if (arg === '--extra-recipe') args.extra.push(argv[++i] ?? '');
    else if (arg === '--opt') args.opts.push(argv[++i] ?? '');
    else usage(`unknown argument ${arg}`);
  }
  if (!RECIPE_SETS[args.desktop]) usage(`--desktop must be one of ${Object.keys(RECIPE_SETS).join(', ')}`);
  if (!args.out) usage('--out <dir> is required');
  return args;
}

/** The temp distro name is the only thing that is ever unregistered: refuse anything else. */
function assertTempName(name) {
  if (!name.startsWith(NAME_PREFIX) || !/^pm-sbx-img-[a-z0-9-]+$/.test(name)) {
    throw new Error(`refusing to touch a distro that is not a temp image: ${name}`);
  }
}

/** wsl.exe writes UTF-16 for its own messages; the distro's programs write UTF-8. */
function decodeOutput(buffer) {
  if (buffer.length >= 2 && buffer.indexOf(0) > 0 && buffer[1] === 0) {
    return buffer.toString('utf16le').replace(/\0/g, '');
  }
  return buffer.toString('utf8');
}

function shellQuote(arg) {
  return /^[A-Za-z0-9_./:=@-]+$/.test(arg) ? arg : `'${arg.replace(/'/g, "'\\''")}'`;
}

/** Runs one program with an argv (never a shell). In a dry run it only prints the command. */
function run(program, args, { input, label } = {}) {
  if (dryRun) {
    const extra = input ? ` (stdin: ${input.length} bytes)` : '';
    log(`+ ${[program, ...args].map(shellQuote).join(' ')}${extra}`);
    return Promise.resolve({ stdout: '' });
  }
  return new Promise((resolvePromise, reject) => {
    const child = spawn(program, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    const out = [];
    const err = [];
    child.stdout.on('data', (chunk) => out.push(chunk));
    child.stderr.on('data', (chunk) => err.push(chunk));
    child.on('error', (error) => reject(new Error(`${label || program} could not start: ${error.message}`)));
    child.on('close', (code) => {
      const stdout = decodeOutput(Buffer.concat(out));
      if (code === 0) {
        resolvePromise({ stdout });
        return;
      }
      const tail = (text) => text.trim().split(/\r?\n/).slice(-6).join(' | ').slice(-600);
      reject(new Error(`${label || program} exited with ${code}: ${tail(decodeOutput(Buffer.concat(err)))} ${tail(stdout)}`));
    });
    child.stdin.on('error', () => {}); // a program that does not read stdin closes it early: that is its own exit code
    child.stdin.end(input);
  });
}

function wsl(args, options) {
  return run('wsl', args, options);
}

/** Runs a program inside the distro as root, the way install.rs does (`-d <name> -u root --exec`). */
function execRoot(name, argv, options = {}) {
  return wsl(['-d', name, '-u', 'root', '--exec', ...argv], { label: argv[0], ...options });
}

async function step(title, fn) {
  const started = Date.now();
  log(`> ${title}`);
  const result = await fn();
  log(`< ${title}: ${((Date.now() - started) / 1000).toFixed(1)}s`);
  return result;
}

/** Same rules as install.rs GuestFiles::list: the shipped files only (no .md, no test_*, no .pyc). */
function shipped(fileName) {
  return !(fileName.endsWith('.md') || fileName.startsWith('test_') || fileName.endsWith('.pyc'));
}

function collect(dir, prefix, out) {
  const entries = readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1));
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) collect(path, `${prefix}/${entry.name}`, out);
    else if (shipped(entry.name)) out.push([`${prefix}/${entry.name}`, path]);
  }
  return out;
}

function guestFileList() {
  const files = [];
  for (const folder of GUEST_FOLDERS) collect(join(PKG, 'guest', folder), folder, files);
  collect(join(PKG, 'recipes'), 'recipes', files);
  return files.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
}

function tarHeader(name, size) {
  const header = Buffer.alloc(512, 0);
  header.write(name, 0, 'ascii');
  const executable = !(name.endsWith('.json') || name.endsWith('.conf') || name.endsWith('.desktop'));
  const octal = (value, width) => `${value.toString(8).padStart(width - 1, '0')}\0`;
  header.write(octal(executable ? 0o755 : 0o644, 8), 100, 'ascii');
  header.write(octal(0, 8), 108, 'ascii');
  header.write(octal(0, 8), 116, 'ascii');
  header.write(octal(size, 12), 124, 'ascii');
  header.write(octal(0, 12), 136, 'ascii');
  header.write('        ', 148, 'ascii');
  header.write('0', 156, 'ascii');
  header.write('ustar\0', 257, 'ascii');
  header.write('00', 263, 'ascii');
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 'ascii');
  return header;
}

/** A ustar archive of the guest files, Windows line endings taken out (as GuestFiles::tar_stream does). */
function buildGuestTar() {
  const parts = [];
  let count = 0;
  for (const [name, path] of guestFileList()) {
    if (name.length > 100 || !/^[\x20-\x7e]+$/.test(name) || name.startsWith('/') || name.includes('..')) {
      throw new Error(`bad name in the guest archive: ${name}`);
    }
    const data = readFileSync(path).toString('latin1');
    const bytes = Buffer.from(data.replace(/\r\n/g, '\n'), 'latin1');
    const padded = Math.ceil(bytes.length / 512) * 512;
    parts.push(tarHeader(name, bytes.length), bytes, Buffer.alloc(padded - bytes.length, 0));
    count += 1;
  }
  parts.push(Buffer.alloc(1024, 0));
  return { tar: Buffer.concat(parts), count };
}

function sha256File(path) {
  return new Promise((resolvePromise, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(path);
    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolvePromise(hash.digest('hex')));
  });
}

async function requireZstd() {
  try {
    await run('zstd', ['--version'], { label: 'zstd' });
  } catch (error) {
    throw new Error(`zstd is not on PATH (${error.message}). Install zstd and put it on PATH, then run this again.`);
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  dryRun = args.dryRun;
  const outDir = resolve(args.out);
  const recipes = [...RECIPE_SETS[args.desktop], ...args.extra];
  const name = `${NAME_PREFIX}${args.desktop}-${randomBytes(4).toString('hex')}`;
  assertTempName(name);
  const work = join(outDir, '.work', name);
  const tarPath = join(work, 'export.tar');
  const finalName = `${args.desktop}.tar.zst`;
  const finalPath = join(outDir, finalName);
  const jsonPath = join(outDir, `${args.desktop}.json`);
  log(`desktop ${args.desktop}: recipes ${recipes.join(',')}; temp distro ${name}${dryRun ? ' (dry run)' : ''}`);

  if (!dryRun) {
    await step('check zstd', requireZstd);
    mkdirSync(work, { recursive: true });
  } else {
    log('+ zstd --version (checked for real before a build)');
  }

  let registered = false;
  let guestDigest = null;
  try {
    await step('install the temp distro', async () => {
      await wsl(['--install', DISTRO_IMAGE, '--name', name, '--location', work, '--no-launch'], { label: 'wsl --install' });
      registered = true;
    });

    const guest = dryRun ? { tar: Buffer.alloc(0), count: guestFileList().length } : buildGuestTar();
    await step(`copy ${guest.count} guest files`, async () => {
      await execRoot(name, ['/bin/mkdir', '-p', GUEST_SHARE]);
      if (dryRun) {
        log(`+ (tar of ${guest.count} guest files) | wsl -d ${name} -u root --exec /bin/tar -x -m -C ${GUEST_SHARE} -f -`);
        return;
      }
      await execRoot(name, ['/bin/tar', '-x', '-m', '-C', GUEST_SHARE, '-f', '-'], { input: guest.tar });
    });

    await step('install-time wsl.conf, then restart', async () => {
      await execRoot(name, ['/bin/cp', INSTALL_CONF, '/etc/wsl.conf']);
      await wsl(['--terminate', name], { label: 'wsl --terminate' });
    });

    // Extras go on top of the finished desktop, in the order an install on a PC uses: the coding recipe adds a package
    // source early, and the base recipe's packages cannot be resolved with that source already in place.
    if (args.extra.length > 0) {
      await step('apply the desktop recipes first', async () => {
        const argv = ['/usr/bin/python3', '-I', `${GUEST_SHARE}/desktop/apply-recipe.py`];
        for (const recipe of RECIPE_SETS[args.desktop]) argv.push('--recipe', recipe);
        await execRoot(name, argv, { label: 'apply-recipe.py' });
      });
    }

    await step(`apply recipes ${recipes.join(',')} (image build)`, async () => {
      const argv = ['/usr/bin/python3', '-I', `${GUEST_SHARE}/desktop/apply-recipe.py`, '--image-build'];
      for (const recipe of recipes) argv.push('--recipe', recipe);
      for (const opt of args.opts) argv.push('--opt', opt);
      const { stdout } = await execRoot(name, argv, { label: 'apply-recipe.py' });
      const result = stdout.split(/\r?\n/).filter((line) => line.startsWith('RESULT ')).pop();
      if (result) log(result.slice(0, 400));
    });

    await step('restart', () => wsl(['--terminate', name], { label: 'wsl --terminate' }));

    await step('read the image record', async () => {
      if (dryRun) {
        log(`+ wsl -d ${name} -u root --exec /bin/cat /etc/pm-sbx/image.json`);
        return;
      }
      await execRoot(name, ['/bin/cat', '/etc/pm-sbx/image.json']);
      // The app compares this digest with its own, so it comes from the same recipe, not from the guest's record.
      guestDigest = sandboxGuestDigest(PKG);
    });

    await step('export the distro', () => wsl(['--export', name, tarPath], { label: 'wsl --export' }));

    await step('compress with zstd -T0 -19', () =>
      run('zstd', ['-T0', '-19', '-q', '-f', tarPath, '-o', finalPath], { label: 'zstd' }));
  } finally {
    if (registered) {
      try {
        assertTempName(name);
        await wsl(['--unregister', name], { label: 'wsl --unregister' });
        if (!dryRun) log(`unregistered ${name}`);
      } catch (error) {
        log(`could not unregister ${name}: ${error.message}`);
      }
    }
    if (!dryRun) rmSync(work, { recursive: true, force: true });
  }

  if (dryRun) {
    log(`would write ${jsonPath}`);
    return;
  }
  const size = statSync(finalPath).size;
  const sha256 = await sha256File(finalPath);
  const record = {
    desktop: args.desktop,
    file: finalName,
    size,
    sha256,
    guest_digest: guestDigest,
    recipes,
    built_at: new Date().toISOString(),
  };
  writeFileSync(jsonPath, `${JSON.stringify(record, null, 2)}\n`, 'utf8');
  console.log(JSON.stringify(record, null, 2));
  log(`wrote ${finalPath} and ${jsonPath}`);
}

main().catch((error) => {
  console.error(`[build-sandbox-image] failed: ${error.message}`);
  process.exit(1);
});
