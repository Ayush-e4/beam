// The web app's engine worker, cut down to what the end-to-end tests drive: it pairs with the
// peer over WebTransport, then receives (or declines) what the peer offers, or sends it the
// picked files the page passes on. The page sends
// `{ op, ...args }` and gets one reply per message; `{ ready: true }` comes first.
import init, * as beam from '/beam_web.js';

await init();
let identity;
let invite;

// Opens the phone's sealed offer (hex), connects where it says, pairs and swaps names.
async function connect(sealed) {
  const offer = invite.openOffer(new Uint8Array(sealed.match(/../g).map((b) => parseInt(b, 16))));
  const transport = new WebTransport(`https://${offer.addresses[0]}/beam`, {
    serverCertificateHashes: [{ algorithm: 'sha-256', value: offer.certificateHash }],
  });
  await transport.ready;
  const stream = await transport.createBidirectionalStream();
  const handshake = beam.BeamHandshake.pairing(identity, invite);
  const connection = await beam.BeamConnection.establish(handshake, stream.readable, stream.writable);
  const phone = await connection.introduce('Test browser', identity.discoveryKey);
  return { transport, connection, phone: phone.name };
}

async function sha256(file) {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await file.arrayBuffer()));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

const ops = {
  invite() {
    identity ??= beam.Identity.generate();
    invite = new beam.Invite(identity, 'chacha-poly');
    return { invite: invite.text() };
  },

  async receive({ sealed, decline }) {
    const { transport, connection, phone } = await connect(sealed);
    try {
      const offer = await connection.readOffer();
      const offered = offer.files.map(({ path, size }) => ({ path, size }));
      const code = connection.verificationCode;
      if (decline) {
        await connection.decline(offer);
        return { code, phone, offered, declined: true };
      }
      const progress = [];
      const started = performance.now();
      const root = await navigator.storage.getDirectory();
      const received = await connection.receive(offer, root, (done, total) =>
        progress.push({ done, total }),
      );
      const ms = performance.now() - started;
      const files = [];
      for (const { path, file } of await received.files()) {
        files.push({ path, name: file.name, size: file.size, sha256: await sha256(file) });
      }
      await received.remove();
      return { code, phone, offered, files, ms, progress };
    } finally {
      transport.close();
    }
  },

  async send({ sealed, files }) {
    const { transport, connection, phone } = await connect(sealed);
    try {
      const progress = [];
      const started = performance.now();
      const outcome = await connection.sendFiles(files, (done, total) =>
        progress.push({ done, total }),
      );
      const ms = performance.now() - started;
      return { code: connection.verificationCode, phone, outcome, ms, progress };
    } finally {
      transport.close();
    }
  },
};

self.onmessage = async ({ data }) => {
  try {
    self.postMessage(await ops[data.op](data));
  } catch (error) {
    self.postMessage({ failed: `${error.name}/${error.category ?? '-'}: ${error.message}` });
  }
};
self.postMessage({ ready: true });
