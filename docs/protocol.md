# Protocol

Draft design of the Beam wire protocol. Details will be finalized during implementation and
validated with tests and fuzzing.

## Goals

- One session protocol over every transport (QUIC, TCP, WebTransport, WebRTC data channels).
- End-to-end encrypted and authenticated, independent of the transport's own security.
- Resumable at chunk granularity; integrity-checked end to end.
- Versioned and forward-compatible.

## Identities

- Each device has a long-term **device key pair** (X25519 for key agreement, Ed25519 for
  signatures) stored in the platform keystore. On Android the private key never leaves Keystore
  where hardware support allows; otherwise it is wrapped by a Keystore key.
- A device is identified by its **device ID**: the first 16 bytes of
  `BLAKE3-derive_key("beam 2026-11-25 device id v1", X25519 public key)`, written as 32
  lowercase hex characters. It is derived, never chosen, so no other key can claim it.
- Trusted devices are stored after first pairing; later connections skip the QR step.

## Pairing (first connection)

The device with a camera scans; the other displays. Usually a laptop or browser shows the QR
code and the phone scans it. Between two phones, either can show it.

```
Laptop / browser shows QR ──► phone scans          (phone ↔ computer)
Phone A shows QR          ──► phone B scans        (phone ↔ phone)

QR payload: { version, pairing_token, displayer_device_pubkey, cipher, signaling_room,
              connection hints (LAN addresses, ports; only if the displayer accepts connections) }
```

The QR code holds text: `BEAM1:` then unpadded base32 (QR alphanumeric mode, so the code stays
small) of:

```
version u8 = 1 | cipher u8 (0 AES-256-GCM, 1 ChaCha20-Poly1305) | pairing_token [16]
| displayer key [32] | hint count u8 (at most 4)
hint = family u8 (4 or 6) | address [4 or 16] | port u16 big-endian
```

The `signaling_room` is not sent: both sides derive it as the first 16 bytes of
`BLAKE3-derive_key("beam 2026-11-25 pairing room v1", pairing_token)`. With two hints (one
IPv6) the text is 130 characters. A web link wrapping the same payload, so that a phone's
own camera app opens Beam, can follow once the public name and domain are chosen.

1. The displayer generates a one-time `pairing_token` (short-lived, single use) and picks the
   session `cipher`: AES-256-GCM when its CPU accelerates AES, otherwise ChaCha20-Poly1305. The
   scanner uses the same cipher. Trusted devices remember the cipher chosen at pairing.
   (Provisional: revisit after measuring on real devices in M2.)
2. The two devices connect. Who connects is independent of who displayed the code:
   - **Displayer accepts connections** (a phone or the desktop app): the scanner connects
     using the hints, or the signaling room if they fail.
   - **Displayer is a browser** (browsers cannot accept connections): the phone joins the
     signaling room and posts its connection hints and WebTransport certificate hash, sealed
     with a key derived from the pairing token so the server cannot alter them. The browser
     then connects to the phone.
