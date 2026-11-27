// The real clipboard test: headless Chromium on this Mac links with the Android app's code on
// a phone over Wi-Fi. The phone runs BrowserLinkOnDeviceTest with the browser's pairing code
// and posts its sealed offer in the code's room on the signaling service running on this Mac
// (the phone reaches it through `adb reverse`), where the page picks it up. The page connects
// to the phone's Wi-Fi address, sends text and an image, and gets back what the phone's focus
// window sends. Then, without a code, the phone waits for this trusted browser in their
// meeting room and the page meets it again. tools/phone-browser-test.sh runs it.
// Usage: node phone-clipboard.e2e.mjs <dir with beam_web.js + beam_web_bg.wasm> <adb> <ws://signaling>
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

import { chromium } from 'playwright-core';

import { check, openPage } from './harness.mjs';

const [pkgDir, adb, signaling] = process.argv.slice(2);
const RUNNER = 'app.beam.core.test/androidx.test.runner.AndroidJUnitRunner';
const TEST = 'app.beam.core.clipboard.BrowserLinkOnDeviceTest';

// Waits for the phone test's next log line starting with `prefix` and returns the rest.
function phoneLog() {
  const logcat = spawn(adb, ['logcat', '-v', 'raw', '-s', 'BeamE2E:I'], { stdio: ['ignore', 'pipe', 'inherit'] });
  const lines = createInterface({ input: logcat.stdout })[Symbol.asyncIterator]();
  async function line(prefix, ms = 60_000) {
    const deadline = Date.now() + ms;
    while (Date.now() < deadline) {
      const { value, done } = await lines.next();
      if (done) break;
      if (value.startsWith(prefix)) return value.slice(prefix.length);
    }
    throw new Error(`phone: no "${prefix}" line`);
  }
  return { line, stop: () => logcat.kill() };
}

