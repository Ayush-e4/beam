// What the browser end-to-end tests share: the native peer that plays the phone, a page that
// serves the engine, and the check that prints each result.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

// Never contacted: every request to it is answered by the route below.
export const ORIGIN = 'http://localhost:9';
const STEP_MS = 20_000;
const here = path.dirname(fileURLToPath(import.meta.url));

// Starts the peer (`webtransport_peer <args>`). `line(prefix)` waits for its next stdout line,
// which must start with `prefix`, and returns the rest.
export function startPeer(peerBin, args) {
  const peer = spawn(peerBin, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  const reader = createInterface({ input: peer.stdout });
  const seen = [];
  reader.on('line', (line) => seen.push(line));
  const lines = reader[Symbol.asyncIterator]();
  let stderr = '';
  peer.stderr.on('data', (chunk) => (stderr += chunk));
  const exited = new Promise((resolve) => peer.on('exit', (code) => resolve(code)));
  async function line(prefix, ms = STEP_MS) {
    let timer;
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`peer: no "${prefix}" line`)), ms);
    });
    const { value, done } = await Promise.race([lines.next(), timeout]).finally(() =>
      clearTimeout(timer),
    );
    if (done || !value.startsWith(prefix)) {
      throw new Error(`peer: expected "${prefix}", got ${done ? 'EOF' : value}\n${stderr}`);
    }
    return value.slice(prefix.length);
  }
  return { line, exited, seen, kill: () => peer.kill(), stderr: () => stderr };
}

// `port=… hash=…` from the peer's LISTENING line, as `{ port, hash }`.
export function listening(line) {
  return Object.fromEntries(line.split(' ').map((kv) => kv.split('=')));
}

// A page on ORIGIN that serves the engine (`/beam_web.js`, `/beam_web_bg.wasm` from `pkgDir`)
// and the scripts in this folder (`/worker.mjs`), with the engine loaded as `globalThis.beam`.
export async function openPage(browser, pkgDir) {
  const context = await browser.newContext();
  await context.grantPermissions(['local-network-access']); // to reach 127.0.0.1
  // The context's route also answers the page's workers.
  await context.route(`${ORIGIN}/**`, async (route) => {
    const name = path.basename(new URL(route.request().url()).pathname);
    if (name === '') {
      return route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>beam</title>' });
    }
    const dir = name.startsWith('beam_web') ? pkgDir : here;
    const contentType = name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript';
    return route.fulfill({ contentType, body: await readFile(path.join(dir, name)) });
  });
  const page = await context.newPage();
  page.on('pageerror', (error) => console.error('page error:', error));
  await page.goto(`${ORIGIN}/`);
  await page.evaluate(async () => {
    const beam = await import('/beam_web.js');
    await beam.default();
    globalThis.beam = beam;
  });
  return page;
}

export function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok - ${message}`);
}
