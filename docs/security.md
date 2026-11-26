# Security

## Promises to users

1. Files and clipboard data travel **device to device, end-to-end encrypted**.
2. Beam's servers (signaling, relay) **never see file contents** and cannot impersonate a device.
3. No account, no contact upload, no tracking of what you send.
4. Nothing is accepted without the receiver's consent, except from devices they chose to trust.

## Threat model

| Threat                                    | Mitigation                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------------- |
| Eavesdropper on the same Wi-Fi            | All sessions encrypted with keys from the Noise handshake                              |
| Man-in-the-middle during pairing          | Handshake bound to the displayed key and one-time token in the QR code                 |
| Malicious or compromised signaling server | Signaling messages sealed with a QR-token-derived key; server only relays opaque blobs |
| Relay (TURN) operator                     | Data is end-to-end encrypted before reaching the relay                                 |
| Malicious nearby device sending files     | Transfers from unknown devices require explicit acceptance; rate limiting              |
| Malformed packets exploiting parsers      | Rust core, strict length checks, fuzz-tested parsers                                   |
| Path traversal in file names              | Names sanitized; writes confined to the chosen destination                             |
| Stolen pairing QR                         | Token is single-use and short-lived                                                    |
| Lost or stolen phone                      | Trusted devices can be revoked; keys protected by Android Keystore                     |
| Replay of old messages                    | Nonces and session-bound keys; frames outside the session are rejected                 |

## Cryptography

- **Key agreement:** X25519. **Signatures:** Ed25519.
- **Handshake:** Noise (XX pattern for first pairing, IK for known devices), implemented with
  the `snow` library: `Noise_XX_25519_AESGCM_BLAKE2s` / `Noise_IK_25519_AESGCM_BLAKE2s`, or the
  `ChaChaPoly` variants. Pairing binds the one-time token and the displaying device's key from the QR
  code into the prologue; the scanning device rejects any other key as soon as it arrives.
- **QUIC TLS:** QUIC's mandatory TLS 1.3 uses a throwaway self-signed certificate per endpoint
  and clients accept any certificate. This is deliberate: identity and end-to-end encryption
  come from the Noise handshake inside the stream, never from the transport.
- **Bulk encryption:** AES-256-GCM with hardware acceleration; ChaCha20-Poly1305 fallback. The
  device showing the pairing QR code picks one and puts it in the code.
- **Integrity:** BLAKE3 per chunk plus whole-file root hash.
- Only well-maintained, audited libraries; no custom crypto primitives.

## Key storage

- Android: the device's X25519 private key is wrapped (AES-256-GCM) by a key that lives in
  Android Keystore, in StrongBox where the phone has one, otherwise in the TEE. Noise needs the
  raw X25519 key in the engine, so it is unwrapped into memory that is wiped on drop. The
  wrapped key lives in no-backup storage; the Keystore key cannot leave the phone, so a backup
  would be useless. The Keystore key needs no user authentication, because transfers run in
  the background while the phone is locked. If the wrapped key can't be read (the Keystore key
  is gone, the file is damaged), the app creates a new identity, keeps the old file for
  diagnosis and tells the user to pair trusted devices again.
- Web app: non-extractable WebCrypto keys stored in IndexedDB.
- Trusted-device records store the device's public key, name, cipher and discovery key. The
  discovery key is the only secret: it lets this phone recognise that device's beacons, and
  can't be used to connect or impersonate it. Records live in no-backup app storage.
- Transfer history (device names, file names and sizes, outcomes, times, where files were
  saved) never leaves the phone: no-backup app storage, the newest 500 transfers, clearable by
  the person. The engine's list of received transfer IDs lives next to it.

## Web app specifics

- Served only over HTTPS with a strict Content Security Policy and Subresource Integrity.
- WebTransport certificate pinned by its hash, which the phone sends through the signaling room
  sealed with the QR-token-derived key.
- No third-party scripts on the transfer pages (analytics only on the marketing site, and
  privacy-friendly).

## Offline mode caveat

The offline local web server runs over plain HTTP on the LAN. Beam still encrypts the payload at
the application layer using keys from the QR code, but browsers treat the page as insecure, so
some features (clipboard, installation) are unavailable. The UI makes this clear.

## Privacy

- mDNS advertisements contain a beacon that changes every 15 minutes and on every network
  change. Only trusted devices can link it to the device (see `protocol.md`, Discovery);
  it never contains the device ID or the user's name.
- Logs contain no file names or contents; crash reports are opt-in.
- Play Store data-safety labels must accurately describe this behaviour.

## Abuse

- Every offer says whether it comes from a trusted device; offers from unknown devices always
  need the person's approval.
- The one exception: an offer that continues a transfer the person already accepted, of the
  same files (the same manifest fingerprint) from the same device (the same authenticated
  key), unfinished on disk. Reusing a transfer ID for other files always asks again. Progress
  is kept per sending key, so another device can neither continue nor add to it. Unfinished
  transfers are deleted after 7 days without progress.
- Unknown offers are limited to 5 per address per minute; over the limit they are declined as
  busy without reaching the app.
- An address whose unknown offers are declined 3 times within 10 minutes is refused for 10
  minutes, before any handshake work. Limits are per IP address (keys are free to make) and
  never apply to trusted devices.
- Incoming connections are handled side by side: at most 4 per address and 32 in total may be
  in their handshake or offer at once, with 10 s for the handshake and 30 s for the offer, so
  a device that connects and stays silent can't stop anyone else from sending.
- Broadcast features (later versions) require explicit opt-in on each receiver.

## Process

- Dependency auditing in CI (`cargo audit`, `npm audit`, Gradle dependency checks).
- Fuzzing for the frame parser, manifest parser and pack format.
- No `unsafe` Rust (`unsafe_code = "deny"`), with two documented exceptions in `beam-ffi`,
  each in its own file with a SAFETY note:
  - `shared.rs` duplicates a file descriptor the Android app holds open (`dup`), because shared
    gallery and picker items can't be reopened by path. The engine never closes or takes over
    the app's own descriptor.
  - `network.rs` (Android only) calls the NDK's `android_setsocknetwork` so a connection to a
    nearby device goes over the Wi-Fi network instead of a VPN that would capture it and can't
    reach the device. It only sets a socket option on a socket the engine owns; an unknown
    network fails with an error. Only connections to nearby devices are affected; everything
    else stays on the VPN, and Beam traffic is end-to-end encrypted either way.
- Security review before each major release; an external audit is planned before v2.
- Private disclosure channel for vulnerability reports.
