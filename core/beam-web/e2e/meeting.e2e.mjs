// Meeting again without a pairing code: headless Chromium and the phone app's engine
// (`browser_meeting_peer`, as if the two paired earlier) find each other in their meeting
// room on the signaling service (`signal/` under `wrangler dev`). The phone posts its offer
// when the browser joins; a stranger who knows the phone's key is dropped; the browser links
// with Noise IK and clipboard items cross both ways.
// Usage (tools/web-engine-test.sh runs it):
//   node meeting.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <browser_meeting_peer> <ws://signaling>
import { chromium } from 'playwright-core';

import { check, openPage, startPeer } from './harness.mjs';

const [pkgDir, peerBin, signaling] = process.argv.slice(2);

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const page = await openPage(browser, pkgDir);
  const browserKey = await page.evaluate(() => {
    const { beam } = globalThis;
    globalThis.identity = beam.Identity.generate();
    return Array.from(globalThis.identity.publicKey, (b) => b.toString(16).padStart(2, '0')).join('');
  });
  const peer = startPeer(peerBin, [browserKey, signaling]);
  try {
    const phoneKey = await peer.line('PHONE key=');
    const result = await page.evaluate(
      async ({ phoneKey, signaling }) => {
        const { beam } = globalThis;
        const hexToBytes = (hex) => new Uint8Array(hex.match(/../g).map((b) => parseInt(b, 16)));
        const phone = hexToBytes(phoneKey);
        const meeting = globalThis.identity.meet(phone);
        const room = Array.from(meeting.room, (b) => b.toString(16).padStart(2, '0')).join('');

        // As the web app will: wait in the room, and take the phone's offer.
        const socket = new WebSocket(`${signaling}/rooms/${room}`);
        socket.binaryType = 'arraybuffer';
        const offer = await new Promise((resolve, reject) => {
          socket.addEventListener('message', (event) => {
            if (typeof event.data !== 'string') resolve(meeting.openOffer(new Uint8Array(event.data)));
          });
          socket.addEventListener('close', (event) => reject(new Error(`the room closed (${event.code})`)));
        });
        socket.close();

        const connect = async (identity) => {
          const transport = new WebTransport(`https://${offer.addresses[0]}/beam`, {
            serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
          });
          await transport.ready;
          const stream = await transport.createBidirectionalStream();
          const handshake = beam.BeamHandshake.knownDevice(identity, 'chacha-poly', phone);
          const connection = await beam.BeamConnection.establish(handshake, stream.readable, stream.writable);
          return { transport, link: connection.link() };
        };

        // Someone who knows the phone's key (and saw the offer) but isn't the trusted browser.
        let stranger;
        try {
          const { transport, link } = await connect(beam.Identity.generate());
          const clip = await Promise.race([link.nextClip(), new Promise((r) => setTimeout(() => r('silence'), 3000))]);
          stranger = clip === null || clip === 'silence' ? 'dropped' : 'got a clip';
          transport.close();
        } catch {
          stranger = 'dropped';
        }

        const { transport, link } = await connect(globalThis.identity);
        try {
          const first = await link.nextClip();
          await link.sendText('from the browser again', false);
          link.stop();
          await new Promise((resolve) => setTimeout(resolve, 500));
          return { stranger, first: first?.text };
        } finally {
          transport.close();
        }
      },
      { phoneKey, signaling },
    );
    check(result.stranger === 'dropped', `a stranger with the phone's key is dropped (${result.stranger})`);
    check((await peer.line('LINKED')) === '', 'the phone links with its trusted browser');
    check(result.first === 'from the phone again', 'the phone\'s clipboard reaches the browser');
    check((await peer.line('CLIP text=')) === 'from the browser again', 'the browser\'s clipboard reaches the phone');
    check((await peer.line('ENDED ')) === 'clean', 'stopping ends the link for the phone');
    check((await peer.exited) === 0, 'the phone exits cleanly');
  } finally {
    peer.kill();
  }
} finally {
  await browser.close();
}
