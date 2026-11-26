# Architecture

## Overview

```
┌──────────────────── Android app ────────────────────┐
│  UI (Kotlin + Jetpack Compose)                       │
│  Services: transfer foreground service, clipboard    │
│  Platform: discovery (NSD/mDNS), Wi-Fi Direct/Aware, │
│            USB network, Keystore, storage            │
│  ─────────────── JNI boundary ───────────────        │
│  beam-core (Rust): protocol, QUIC/TCP transport,     │
│            crypto, hashing, file I/O, packing        │
│  libwebrtc (C++): WebRTC peer for browsers           │
└──────────────────────────────────────────────────────┘
        ▲ QUIC/TCP (LAN, direct Wi-Fi, USB)      ▲ WebTransport / WebRTC
        ▼                                        ▼
  Another Android phone              Web app (React PWA) in any browser
                                                 ▲
                                                 │ signaling only (no file data)
                                                 ▼
                                Signaling service (Cloudflare Workers)
                                TURN relay (fallback only)
```

## Components

### Android app (`android/`)

- **UI:** Kotlin + Jetpack Compose, single-activity, unidirectional data flow (ViewModel → state).
- **Transfer service:** a foreground service (`dataSync` type) owns every active transfer so
  Android doesn't kill it. Note Android 15+ limits `dataSync` services to about 6 hours per 24 h.
- **Discovery:** Network Service Discovery (mDNS) on the LAN, plus QR pairing.
- **Links:** LAN, Wi-Fi Direct / Wi-Fi Aware, local-only hotspot, internet (WebRTC/QUIC),
  Bluetooth fallback (v1.1), USB network (later).
- **Security:** pairing keys in Android Keystore (StrongBox where available).
- **Storage:** Room for history and trusted devices; MediaStore / SAF for user files.
- **Permissions:** the Android 17 local network permission is requested with clear onboarding.

### beam-core (`core/`, Rust)

Shared engine, compiled for Android, the browser (WebAssembly) and later desktop.

- **Features:** the default `native` feature adds sockets, QUIC, the phone's WebTransport
  server and the tokio-driven transfer loops. Without it the crate builds for
  `wasm32-unknown-unknown`, keeping the protocol, crypto, framing and handshake, so the web
  app runs the same engine as the phone. In browsers, randomness comes from
  `crypto.getRandomValues` (ring's `wasm32_unknown_unknown_js`, getrandom's `wasm_js` with
  the cfg in `core/.cargo/config.toml`), and the C parts (ring, zstd) build with clang.

- Session and protocol state machine (see `protocol.md`).
- Transports: QUIC (`quinn`) and TCP, chosen per link by benchmark results.
- Crypto: hardware-accelerated AES-GCM / ChaCha20-Poly1305, Noise-based handshakes.
- Integrity: BLAKE3 per-chunk hashes, enabling resume.
- File pipeline: parallel read → hash → encrypt → send; small-file packing; adaptive zstd.
- Source files are read by position through the `ReadAt` trait: a `File` on the phone and the
  Mac, a picked file (`FileReaderSync`) in the browser's worker. Paths are only needed for
  folders picked on the Mac.
- Exposed to Kotlin through `beam-ffi`, a thin layer whose bindings UniFFI generates (Kotlin
  package `app.beam.core`, loaded through JNA). Every fallible call returns a typed
  `BeamException` with one subclass per error category (network, storage, permission,
  protocol, crypto, cancelled), so the app picks a message and an action without parsing
  strings. `core/uniffi-bindgen` pins the generator to the same UniFFI version.

### beam-web (`core/beam-web`, Rust → WebAssembly)

beam-core without `native`, wrapped with `wasm-bindgen` for the web app: `Identity` (the
browser's key pair, saved in IndexedDB by the app), `Invite` (the QR text and signaling room),
`BeamHandshake` (pairing or known device; the browser always initiates) and `BeamConnection`,
which runs the Noise handshake and encrypted session over a WebTransport bidirectional stream.
`js_stream.rs` adapts the stream's `readable`/`writable` into tokio's byte-stream traits, so the
engine code is unchanged. Errors are `Error`s named `BeamError` with a `category`.

### WebRTC peer (Android)

Google's libwebrtc for browsers that lack WebTransport. Data channels carry the same framed
protocol as the native transports, so the session logic stays in `beam-core`.

### Web app (`web/`, React PWA)

- Served over HTTPS from our domain so it is installable and gets full browser APIs.
- Shows the pairing QR code for the phone to scan, then connects to the phone directly:
  **WebTransport** first (certificate pinned by the hash the phone sends during pairing),
  **WebRTC** data channels as fallback.
- Streams received files straight to disk (File System Access API on Chromium; streamed
  service-worker downloads on Safari/Firefox).
- Clipboard via the Async Clipboard API (works while the tab is focused).
- Web Share Target so the installed PWA appears in the desktop share menu.
- Built with Vite + React as a static PWA (no server rendering needed).
- Animations with GSAP (official React hooks) and anime.js, shared with the website.

### Website (`site/`, Next.js)

Marketing site: landing page, features, live benchmarks, downloads. Next.js for SEO and
static generation, GSAP and anime.js for animation.

### Signaling (`signal/`, Cloudflare Workers + Durable Objects)

- Exchanges connection offers between a phone and a browser; never sees file data.
- Rooms are keyed by a one-time pairing token from the QR code and expire quickly.
- Messages are sealed with a key derived from the QR token, so the server cannot read, alter or
  impersonate them.

### TURN relay

Used only when a direct connection fails (strict office or school networks). Data stays
end-to-end encrypted. Free tier has a monthly cap; unlimited on Pro.

## Connect anywhere

Beam never requires devices to share a Wi-Fi network. It tries the fastest path first and falls
back automatically; the user never picks a mode, the UI only shows which link is in use.

```
Same network → Direct Wi-Fi → Phone hotspot → Internet peer-to-peer → Encrypted relay
                                                       (small items only) → Bluetooth
```

| Path                  | When                                       | How                                                                   |
| --------------------- | ------------------------------------------ | --------------------------------------------------------------------- |
| Same network          | Both devices on the same Wi-Fi/LAN         | Direct LAN connection (signaling via Workers when online)             |
| Offline local server  | Shared Wi-Fi but no internet               | Phone hosts a local web server; reduced browser features              |
| Direct Wi-Fi          | Phone ↔ phone, no shared network           | Wi-Fi Direct / Wi-Fi Aware link                                       |
| Phone hotspot         | Phone ↔ laptop/tablet, no shared network   | Android local-only hotspot (no internet sharing, no mobile data used) |
| Internet peer-to-peer | Devices on different networks, both online | WebRTC/QUIC with NAT traversal (STUN)                                 |
| Encrypted relay       | Peer-to-peer blocked (strict networks)     | TURN, end-to-end encrypted; free tier capped                          |
| Bluetooth (v1.1)      | No network at all, small items             | BLE for text, links and clipboard                                     |
| Cable (later)         | USB-C connected                            | USB network interface                                                 |

## Repository layout

```
android/   Android app (Gradle, Kotlin)
core/      Rust workspace: beam-core engine, beam-ffi bindings, fuzz targets
web/       React PWA (Vite)
site/      Next.js marketing website
packages/  Shared React UI: design system, animation helpers
signal/    Cloudflare Workers signaling service
docs/      Planning and design documents
tools/     Benchmarks, test harnesses, scripts
```

## Cross-cutting rules

- Files stay under 300 lines; modules have one responsibility.
- All errors are typed and surfaced; nothing fails silently.
- Every component has its own tests and lint configuration (see `testing.md`).