3. Both sides run a **Noise XX handshake** bound to the pairing token and the displayer's key
   from the QR. The scanner rejects any other key, which defeats man-in-the-middle attacks,
   including a malicious signaling server.
   Right after the handshake each side sends a `HELLO`
   (`major u16 | minor u16 | name_len u8 | name | discovery_key`; the name is UTF-8, at most
   64 bytes, no control characters; the discovery key is 32 bytes) so the other can list it by
   name and recognise its beacons (see Discovery). A different major version ends pairing.
   The scanner tries every connection hint at once and keeps the first that completes the
   handshake. A code pairs one device; the displayer ignores failed attempts (an old code,
   another device's key) and keeps waiting until it is closed.
4. Both sides display a short **verification code** derived from the handshake transcript
   (optional confirmation for high-trust pairing).
5. Each side stores the other as a trusted device.

**Sealed signaling messages.** What the two devices pass each other through the signaling
room is sealed with ChaCha20-Poly1305 under
`BLAKE3-derive_key("beam 2026-11-26 signaling seal v1", pairing_token)`, so the server can
neither read nor forge it:

```
sealed  = version u8 (1) | nonce [12] (random) | ciphertext and tag
aad     = "beam signal v1" | version u8 | from u8 (0 displayer, 1 scanner)
message = type u8 | body
  1 offer: hints (as in the invite) | WebTransport certificate SHA-256 [32]
```

`from` is bound in, so a message opens only as coming from the side that sealed it; the server
can't bounce a device's own offer back to it.

## Discovery (trusted devices)

- **LAN:** mDNS service `_beam._tcp`. The instance name is a **beacon**, never the device ID or
  the user's name, and the SRV record gives the port of the device's listener. TXT holds only
  `v` (protocol major version).
- **Beacon:** `nonce (8 bytes) | tag (8 bytes)` as 26 base32 characters, where
  `tag = BLAKE3-keyed(discovery_key, "beam beacon" | nonce)[..8]`. The device picks a new random
  nonce every 15 minutes and whenever its network changes, so the name it announces can't be
  used to follow it across networks or over time.
- **Discovery key:** 32 bytes,
  `BLAKE3-derive_key("beam 2026-11-25 discovery key v1", static private key | generation u32 LE)`.
  It is sent only inside the encrypted pairing `HELLO`, and each side keeps the other's key
  with the trusted device. A device recognises a peer by finding which stored key matches the
  beacon's tag; to everyone else the beacon is random. Removing a trusted device moves this
  device to the next generation, so the removed device can no longer recognise it; the
  remaining trusted devices learn the new key the next time the two connect.
- After recognising a beacon, a device still connects with the Noise handshake for the
  stored public key, so a forged or replayed beacon can waste a connection attempt but
  can't impersonate anyone.
- **Signaling:** trusted devices can meet in a room derived from their shared secret.
- **Direct Wi-Fi:** Wi-Fi Aware publish/subscribe with the same service name.

## Signaling rooms

The signaling service (`signal/`) lets two devices that can't reach each other directly yet
(usually a browser and a phone) swap connection details. It only relays them: what devices
send is sealed with a key the server doesn't have, so it can't read, change or forge it.

- **Joining:** a WebSocket to `GET /rooms/{room}`, where `room` is the 16-byte room name as 32
  lowercase hex characters (for pairing, the `signaling_room` derived from the pairing token).
  Anything else is refused with HTTP 400 (`bad_room`); a plain HTTP request gets 426.
- **Messages:** devices send binary frames only. The room passes each one, unchanged, to the
  other device, or keeps it (up to 8) for the next device to join. The room's own notices are
  JSON text frames: `{"type":"joined","peers":n}` on joining (`n` already there),
  `peer-joined`, `peer-left`, and `buffer-full` when a message couldn't be kept.
- **Limits:** two devices per room, 16 KiB per message, 64 messages per device. A room closes
  10 minutes after its first device joins.
- **Close codes:** 4001 room full, 4002 room expired, 4003 too many messages, 4004 message
  too big, 4005 text sent (messages must be binary).

## Session layer

After the handshake, all messages are framed and encrypted with session keys:

```
Frame = { type: u8, stream_id: u32, length: u32, payload: [u8] }
```

Encoded frames form a plaintext stream that is cut into encrypted records of at most 65 535
bytes (`length: u16 BE`, then ciphertext with a 16-byte tag). Nonces are implicit counters, so a
dropped, replayed, reordered or altered record fails authentication and ends the session; the
transfer then resumes from the journal on a new connection. Each direction rekeys every 2^20
records.

| Type                | Purpose                                                |
| ------------------- | ------------------------------------------------------ |
| `HELLO`             | Protocol version, capabilities, device info            |
| `OFFER`             | Transfer manifest proposed by the sender               |
| `ACCEPT` / `REJECT` | Receiver decision (with optional per-file selection)   |
| `CHUNK`             | File data                                              |
| `ACK`               | Chunk ranges received and verified                     |
| `RESUME`            | Receiver's verified ranges for an interrupted transfer |
| `CLIPBOARD`         | Clipboard item (text, image, file reference)           |
| `CANCEL`            | Abort a transfer                                       |
| `PING` / `PONG`     | Liveness and latency measurement                       |
| `ERROR`             | Typed error with code and message                      |

Unknown frame types are ignored if marked optional, otherwise the session fails with a
version error. Capabilities in `HELLO` negotiate features (compression, packing, resume).

## Transfers

### Manifest (`OFFER`)

```
{ transfer_id, items: [ { item_id, relative_path, size, mime, modified_at, chunk_size } ],
  total_size }
```

- Files are split into fixed-size chunks (default 4 MiB, tuned by benchmarks; 64 KiB–16 MiB).
- Paths are sanitized and the manifest is validated as a whole before anything is written
  (see `beam-core/src/safe_path.rs` and `manifest.rs`).
- The manifest carries **no file hashes**, so the sender can start streaming immediately
  instead of reading every file twice.
- Encoded as `version | transfer_id | total_size | count | items` (little-endian; each item
  has its ID, size, chunk size, optional modified time, UTF-8 path and MIME type). Large
  manifests span several `OFFER` frames of 1 MiB; a flag marks the last piece, and the
  receiver caps the total at 64 MiB before validating it as a whole. Item ID `u32::MAX` is
  reserved for the small-file pack stream.
- The sender builds the manifest from the picked files and folders (`sender_files.rs`): each
  pick lands at the top of the destination under its own name, folders keep their layout,
  names are sanitized like the receiver does and colliding names get ` (2)`, ` (3)`, …
  Symbolic links inside folders, special files and empty folders are skipped. The transfer ID
  is random, MIME types are guessed from the extension, and a file whose size or modified time
  changes after the offer fails the transfer instead of sending mixed data.

### Transfer messages

One transfer runs per connection at a time. All integers are little-endian; a range is
`item_id u32 | start u64 | end u64` (chunk indices, end exclusive).

| Frame    | Payload                                                                               |
| -------- | ------------------------------------------------------------------------------------- |
| `ACCEPT` | `transfer_id` · count · item IDs (count 0 = every item)                               |
| `REJECT` | `transfer_id` · reason (declined, no space, busy, failed)                             |
| `RESUME` | `transfer_id` · count · item IDs (as in `ACCEPT`) · count · ranges already verified   |
| `ACK`    | count · ranges newly verified                                                         |
| `CANCEL` | `transfer_id` · reason                                                                |
| `CHUNK`  | item ID · chunk index · encoding · decoded length · chunk hash · optional root · body |

Decoders reject truncated payloads, trailing bytes, unknown values and empty or reversed
ranges, and bound every count by the payload size.

### Integrity

- **Chunk hash:** BLAKE3 in key-derivation mode with context `beam 2026-11 chunk hash v1`
  over the chunk bytes.
- **Root hash:** BLAKE3 with context `beam 2026-11 root hash v1` over
  `size (u64 LE) || chunk_size (u32 LE) || chunk_hash_0 || chunk_hash_1 || ...`.
- Each `CHUNK` frame carries its chunk hash; the receiver verifies the length and hash before
  writing, so a corrupted or tampered chunk is rejected immediately.
- The last chunk of an item also carries the item's root hash. The receiver recomputes the root
  from the chunk hashes it verified and compares them, confirming the whole file in one step.
- Separate contexts mean a chunk hash can never be mistaken for a root hash.

### Data flow

- Chunks are sent over multiple parallel streams; each chunk carries `item_id`, chunk index
  and chunk hash.
- The receiver writes chunks to a pre-allocated file at the right offset after verifying them,
  and acknowledges verified ranges.
- A transfer completes when every chunk of every item is verified and every root hash matches;
  then the file is moved into its final location atomically.

### Receiving

- Files are received into `<destination>/.beam-partial/<transfer id>/` next to the resume
  journal, so publishing a finished file is a same-disk rename: a file appears under its real
  name only once it is complete and verified. No manifest path may use that folder.
- Every chunk is checked against the manifest (accepted item, right size, root only on the
  last chunk) and its hash before it is written. Small files must arrive in the pack stream.
- The receiver acknowledges in batches: it flushes written data, records it in the journal,
  then sends `ACK`. An acknowledged chunk is always on disk.
- Root hashes are saved as soon as they arrive. At the end, the receiver re-checks every
  large file's root hash, re-reading only chunks from earlier connections.
- Existing files are never overwritten: a clash gets a numbered name such as `photo (1).jpg`.
  Modified times are restored where the file system allows.

### Small files

Many small files are **packed** into a continuous stream (a tar-like container with per-entry
headers) so the cost per file is close to zero.

```
entry : item_id u32 LE | length u32 LE | file bytes | chunk hash (32)
```

- A file is packed when it is at most 256 KiB and fits in one chunk of its own layout, so its
  entry is exactly chunk 0 of the normal scheme (same chunk hash, root hash and journal record).
- The pack stream is cut into `CHUNK` frames on its own stream and compressed like other data.
- The receiver checks every entry against the manifest (known, packable, right size, received
  once) and verifies its hash before writing; files already in the journal are not expected.

### Compression

Optional zstd per chunk, used only when sampling shows the data is compressible. Already
compressed formats (JPEG, MP4, ZIP, etc.) are skipped.

- Per file: skipped by MIME type, otherwise the first 64 KiB is test-compressed and must shrink
  by at least 10%.
- Per chunk: zstd level 1; a chunk that would not shrink by 10% is sent raw. Each chunk carries
  its encoding (`0` raw, `1` zstd).
- Chunk hashes cover the original bytes. The receiver decompresses into at most the chunk's
  declared size (no decompression bombs), then verifies.

### Resume

- The receiver keeps a small journal of verified ranges per transfer.
- On reconnect, the sender sends `OFFER` with the same `transfer_id`; the receiver answers
  with `RESUME` listing verified ranges, and only missing chunks are sent.
- `RESUME` repeats the selection, because the sender may never have seen the `ACCEPT` (it was
  lost in a drop) or may have restarted since.
- Small files not yet verified go again in a fresh pack stream (piece index back at 0). Empty
  files are never sent: they have no chunks, and the receiver creates them when publishing.
- A large file's root hash covers every chunk hash. If the receiver still needs the last chunk,
  the sender re-reads and hashes the chunks it skips; if the receiver already has the last
  chunk, it also has the root, and only the missing chunks are read.

### Sending and flow control

- Order on each connection: the pack stream first, then large files in manifest order.
- The sender caps unacknowledged data at 64 MiB (`ACK` ranges free it; at least one chunk is
  always allowed), so a slow receiver applies back-pressure instead of filling memory.
- A file whose size or modified time changed since the offer stops the transfer with a clear
  error rather than sending data that would fail its hashes.

### Running a transfer

1. Sender: `OFFER` (one or more frames). Receiver: `ACCEPT`, `RESUME` (it has earlier progress)
   or `REJECT`.
2. Sender streams `CHUNK`s while reading `ACK`s. The receiver makes data durable and acks it
   by volume (1 MiB at the start of each connection, doubling to 8 MiB), every 250 ms while
   anything is unacknowledged, and as soon as all expected data has arrived. Only acked data
   survives a drop, so the small first steps let a link that keeps dying still make progress.
3. When every file is verified and published, the receiver sends an **empty `ACK`** ("done").
   The sender closes; the receiver closes once it sees that (or after 5 s).
4. A side that fails for any reason other than the network sends `CANCEL` (failed) first. A
   dropped connection just ends; both sides reconnect and the receiver answers the repeated
   `OFFER` with `RESUME`.
5. If the link drops after the receiver published but before the sender saw `done`, the sender
   offers again. The receiver, which remembers finished transfer IDs, answers `RESUME` listing
   every chunk, then `done`; nothing is sent twice and no duplicate files appear.

All transfer frames use stream ID 1 (one transfer per connection). File I/O runs off the async
runtime on both sides (`beam-core/src/transfer.rs`).

## Clipboard

One copied item per `CLIPBOARD` frame, on the control stream (ID 0):

```
CLIPBOARD : kind u8 (1 text, 2 image) | flags u8 (bit 0: sensitive)
            | mime_len u8 | mime | body (the rest: UTF-8 text or image bytes)
```

- Text is `text/plain`, 1 byte to 1 MiB. Images are PNG, JPEG, GIF, WebP, AVIF, HEIC/HEIF or
  BMP, at most 8 MiB; bigger images and files go as a transfer. SVG is refused.
- **Sensitive** items (passwords, one-time codes) are kept out of previews and history. A
  receiver also treats text that looks like a one-time code as sensitive.
- **Labels** (`clipboard_label.rs`) let the receiver show a lone `http(s)` link as a link and a
  code big, with a copy button: a code is 4–10 letters and digits with at least 4 digits,
  ignoring spaces, dashes and a short prefix such as `G-`. `find_code` also picks the code out
  of a short message that mentions one ("Your OTP is 482913").
- **Echoes:** writing a received item to the local clipboard looks like a new copy, so each
  side remembers the last item that crossed either way and doesn't send it again
  (`EchoGuard`).
- Pro feature; gated by capability negotiation (not enforced yet).

Decoders reject unknown kinds and flags, a type that doesn't match the kind, text that isn't
UTF-8 and content over the limits (`beam-core/src/clipboard.rs`).

## Errors

Every error has a stable code, a category (`network`, `storage`, `permission`, `protocol`,
`crypto`, `cancelled`) and a human-readable message. Errors are logged locally (no personal
data) and surfaced in the UI with a suggested action.

## Versioning

- `HELLO` carries the protocol major and minor version.
- Major mismatch: the session fails with an "update required" error.
- Minor mismatch: both sides use the common subset of capabilities.

## Interoperability

LocalSend protocol support (v1.1) will be a separate adapter that maps LocalSend's HTTP API to
Beam transfers; it does not change this protocol.
