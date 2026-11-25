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
- A device is identified by its **device ID**: a hash of its public key.
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
4. Both sides display a short **verification code** derived from the handshake transcript
   (optional confirmation for high-trust pairing).
5. Each side stores the other as a trusted device.

## Discovery (trusted devices)

- **LAN:** mDNS service `_beam._udp` advertising device ID and ports (no personal names unless
  the user opts in).
- **Signaling:** trusted devices can meet in a room derived from their shared secret.
- **Direct Wi-Fi:** Wi-Fi Aware publish/subscribe with the same service name.

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
   every 8 MiB, every 250 ms while anything is unacknowledged, and as soon as all expected
   data has arrived.
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

- `CLIPBOARD` frames carry text directly, and images or files as a small transfer.
- Each item has an ID and timestamp; the newest wins, and echoes are suppressed.
- Pro feature; gated by capability negotiation.

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
