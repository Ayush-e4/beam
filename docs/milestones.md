# Milestones

Build order for v1. There are no fixed dates; each milestone ends when its exit criteria pass.
Every milestone is delivered as a series of small PRs.

## M0: Foundations

- Monorepo layout (`android/`, `core/`, `web/`, `site/`, `signal/`, `packages/`, `tools/`).
- Toolchains pinned to latest stable/LTS versions (verified at setup time).
- Formatters, linters and CI for each component.
- **Exit:** empty projects build, lint and test green in CI.

## M1: Core engine (Rust)

- Framing, manifest, chunking, BLAKE3 verification, resume journal.
- Noise handshake and session encryption.
- QUIC and TCP transports; loopback transfers between two in-process peers.
- Small-file packing and adaptive compression.
- **Exit:** loopback transfers of large and many-small-file sets pass with loss and disconnects
  injected; fuzzers run clean.

## M2: Android ↔ Android

- JNI bridge with typed errors.
- Compose UI: home, device list, send, receive, history.
- Foreground transfer service, mDNS discovery, QR pairing, trusted devices.
- **Exit:** reliable phone-to-phone transfers on the LAN on at least three device brands.

## M3: Web app and signaling

- Cloudflare Workers signaling service.
- React PWA: pairing via QR, send and receive, streaming to disk.
- WebTransport path with certificate pinning; WebRTC fallback via libwebrtc on Android.
- **Exit:** phone ↔ browser transfers work on Chrome, Edge, Firefox and Safari.

## M4: Speed

- Direct Wi-Fi mode (Wi-Fi Direct / Aware).
- USB-C cable mode.
- Transport tuning (BBR, batching, buffers), QUIC vs TCP selection, multi-core pipeline.
- Benchmark harness and comparison with LocalSend and Quick Share.
- **Exit:** benchmark targets in `performance.md` met or explained.

## M5: Continuity and offline

- Universal clipboard (Pro).
- Offline mode with the local web server.
- Share Target in the PWA; Android share sheet integration.
- **Exit:** clipboard sync reliable both ways; offline transfers work without internet.

## M6: Product polish

- Design system in `packages/` shared by web app and site; GSAP and anime.js animations.
- Onboarding (including the Android 17 local network permission), empty states, error states.
- Billing (Google Play Billing) for Pro, with server-side purchase verification.
- TURN relay with free-tier cap.
- **Exit:** full end-to-end suite green; internal dogfooding without blocking issues.

## M7: Launch

- Next.js marketing site with benchmarks and downloads.
- Public name, store listing, screenshots, privacy policy and data-safety labels.
- Closed testing track on Google Play (required tester period for new apps), then production.
- **Exit:** live on Google Play; website live.

## After v1

- v1.1: LocalSend compatibility.
- v2: native desktop apps, folder sync, photo backup, notification mirroring.
- v3: webcam and mic, screen mirroring, broadcast, internet relay, automations, CLI.
