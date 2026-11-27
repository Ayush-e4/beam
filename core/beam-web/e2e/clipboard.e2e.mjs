// Clipboard items between headless Chromium and the phone app's engine over the link they
// keep open after pairing: `browser_link_peer` drives the same `Engine` and `ClipLink` calls
// the Android app makes. Items cross both ways, echoes are skipped on each side, a code in a
// message is found, an image arrives whole, and stopping ends the link cleanly for the phone.
// Usage (tools/web-engine-test.sh runs it):
//   node clipboard.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <browser_link_peer>
import { chromium } from 'playwright-core';

import { check, offered, openPage, startPeer } from './harness.mjs';

const [pkgDir, peerBin] = process.argv.slice(2);

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const page = await openPage(browser, pkgDir);
  const text = await page.evaluate(() => {
    const { beam } = globalThis;
    globalThis.identity = beam.Identity.generate();
    globalThis.invite = new beam.Invite(globalThis.identity, 'chacha-poly');
    return globalThis.invite.text();
  });
  const peer = startPeer(peerBin, [text]);
  try {
    const sealed = await offered(peer);
    const result = await page.evaluate(async (sealed) => {
      const { beam } = globalThis;
      const bytes = new Uint8Array(sealed.match(/../g).map((byte) => parseInt(byte, 16)));
      const offer = globalThis.invite.openOffer(bytes);
      const transport = new WebTransport(`https://${offer.addresses[0]}/beam`, {
        serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
      });
      const summary = (clip) => clip && { kind: clip.kind, text: clip.text, code: clip.code, sensitive: clip.sensitive, mime: clip.mime, size: clip.bytes?.length };
      try {
        await transport.ready;
        const stream = await transport.createBidirectionalStream();
        const handshake = beam.BeamHandshake.pairing(globalThis.identity, globalThis.invite);
        const connection = await beam.BeamConnection.establish(handshake, stream.readable, stream.writable);
        await connection.introduce('Test browser', globalThis.identity.discoveryKey);
        const link = connection.link();
        const first = summary(await link.nextClip());
        const echoSent = await link.sendText('from the phone', false);
        const sent = await link.sendText('from the browser', false);
        const otp = summary(await link.nextClip());
        const image = summary(await link.nextClip());
        link.stop();
        const kinds = ['https://beam.app', 'G-123456', 'hello'].map((text) => beam.clipKind(text));
        return { first, echoSent, sent, otp, image, kinds, found: beam.findCode('Use code 1234-5678') };
      } catch (error) {
        return { failed: `${error.name}/${error.category ?? '-'}: ${error.message}` };
      }
    }, sealed);
    check(!result.failed, result.failed ? `link: ${result.failed}\n${peer.stderr()}` : 'linked');
    check((await peer.line('LINKED')) === '', 'the link outlives the pairing object');
    check(result.first?.text === 'from the phone' && result.first.kind === 'text', 'the phone\'s copy arrives');
    check(result.echoSent === false && result.sent === true, 'the browser skips sending back what arrived');
    check((await peer.line('CLIP ')) === 'kind=Text sensitive=false text=from the browser', 'the browser\'s copy arrives');
    check((await peer.line('ECHO ')) === 'skipped', 'the phone skips sending back what arrived');
    check(result.otp?.code === '482913' && result.otp.kind === 'text', 'the code in a message is found');
    check(result.image?.kind === 'image' && result.image.mime === 'image/png' && result.image.size === 100 * 1024, 'the image arrives whole');
    check(result.kinds.join() === 'link,code,text' && result.found === '12345678', 'text checks work in the page');
    check((await peer.line('ENDED ')) === 'clean', 'stopping ends the link cleanly for the phone');
    check((await peer.exited) === 0, 'the phone engine exited cleanly');
  } finally {
    peer.kill();
  }
} finally {
  await browser.close();
}
