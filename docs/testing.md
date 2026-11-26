# Testing

Reliability is the product. Every layer is tested on its own and end to end, and nothing merges
to `master` unless all checks pass.

## Layers

### beam-core (Rust)

- **Unit tests** for every module (framing, manifest, chunking, hashing, packing, resume journal).
- **Property-based tests** (`proptest`) for round-trips: pack → unpack, frame → parse.
- **Fuzzing** (`cargo-fuzz`, in `core/fuzz`): frame decoding, path sanitising, encrypted
  session records, handshake messages, offered manifests, transfer messages (accept, reject,
  resume, ack, chunk headers), the pack format, zstd chunk decoding, the resume journal and scanned pairing codes.
  Run locally with `cargo +nightly fuzz run <target>`.
- **Loopback integration tests:** two in-process peers over QUIC and TCP, with injected packet
  loss, latency and disconnects.
  - `tests/transfer_loopback.rs`: whole transfers, rejection, cancellation, a lost `ACCEPT`, a
    lost "done".
  - `tests/m1_exit_tcp.rs` / `tests/m1_exit_quic.rs` (M1 exit): a large file, 10,000 small
    files and a selection survive repeated cuts (TCP: cuts on writes or reads, often
    mid-record, plus stalls; QUIC: 1–5 % loss, reordering and a link that dies). Each must end
    byte-identical with no duplicates, resuming from the journal every time.
  - The 1 GiB scenarios are `#[ignore]`d in debug runs and run in release in the `m1-exit` CI
    job: `cargo test --release --test m1_exit_tcp --test m1_exit_quic -- --ignored`.
- `clippy` with warnings as errors; `rustfmt` check.

### Android app

- **Unit tests** for ViewModels, repositories and use cases.
- **Engine bridge tests** (`android/core`): JVM tests call the Rust engine through the
  generated UniFFI bindings (built for the host and loaded with JNA), covering results and
  typed `BeamException`s, plus the identity store (first launch, reload, lost wrapping key,
  damaged file, no plaintext key on disk), the trusted-device list (restart, re-pairing,
  lookup by key, removal, damaged files fail loudly), and loopback transfers between two engines,
  including cancelling a receive coroutine and resuming.
- **Engine API tests** (`core/beam-ffi/tests/transfers.rs`): the exported transfer API driven
  as the app drives it: progress on both sides, declining, a device with another key, a
  cancelled receive that resumes, and a repeated offer for a saved transfer.
  `core/beam-ffi/tests/pairing.rs` pairs two engines (names, keys, matching verification
  codes, single-use codes), checks that forged codes fail while the host keeps waiting and
  that an unreachable address doesn't stop pairing, then sends a file using only what
  pairing returned.
- **Instrumented tests** for code that needs the real platform, run on a connected phone with
  `./gradlew :core:connectedDebugAndroidTest`: the Android Keystore key wrapper and the
  identity store on top of it, a 64 MiB loopback transfer between two engines, and the
  phone's own hotspot (it opens, gives its name, password and Wi-Fi QR text, and a transfer
  to the phone's address on it arrives).
- **Compose UI tests** for each screen and key flows (pairing, send, receive, history).
- **Instrumented tests** on emulators for services, permissions and storage.
- **Real-device test matrix** (manual and scripted over adb): Samsung, Pixel, Xiaomi, OnePlus,
  plus at least one low-end device. Covers Wi-Fi Direct/Aware, phone hotspot, internet
  peer-to-peer, background limits and OEM battery killers.
- `ktlint`/`detekt` and Android Lint with zero warnings.

### Web app (React PWA)

- **Unit tests** (Vitest + React Testing Library) for components and hooks.
- **Browser tests** (Playwright) on Chromium, Firefox and WebKit: pairing, send, receive,
  large-file streaming to disk, clipboard.
- ESLint + Prettier, TypeScript strict mode.

### Website (Next.js)

- Playwright smoke tests for pages, plus Lighthouse checks for performance and accessibility.

### Signaling service

- Unit tests for room lifecycle, expiry and message authentication.
- Local integration tests with Cloudflare's local runtime.

## End-to-end scenarios

Automated, using an Android emulator (or a real device over adb) and headless browsers:

| Scenario                                  | Checks                                        |
| ----------------------------------------- | --------------------------------------------- |
| Phone → browser, 1 file                   | Integrity, speed, UI states                   |
| Browser → phone, folder                   | Structure preserved, names sanitized          |
| Phone → phone                             | Both transports, discovery, acceptance        |
| 50 GB single file                         | Streaming to disk, memory stays flat          |
| 10,000 small files                        | Packing, throughput, no per-file stalls       |
| Wi-Fi drop mid-transfer                   | Resume from verified ranges, no corruption    |
| App backgrounded / screen locked          | Transfer continues via foreground service     |
| Storage full                              | Clean failure with a clear message            |
| Unknown sender                            | Approval prompt, rejection works              |
| Clipboard sync                            | Text and image both directions, no echo loops |
| No shared Wi-Fi (hotspot/direct/internet) | Automatic path selection, transfer completes  |
| Offline mode                              | Local server path works without internet      |

## Current test devices

| Device                      | Role                                                           |
| --------------------------- | -------------------------------------------------------------- |
| Samsung Galaxy S21 FE (adb) | Android app; scripted installs and tests over USB              |
| Mac                         | Laptop side: browsers (Chrome, Safari, Firefox), desktop later |
| iPad (Wi-Fi only)           | Real Safari/iPadOS web app; hotspot and internet path testing  |

## Network conditions

Use traffic shaping (e.g. `tc netem` on a Linux test router, or a configurable test proxy) to
simulate latency, loss, jitter and bandwidth caps.

## Benchmarks

- Harness in `tools/bench/` measures throughput, time to first byte, CPU, memory, battery
  drain and thermal state.
- Compares Beam with LocalSend and Quick Share on the same devices and network.
- Results are stored per commit so regressions are caught; published results are reproducible.

## CI (GitHub Actions)

- On every PR: format and lint checks, unit tests for all components, Rust fuzz smoke run,
  web Playwright tests, Android unit and emulator UI tests.
- Nightly: longer fuzzing, full end-to-end suite and benchmarks.
- Dependency audits on every PR.

## Definition of done for a PR

1. Formatted, linted, zero warnings.
2. New behaviour covered by tests; bug fixes include a reproducing test.
3. All CI checks green.
4. No file over 300 lines.
