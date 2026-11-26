// Pairs headless Chromium, running the WebAssembly engine, with a native peer that plays the
// phone over WebTransport, then echoes frames through the encrypted session. Also checks that
// a browser holding the wrong invite is refused. Usage (tools/web-engine-test.sh runs it):
//   node handshake.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <webtransport_peer binary>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { chromium } from 'playwright-core';

import { check, listening, openPage, startPeer } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);
const CLIPBOARD_FRAME = 0x08;
const FRAMES = 4;
const FRAME_BYTES = 256 * 1024;

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
async function pairAndEcho(page, listeningLine, frames) {
  const { port, hash } = listening(listeningLine);
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

async function pairs(browser) {
  const page = await openPage(browser, pkgDir);
  const invite = await newInvite(page);
  check(invite.badCipher === 'BeamError/invalid', 'an unknown cipher is refused as invalid');
  const peer = startPeer(peerBin, [invite.text]);
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
  const page = await openPage(browser, pkgDir);
  const shown = await newInvite(page);
  await newInvite(page); // the browser now holds a different invite than the peer scanned
  const peer = startPeer(peerBin, [shown.text]);
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
