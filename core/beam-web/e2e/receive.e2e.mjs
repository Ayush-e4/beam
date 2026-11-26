// The native peer plays the phone and sends a folder and a file to the engine in a dedicated
// worker (worker.mjs) in headless Chromium, over WebTransport. The files land in OPFS and come
// back as `File`s with the right names and bytes. Also checks that a declined offer reaches the
// phone. Usage (tools/web-engine-test.sh runs it):
//   node receive.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <webtransport_peer binary>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { createHash, randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright-core';

import { ask, check, offered, openPage, paired, startPeer, startWorker } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);
const MIB = 1024 * 1024;
const TRANSFER_MS = 120_000;

// A folder with an incompressible file, a compressible one, an empty one and a nested small
// one, plus a loose file. Returns their paths and the sha256 each should arrive with.
async function makeFiles() {
  const dir = await mkdtemp(path.join(tmpdir(), 'beam-receive-'));
  const files = {
    'trip/video.bin': randomBytes(24 * MIB),
    'trip/notes.txt': Buffer.from('beam '.repeat((3 * MIB) / 5)),
    'trip/photos/empty.jpg': Buffer.alloc(0),
    'trip/photos/small.txt': Buffer.from('a small note'),
    'hello.txt': Buffer.from('hello from the phone'),
  };
  for (const [name, bytes] of Object.entries(files)) {
    await mkdir(path.dirname(path.join(dir, name)), { recursive: true });
    await writeFile(path.join(dir, name), bytes);
  }
  const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
  const expected = Object.fromEntries(Object.entries(files).map(([k, v]) => [k, sha256(v)]));
  const total = Object.values(files).reduce((sum, bytes) => sum + bytes.length, 0);
  return { dir, send: [path.join(dir, 'trip'), path.join(dir, 'hello.txt')], expected, total };
}

async function receivesFiles(page, files) {
  const { invite } = await ask(page, { op: 'invite' });
  const peer = startPeer(peerBin, [invite, 'send', ...files.send]);
  try {
    const result = await ask(page, { op: 'receive', sealed: await offered(peer) });
    check(!result.failed, result.failed ? `receive: ${result.failed}\n${peer.stderr()}` : 'received');
    const { code, browserName } = await paired(peer);
    check(result.code === code, 'both sides show the same code');
    check(result.phone === 'Test phone' && browserName === 'Test browser', 'each learns the other\'s name');
    const sent = await peer.line('SENT outcome=', TRANSFER_MS);
    check(sent.startsWith('Delivered '), `the phone hears the files were delivered (${sent})`);

    const names = Object.keys(files.expected).sort();
    const listed = result.offered.map((f) => f.path).sort();
    check(JSON.stringify(listed) === JSON.stringify(names), `the offer lists ${names.length} files`);
    for (const file of result.files) {
      check(file.sha256 === files.expected[file.path], `${file.path} arrived intact`);
      check(file.name === path.basename(file.path), `${file.path} is named ${file.name}`);
    }
    check(result.files.length === names.length, 'every file came back from OPFS');
    const last = result.progress.at(-1);
    check(
      result.progress.length > 1 && last.done === files.total && last.total === files.total,
      `progress reached ${files.total} bytes in ${result.progress.length} reports`,
    );
    const mbps = files.total / MIB / (result.ms / 1000);
    console.log(`# ${(files.total / MIB).toFixed(1)} MiB in ${Math.round(result.ms)} ms (${mbps.toFixed(0)} MiB/s, loopback)`);
  } finally {
    peer.kill();
  }
}

async function declines(page, files) {
  const { invite } = await ask(page, { op: 'invite' });
  const peer = startPeer(peerBin, [invite, 'send', ...files.send]);
  try {
    const result = await ask(page, { op: 'receive', sealed: await offered(peer), decline: true });
    check(result.declined, result.failed ?? 'the browser declined');
    await paired(peer);
    const sent = await peer.line('SENT outcome=');
    check(sent.startsWith('Rejected(UserDeclined)'), `the phone hears it was declined (${sent})`);
  } finally {
    peer.kill();
  }
}

const files = await makeFiles();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const page = await openPage(browser, pkgDir);
  await startWorker(page);
  await receivesFiles(page, files);
  await declines(page, files);
} finally {
  await browser.close();
  await rm(files.dir, { recursive: true, force: true });
}
