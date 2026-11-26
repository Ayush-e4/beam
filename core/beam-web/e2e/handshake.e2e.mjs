// Pairs headless Chromium, running the WebAssembly engine, with a native peer that plays the
// phone: the browser opens the peer's sealed offer, connects over WebTransport, pairs, swaps
// names, then echoes frames through the encrypted session. Also checks that a browser holding
// the wrong invite can neither open the offer nor pair. Usage (tools/web-engine-test.sh runs it):
//   node handshake.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <webtransport_peer binary>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { chromium } from 'playwright-core';

import { check, offered, openPage, paired, startPeer } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);
const CLIPBOARD_FRAME = 0x08;
const FRAMES = 4;
const FRAME_BYTES = 256 * 1024;

// In the page: makes an identity and invite, and checks bad arguments are tagged `invalid`.
async function newInvite(page) {
  return page.evaluate(() => {
    const { beam } = globalThis;
    globalThis.identity ??= beam.Identity.generate();
    globalThis.shown = globalThis.invite;
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

// In the page: opens the peer's offer with the invite named `opener` ('invite', the current
// one, or 'shown', the one before it), connects, pairs with the current invite, swaps names,
// and echoes random frames.
async function pairAndEcho(page, sealed, frames, opener = 'invite') {
  return page.evaluate(
    async ({ sealed, opener, frames, frameBytes, frameType }) => {
      const { beam, identity, invite } = globalThis;
      const bytes = new Uint8Array(sealed.match(/../g).map((byte) => parseInt(byte, 16)));
      let offer;
      try {
        offer = globalThis[opener].openOffer(bytes);
      } catch (error) {
        return { unopened: `${error.name}/${error.category}` };
      }
      const transport = new WebTransport(`https://${offer.addresses[0]}/beam`, {
        serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
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
        const phone = await connection.introduce('Test browser', identity.discoveryKey);
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
        const { name, publicKey, deviceId } = phone;
        const peer = { name, publicKey: Array.from(publicKey), deviceId };
        return { code: connection.verificationCode, phone: peer, same, ms };
      } catch (error) {
        return { failed: `${error.name}/${error.category ?? '-'}: ${error.message}` };
      } finally {
        transport.close();
      }
    },
    { sealed, opener, frames, frameBytes: FRAME_BYTES, frameType: CLIPBOARD_FRAME },
  );
}

async function pairs(browser) {
  const page = await openPage(browser, pkgDir);
  const invite = await newInvite(page);
  check(invite.badCipher === 'BeamError/invalid', 'an unknown cipher is refused as invalid');
  const peer = startPeer(peerBin, [invite.text]);
  try {
    const result = await pairAndEcho(page, await offered(peer), FRAMES);
    const problem = result.failed ?? result.unopened;
    check(!problem, problem ? `pairing: ${problem}\n${peer.stderr()}` : 'paired');
    const { code, browserName } = await paired(peer);
    check(result.code === code, `both sides show code ${code}`);
    check(result.phone.name === 'Test phone', `the browser learns the phone's name`);
    check(result.phone.publicKey.length === 32, `and its public key (${result.phone.deviceId})`);
    check(browserName === 'Test browser', `the phone learns the browser's name`);
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
    const sealed = await offered(peer);
    const unopened = await pairAndEcho(page, sealed, 1);
    check(
      unopened.unopened === 'BeamError/crypto',
      `a browser with another invite can't open the offer (${unopened.unopened})`,
    );
    const result = await pairAndEcho(page, sealed, 1, 'shown');
    check(Boolean(result.failed), `nor pair, even knowing where to connect (${result.failed})`);
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
