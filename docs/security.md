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

- Android: device private key in Android Keystore (StrongBox where available), or wrapped by a
  Keystore key when the algorithm isn't supported in hardware.
- Web app: non-extractable WebCrypto keys stored in IndexedDB.
- Trusted-device records store public keys only.

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

- mDNS advertisements contain a random device ID, not the user's name, unless they opt in.
- Logs contain no file names or contents; crash reports are opt-in.
- Play Store data-safety labels must accurately describe this behaviour.

## Abuse

- Unknown senders always need approval; repeated rejections trigger temporary blocking.
- Broadcast features (later versions) require explicit opt-in on each receiver.

## Process

- Dependency auditing in CI (`cargo audit`, `npm audit`, Gradle dependency checks).
- Fuzzing for the frame parser, manifest parser and pack format.
- Security review before each major release; an external audit is planned before v2.
- Private disclosure channel for vulnerability reports.
