// Files picked in a page (an <input type=file>) go to the engine in its dedicated worker
// (worker.mjs), which sends them over WebTransport to the native peer playing the phone. The
// phone saves them intact. Also checks a declined offer and a file changed after picking.
// Usage (tools/web-engine-test.sh runs it):
//   node send.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <webtransport_peer binary>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { createHash, randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright-core';

import { check, offered, openPage, paired, startPeer, startWorker } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);
const MIB = 1024 * 1024;
const TRANSFER_MS = 120_000;
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

// An incompressible file, a compressible one and an empty one, to pick.
async function makeFiles(dir) {
  const files = {
    'video.bin': randomBytes(24 * MIB),
    'notes.txt': Buffer.from('beam '.repeat((3 * MIB) / 5)),
    'empty.jpg': Buffer.alloc(0),
  };
  await mkdir(path.join(dir, 'picked'));
  for (const [name, bytes] of Object.entries(files)) {
    await writeFile(path.join(dir, 'picked', name), bytes);
  }
  const total = Object.values(files).reduce((sum, bytes) => sum + bytes.length, 0);
  return { files, paths: Object.keys(files).map((n) => path.join(dir, 'picked', n)), total };
}

// Picks `paths` in the page, as a person would, and hands the Files to the worker to send.
async function pickAndSend(page, paths, sealed) {
  await page.evaluate(() => {
    if (!document.querySelector('input')) {
      const input = document.createElement('input');
      input.type = 'file';
      input.multiple = true;
      document.body.append(input);
    }
  });
  await page.locator('input').setInputFiles(paths);
  return page.evaluate(
    (sealed) =>
      globalThis.ask({ op: 'send', sealed, files: [...document.querySelector('input').files] }),
    sealed,
  );
}

async function sendsFiles(page, dir, picked) {
  const inbox = path.join(dir, 'inbox');
  await mkdir(inbox);
  const { invite } = await page.evaluate(() => globalThis.ask({ op: 'invite' }));
  const peer = startPeer(peerBin, [invite, 'receive', inbox]);
  try {
    const result = await pickAndSend(page, picked.paths, await offered(peer));
    check(!result.failed, result.failed ? `send: ${result.failed}\n${peer.stderr()}` : 'sent');
    const { code, browserName } = await paired(peer);
    check(result.code === code, 'both sides show the same code');
    check(result.phone === 'Test phone' && browserName === 'Test browser', 'each learns the other\'s name');
    const files = await peer.line('OFFERED ');
    check(files === `files=3 bytes=${picked.total}`, `the phone is offered ${files}`);
    await peer.line('RECEIVED files=3', TRANSFER_MS);
    check(result.outcome === 'delivered', 'the browser hears the files were delivered');
    for (const [name, bytes] of Object.entries(picked.files)) {
      const saved = await readFile(path.join(inbox, name));
      check(sha256(saved) === sha256(bytes), `${name} arrived intact on the phone`);
    }
    const last = result.progress.at(-1);
    check(
      result.progress.length > 1 && last.done === picked.total && last.total === picked.total,
      `progress reached ${picked.total} bytes in ${result.progress.length} reports`,
    );
    const mbps = picked.total / MIB / (result.ms / 1000);
    console.log(
      `# ${(picked.total / MIB).toFixed(1)} MiB in ${Math.round(result.ms)} ms (${mbps.toFixed(0)} MiB/s, loopback)`,
    );
  } finally {
    peer.kill();
  }
}

async function declined(page, dir, picked) {
  const { invite } = await page.evaluate(() => globalThis.ask({ op: 'invite' }));
  const peer = startPeer(peerBin, [invite, 'receive', path.join(dir, 'unused'), '--decline']);
  try {
    const result = await pickAndSend(page, picked.paths, await offered(peer));
    check(result.outcome === 'declined', `the browser hears it was declined (${result.outcome})`);
  } finally {
    peer.kill();
  }
}

async function changedAfterPicking(page, dir, picked) {
  const inbox = path.join(dir, 'inbox-changed');
  await mkdir(inbox);
  const { invite } = await page.evaluate(() => globalThis.ask({ op: 'invite' }));
  const peer = startPeer(peerBin, [invite, 'receive', inbox]);
  try {
    const sealed = await offered(peer);
    await page.locator('input').setInputFiles(picked.paths);
    const video = picked.paths[0];
    await writeFile(video, randomBytes(24 * MIB)); // edited after it was picked
    await utimes(video, new Date(), new Date(Date.now() + 60_000));
    const result = await page.evaluate(
      (sealed) =>
        globalThis.ask({ op: 'send', sealed, files: [...document.querySelector('input').files] }),
      sealed,
    );
    check(
      result.failed?.startsWith('BeamError/storage'),
      `a file edited after picking fails as a storage error (${result.failed ?? result.outcome})`,
    );
  } finally {
    peer.kill();
  }
}

const dir = await mkdtemp(path.join(tmpdir(), 'beam-send-'));
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const picked = await makeFiles(dir);
  const page = await openPage(browser, pkgDir);
  await startWorker(page);
  await sendsFiles(page, dir, picked);
  await declined(page, dir, picked);
  await changedAfterPicking(page, dir, picked);
} finally {
  await browser.close();
  await rm(dir, { recursive: true, force: true });
}
