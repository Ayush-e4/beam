// Pairs headless Chromium with the phone app's engine: `browser_pairing_peer` drives the same
// `Engine.pairWithBrowser` calls the Android app makes. The browser opens the phone's sealed
// offer, connects and pairs. A browser that connects with another code first is ignored, the
// right one pairs, each side learns the other's name and key, and the code is then spent.
// Usage (tools/web-engine-test.sh runs it):
//   node phone-pairing.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <browser_pairing_peer>
import { chromium } from 'playwright-core';

import { check, offered, openPage, startPeer } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);

// In the page: opens `sealed` with the invite that was shown, then pairs with that invite, or
// with a new one if `impostor` (a browser that learned the address but holds another code).
async function pair(page, sealed, impostor) {
  return page.evaluate(
    async ({ sealed, impostor }) => {
      const { beam } = globalThis;
      const bytes = new Uint8Array(sealed.match(/../g).map((byte) => parseInt(byte, 16)));
      const offer = globalThis.invite.openOffer(bytes);
      const invite = impostor ? new beam.Invite(globalThis.identity, 'chacha-poly') : globalThis.invite;
      const transport = new WebTransport(`https://${offer.addresses[0]}/beam`, {
        serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
      });
      try {
        await transport.ready;
        const stream = await transport.createBidirectionalStream();
        const handshake = beam.BeamHandshake.pairing(globalThis.identity, invite);
        const connection = await beam.BeamConnection.establish(
          handshake,
          stream.readable,
          stream.writable,
        );
        const phone = await connection.introduce('Test browser', globalThis.identity.discoveryKey);
        const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
        return { code: connection.verificationCode, name: phone.name, key: hex(phone.publicKey) };
      } catch (error) {
        return { failed: `${error.name}/${error.category ?? '-'}: ${error.message}` };
      } finally {
        transport.close();
      }
    },
    { sealed, impostor },
  );
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const page = await openPage(browser, pkgDir);
  const { text, browserKey } = await page.evaluate(() => {
    const { beam } = globalThis;
    globalThis.identity = beam.Identity.generate();
    globalThis.invite = new beam.Invite(globalThis.identity, 'chacha-poly');
    const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
    return { text: globalThis.invite.text(), browserKey: hex(globalThis.identity.publicKey) };
  });
  const peer = startPeer(peerBin, [text]);
  try {
    const phoneKey = await peer.line('PHONE key=');
    const sealed = await offered(peer);

    const impostor = await pair(page, sealed, true);
    check(Boolean(impostor.failed), `a browser holding another code is refused (${impostor.failed})`);

    const result = await pair(page, sealed, false);
    check(!result.failed, result.failed ? `pairing: ${result.failed}\n${peer.stderr()}` : 'paired');
    const [code, rest] = (await peer.line('PAIRED code=')).split(' name=');
    const [browserName, pairedKey] = rest.split(' key=');
    check(result.code === code, `both sides show code ${code}`);
    check(result.name === 'Test phone' && result.key === phoneKey, 'the browser learns the phone');
    check(browserName === 'Test browser' && pairedKey === browserKey, 'the phone learns the browser');
    check((await peer.line('SPENT ')).length > 0, 'the code then pairs no other browser');
    check((await peer.exited) === 0, 'the phone engine exited cleanly');
  } finally {
    peer.kill();
  }
} finally {
  await browser.close();
}