await new Promise((resolve) => spawn(adb, ['logcat', '-c'], { stdio: 'inherit' }).on('exit', resolve));
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const log = phoneLog();
try {
  const page = await openPage(browser, pkgDir);
  const code = await page.evaluate((signaling) => {
    const { beam } = globalThis;
    globalThis.identity = beam.Identity.generate();
    globalThis.invite = new beam.Invite(globalThis.identity, 'chacha-poly');
    const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
    // As the web app will: wait in a room for the phone's first offer.
    globalThis.waitInRoom = (room) =>
      new Promise((resolve, reject) => {
        const socket = new WebSocket(`${signaling}/rooms/${hex(room)}`);
        socket.binaryType = 'arraybuffer';
        socket.addEventListener('message', (event) => {
          if (typeof event.data === 'string') return; // the room's own notices
          resolve(new Uint8Array(event.data));
          socket.close();
        });
        socket.addEventListener('close', (event) => reject(new Error(`the room closed (${event.code})`)));
      });
    // Tries each address the phone offered, and once more, before giving up.
    globalThis.connectTo = async (offer) => {
      let last;
      for (const address of [...offer.addresses, ...offer.addresses]) {
        const attempt = new WebTransport(`https://${address}/beam`, {
          serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
        });
        try {
          await attempt.ready;
          return { transport: attempt, address };
        } catch (error) {
          last = error;
        }
      }
      throw last;
    };
    globalThis.phoneOffer = globalThis.waitInRoom(globalThis.invite.room);
    globalThis.phoneOffer.catch(() => {}); // awaited later
    return globalThis.invite.text();
  }, signaling);
  const instrument = spawn(adb, ['shell', 'am', 'instrument', '-w', '-e', 'class', TEST, '-e', 'browserCode', `'${code}'`, '-e', 'signaling', signaling, RUNNER], { stdio: ['ignore', 'pipe', 'inherit'] });
  let report = '';
  instrument.stdout.on('data', (chunk) => (report += chunk));
  const finished = new Promise((resolve) => instrument.on('exit', resolve));

  // The phone test may end before it logs anything (a crash, a missing code): fail then.
  const ended = finished.then(() => {
    throw new Error(`the phone test ended early\n${report.trim()}`);
  });
  await Promise.race([log.line('ANNOUNCED'), ended]);
  ended.catch(() => {}); // later lines are awaited on their own
  const result = await page.evaluate(async () => {
    const { beam } = globalThis;
    const offer = globalThis.invite.openOffer(await globalThis.phoneOffer);
    let transport;
    try {
      let connectedTo;
      ({ transport, address: connectedTo } = await globalThis.connectTo(offer));
      const stream = await transport.createBidirectionalStream();
      const handshake = beam.BeamHandshake.pairing(globalThis.identity, globalThis.invite);
      const connection = await beam.BeamConnection.establish(handshake, stream.readable, stream.writable);
      const phone = await connection.introduce('Chrome on Mac', globalThis.identity.discoveryKey);
      globalThis.phoneKey = phone.publicKey;
      const link = connection.link();
      const started = performance.now();
      await link.sendText('from the Mac', false);
      await link.sendImage('image/png', new Uint8Array(300 * 1024).fill(7), false);
      const first = await link.nextClip();
      const second = await link.nextClip();
      const ms = Math.round(performance.now() - started);
      link.stop();
      await new Promise((resolve) => setTimeout(resolve, 500));
      return { address: connectedTo, phone: phone.name, first: first?.text, second: { text: second?.text, code: second?.code, sensitive: second?.sensitive }, ms };
    } catch (error) {
      return { failed: `${error.name}/${error.category ?? '-'}: ${error.message} (offered ${offer.addresses.join(', ')})` };
    } finally {
      transport?.close();
    }
  });
  check(!result.failed, result.failed ?? `linked with ${result.phone} at ${result.address}`);
  check((await log.line('LINKED ')) === 'Chrome on Mac', 'the phone knows the browser by name');
  check((await log.line('GOT ')).length > 0, 'the phone got the text and the image');
  check(result.first === 'from the phone', 'the phone\'s clipboard reaches the Mac');
  check(result.second.code === '482913' && result.second.sensitive === false, 'a code in a message arrives with the code found');
  console.log(`# round trip with focus windows: ${result.ms} ms`);
  check((await log.line('ENDED')) === '', 'stopping ends the link on the phone');

  // Later, without a code: the phone waits for its trusted browser in their meeting room.
  await log.line('MEETING');
  const again = await page.evaluate(async () => {
    const { beam } = globalThis;
    const meeting = globalThis.identity.meet(globalThis.phoneKey);
    let transport;
    try {
      const offer = meeting.openOffer(await globalThis.waitInRoom(meeting.room));
      ({ transport } = await globalThis.connectTo(offer));
      const stream = await transport.createBidirectionalStream();
      const handshake = beam.BeamHandshake.knownDevice(globalThis.identity, 'chacha-poly', globalThis.phoneKey);
      const connection = await beam.BeamConnection.establish(handshake, stream.readable, stream.writable);
      const link = connection.link();
      const started = performance.now();
      await link.sendText('again from the Mac', false);
      const back = await link.nextClip();
      const ms = Math.round(performance.now() - started);
      link.stop();
      await new Promise((resolve) => setTimeout(resolve, 500));
      return { back: back?.text, ms };
    } catch (error) {
      return { failed: `${error.name}/${error.category ?? '-'}: ${error.message}` };
    } finally {
      transport?.close();
    }
  });
  check(!again.failed, again.failed ?? 'met the phone again without a code');
  check((await log.line('MET ')) === 'Chrome on Mac', 'the phone links its trusted browser again');
  check(again.back === 'again from the phone', 'clipboard items cross again');
  console.log(`# meeting round trip with a focus window: ${again.ms} ms`);
  check((await log.line('DONE')) === '', 'the phone stops waiting once let go');
  await finished;
  check(/OK \(1 test\)/.test(report), `the phone test passed\n${report.trim()}`);
} finally {
  log.stop();
  await browser.close();
}
