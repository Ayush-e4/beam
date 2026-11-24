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

```
Phone shows QR ──► Browser/other phone scans
QR payload: { version, pairing_token, phone_device_pubkey,
              connection hints (LAN addresses, ports),
              webtransport_cert_hash, signaling_room }
```

1. The phone generates a one-time `pairing_token` (short-lived, single use).
2. The scanner connects using the hints (or the signaling room if direct hints fail).
3. Both sides run a **Noise XX-style handshake** bound to the pairing token and the phone key
   from the QR. A mismatch aborts the connection, which defeats man-in-the-middle attacks,
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

### Small files

Many small files are **packed** into a continuous stream (a tar-like container with per-entry
headers) so the cost per file is close to zero.

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
