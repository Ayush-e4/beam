// Pairs headless Chromium, running the WebAssembly engine, with a native peer that plays the
// phone over WebTransport, then echoes frames through the encrypted session. Also checks that
// a browser holding the wrong invite is refused. Usage (tools/web-engine-test.sh runs it):
//   node handshake.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <webtransport_peer binary>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const [pkgDir, peerBin] = process.argv.slice(2);
// Never contacted: every request to it is answered by the route below.
const ORIGIN = 'http://localhost:9';
const CLIPBOARD_FRAME = 0x08;
const FRAMES = 4;
const FRAME_BYTES = 256 * 1024;
const STEP_MS = 20_000;

function startPeer(invite) {
  const peer = spawn(peerBin, [invite], { stdio: ['ignore', 'pipe', 'pipe'] });
  const reader = createInterface({ input: peer.stdout });
  const seen = [];
  reader.on('line', (line) => seen.push(line));
  const lines = reader[Symbol.asyncIterator]();
  let stderr = '';
  peer.stderr.on('data', (chunk) => (stderr += chunk));
  const exited = new Promise((resolve) => peer.on('exit', (code) => resolve(code)));
  async function line(prefix) {
    let timer;
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`peer: no "${prefix}" line`)), STEP_MS);
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

async function openPage(browser) {
  const context = await browser.newContext();
  await context.grantPermissions(['local-network-access']); // to reach 127.0.0.1
  const page = await context.newPage();
  await page.route(`${ORIGIN}/**`, async (route) => {
    const name = path.basename(new URL(route.request().url()).pathname);
    if (name === '') {
      return route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>beam</title>' });
    }
    const contentType = name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript';
    return route.fulfill({ contentType, body: await readFile(path.join(pkgDir, name)) });
  });
  page.on('pageerror', (error) => console.error('page error:', error));
  await page.goto(`${ORIGIN}/`);
  await page.evaluate(async () => {
    const beam = await import('/beam_web.js');
    await beam.default();
    globalThis.beam = beam;
  });
  return page;
}

// In the page: makes an identity and invite, and checks bad arguments are tagged `invalid`.
async function newInvite(page) {
  return page.evaluate(() => {
    const { beam } = globalThis;
    globalThis.identity = beam.Identity.generate();
    globalThis.invite = new beam.Invite(globalThis.identity, 'chacha-poly');
    let badCipher = null;
    try {
      new beam.Invite(globalThis.identity, 'rot13');
    } catch (error) {
      badCipher = `${error.name}/${error.category}`;
    }
    return { text: globalThis.invite.text(), badCipher };
  });
}

// In the page: opens WebTransport to the peer, pairs, and echoes random frames.
async function pairAndEcho(page, listening, frames) {
  const { port, hash } = Object.fromEntries(listening.split(' ').map((kv) => kv.split('=')));
  return page.evaluate(
    async ({ port, hash, frames, frameBytes, frameType }) => {
      const { beam, identity, invite } = globalThis;
      const value = new Uint8Array(hash.match(/../g).map((byte) => parseInt(byte, 16)));
      const transport = new WebTransport(`https://127.0.0.1:${port}/beam`, {
        serverCertificateHashes: [{ algorithm: 'sha-256', value }],
      });
      try {
        await transport.ready;
        const stream = await transport.createBidirectionalStream();
        const handshake = beam.BeamHandshake.pairing(identity, invite);
        const connection = await beam.BeamConnection.establish(
          handshake,
          stream.readable,
          stream.writable,
        );
        const started = performance.now();
        let same = true;
        for (let i = 0; i < frames; i++) {
          const sent = new Uint8Array(frameBytes);
          for (let at = 0; at < frameBytes; at += 65536) {
            crypto.getRandomValues(sent.subarray(at, at + 65536));
          }
          await connection.send(frameType, sent);
          const got = await connection.recv();
          same &&=
            got?.frameType === frameType &&
            got.payload.length === sent.length &&
            got.payload.every((byte, j) => byte === sent[j]);
        }
        const ms = Math.round(performance.now() - started);
        await connection.close();
        return { code: connection.verificationCode, same, ms };
      } catch (error) {
        return { failed: `${error.name}/${error.category ?? '-'}: ${error.message}` };
      } finally {
        transport.close();
      }
    },
    { port, hash, frames, frameBytes: FRAME_BYTES, frameType: CLIPBOARD_FRAME },
  );
}

function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok - ${message}`);
}

async function pairs(browser) {
  const page = await openPage(browser);
  const invite = await newInvite(page);
  check(invite.badCipher === 'BeamError/invalid', 'an unknown cipher is refused as invalid');
  const peer = startPeer(invite.text);
  try {
    const result = await pairAndEcho(page, await peer.line('LISTENING '), FRAMES);
    check(!result.failed, result.failed ? `pairing: ${result.failed}\n${peer.stderr()}` : 'paired');
    const peerCode = await peer.line('PAIRED code=');
    check(result.code === peerCode, `both sides show code ${peerCode}`);
    check(result.same, `${FRAMES} x ${FRAME_BYTES / 1024} KiB echoed intact in ${result.ms} ms`);
    check((await peer.line('ECHOED frames=')) === String(FRAMES), 'the peer saw every frame');
    check((await peer.exited) === 0, 'the peer exited cleanly');
  } finally {
    peer.kill();
  }
}

async function refusesWrongInvite(browser) {
  const page = await openPage(browser);
  const shown = await newInvite(page);
  await newInvite(page); // the browser now holds a different invite than the peer scanned
  const peer = startPeer(shown.text);
  try {
    const result = await pairAndEcho(page, await peer.line('LISTENING '), 1);
    check(Boolean(result.failed), `a browser with the wrong invite is refused (${result.failed})`);
    // The browser stops at the peer's first reply and leaves, so the peer can't complete the
    // handshake (it would only notice at QUIC's idle timeout, so don't wait for it to exit).
    await new Promise((resolve) => setTimeout(resolve, 500));
    check(!peer.seen.some((line) => line.startsWith('PAIRED')), 'the peer never reports a pairing');
  } finally {
    peer.kill();
  }
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  await pairs(browser);
  await refusesWrongInvite(browser);
} finally {
  await browser.close();
}
