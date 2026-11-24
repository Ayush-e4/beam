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
- **Links:** LAN, Wi-Fi Direct / Wi-Fi Aware (direct mode), USB network (cable mode).
- **Security:** pairing keys in Android Keystore (StrongBox where available).
- **Storage:** Room for history and trusted devices; MediaStore / SAF for user files.
- **Permissions:** the Android 17 local network permission is requested with clear onboarding.

### beam-core (`core/`, Rust)

Shared engine, compiled for Android now and desktop later.

- Session and protocol state machine (see `protocol.md`).
- Transports: QUIC (`quinn`) and TCP, chosen per link by benchmark results.
- Crypto: hardware-accelerated AES-GCM / ChaCha20-Poly1305, Noise-based handshakes.
- Integrity: BLAKE3 per-chunk hashes, enabling resume.
- File pipeline: parallel read → hash → encrypt → send; small-file packing; adaptive zstd.
- Exposed to Kotlin through a thin, versioned JNI layer with explicit error types.

### WebRTC peer (Android)

Google's libwebrtc for browsers that lack WebTransport. Data channels carry the same framed
protocol as the native transports, so the session logic stays in `beam-core`.

### Web app (`web/`, React PWA)

- Served over HTTPS from our domain so it is installable and gets full browser APIs.
- Connects to the phone directly: **WebTransport** first (certificate pinned by hash from the
  QR code), **WebRTC** data channels as fallback.
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
- Messages are authenticated with keys from the QR code, so the server cannot impersonate a peer.

### TURN relay

Used only when a direct connection fails (strict office or school networks). Data stays
end-to-end encrypted. Free tier has a monthly cap; unlimited on Pro.

## Connection modes

| Mode         | When                                    | Path                                             |
| ------------ | --------------------------------------- | ------------------------------------------------ |
| Connected    | Phone and browser both online           | Signaling via Workers, data direct over LAN      |
| Offline      | No internet, shared Wi-Fi               | Phone hosts a local web server; reduced features |
| Direct Wi-Fi | Phone ↔ phone, no router or faster path | Wi-Fi Direct / Aware link                        |
| Cable        | USB-C connected                         | USB network interface                            |
| Relay        | Direct path blocked                     | TURN, end-to-end encrypted                       |

## Repository layout

```
android/   Android app (Gradle, Kotlin)
core/      beam-core Rust workspace
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
