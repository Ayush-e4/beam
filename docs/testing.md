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
- **Browser build** (`wasm` CI job): clippy for `wasm32-unknown-unknown` without the `native`
  feature, and the unit tests without it. Locally the C parts need a clang that targets wasm:
  Apple's clang doesn't, so point `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown`
  at the Android NDK's `clang` and `llvm-ar`.

### Browser engine (`core/beam-web`)

`tools/web-engine-test.sh` (CI job `web-engine`) builds the WebAssembly engine and a native peer
that plays the phone (`beam-core/examples/webtransport_peer.rs`), then runs
`core/beam-web/e2e/handshake.e2e.mjs` in headless Chromium: the browser shows an invite, the
peer "scans" it, serves WebTransport and prints the sealed offer it would post in the signaling
room. The browser opens the offer and connects where it says; both pair, must show the same
verification code and learn each other's name, and 1 MiB of random frames is echoed through
the encrypted session. A browser holding another invite can't open the offer (`crypto`), nor
pair even knowing the address, and the peer never reports a pairing. The receive and send
tests below pair the same way. Locally,
`CHROMIUM_PATH` can point at any Chromium; `wasm-bindgen-cli` must match `core/Cargo.lock`.

It then runs `e2e/receive.e2e.mjs`: the peer sends a folder (a 24 MiB incompressible file, a
compressible one, an empty one, a nested one) and a loose file to the engine in a dedicated
worker (`e2e/worker.mjs`, a stand-in for the web app's). Every file must come back from OPFS
with its name and SHA-256, progress must reach the total, and the phone must hear `Delivered`.
A declined offer must reach the phone as `UserDeclined`. It prints the speed; with a release
peer, loopback reaches about 70–100 MiB/s on one browser core.

Then `e2e/send.e2e.mjs` does the reverse. Files are picked in the page through a real
`<input type=file>` and passed to the worker. The peer must save them intact, progress must
reach the total, and the browser must hear `delivered`. A declined offer must come back as
`declined`. A file edited on disk after it was picked must fail as a storage error naming it.

`e2e/phone-pairing.e2e.mjs` pairs the browser with the phone app's engine instead:
`beam-ffi/examples/browser_pairing_peer.rs` makes the same `Engine.pairWithBrowser` calls as
the Android app. A browser that connects holding another code must be refused while the phone
keeps waiting. Then the right browser pairs, each side must learn the other's name and public
key, and the code must refuse a second pairing.

`e2e/meeting.e2e.mjs` meets that browser again without a code, through the real signaling
service (`signal/` under `wrangler dev`, which the script starts). The phone stand-in,
`beam-ffi/examples/browser_meeting_peer.rs`, makes the Android app's `Engine.meetBrowser`
calls for a browser it trusts. The browser derives the same room from the two keys, waits
there and opens the offer the phone posts when it joins. A stranger who knows the phone's key
and connects first must be dropped. The trusted browser then links with Noise IK, and clipboard
items must cross both ways and the link must end cleanly.

The same script then runs beam-web's browser-only Rust tests (`core/beam-web/tests`) in that
Chromium: `wasm-bindgen-test-runner` serves them and `e2e/wasm-tests.e2e.mjs` opens the page
and reports the result. `tests/opfs_staging.rs` runs in a dedicated worker, where OPFS sync
handles exist. A sender reads files from memory into a receiver staging in OPFS, and the tests
check that:

- a whole transfer (a multi-chunk file, a packed small file, an empty file) arrives intact;
- a transfer resumes after the worker stops mid-transfer;
- a second tab can't open a transfer that is in progress.

`tests/transfer_loops.rs` runs the real `send_transfer` and `receive_transfer` there, over an
encrypted connection on an in-memory pipe, into OPFS. One test sends with one piece in flight,
so only the receiver's ack timer (the browser's `setTimeout`) can keep it moving: it hangs if
the timers break.

`tests/pairing.rs` opens offers that beam-core sealed: the addresses (IPv4 and bracketed IPv6)
and certificate hash come out as sealed, and another pairing's offer, the browser's own message
bounced back, and an offer without an address are all refused.

### Mac command-line device (`core/beam-cli`)

A second Beam device for when there's no second phone: `beam` pairs, sends and receives with
the phone over the real Wi-Fi. Build it with `cargo build -p beam-cli`; its state (identity and
trusted devices) lives in `$BEAM_HOME`, or `~/.beam-cli`.

- `beam pair`: shows a code for the phone to enter, then prints `PAIRED name=… verify=…`.
- `beam receive [--port 47123] [--into inbox] [--accept-all]`: receives until stopped.
- `beam send WHO HOST PORT FILE...`: sends to a trusted device.
- `beam id`, `beam devices`, `beam pair-with CODE`.

Output is one line per event (`LISTENING`, `RECEIVED`, `SENT`, `DECLINED`, …) so scripts can
read it. `tests/two_devices.rs` pairs two CLI devices over loopback, sends a file and checks
that a device that never paired is declined.

`tools/mac-peer-test.sh` (`MIB=64` by default) runs the phone on USB against the CLI over a
real network: the phone pairs with `beam pair`'s code (both sides must show the same
verification code), sends the Mac random data, announces itself over mDNS (the Mac must see
its beacon with `dns-sd`), then receives data from the Mac, and the script
prints each direction's speed. By default both use the Wi-Fi they're on; with `--hotspot` the
phone opens its own hotspot and the Mac joins it (leaving its Wi-Fi for the test, then
rejoining). The phone side is `MacPeerOnDeviceTest`, skipped in ordinary test runs. To tell
Beam's speed from the network's, compare with raw TCP (`nc`) between the same two devices.

`tools/phone-browser-test.sh` is the real clipboard test between the phone on USB and headless
Chromium on the Mac, over the Wi-Fi they share. The phone (`BrowserLinkOnDeviceTest`, skipped in
ordinary runs) links with the browser's pairing code through `BrowserLinking`, as the app does,
and posts its sealed offer in the code's room on the signaling service, which the script runs on
the Mac under Wrangler (the phone reaches it through `adb reverse`); the page waits in that room
for it, as the web app will. The browser then sends text and a 300 KiB image, and gets back what the phone's focus window sends, including
a message with a one-time code. The phone's screen must be on and unlocked.

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
- `tools/signal-room-test.sh` runs the Worker under `wrangler dev` and has beam-core's
  `SignalRoom` join it as a phone and a browser: the kept offer, the answer, a third device
  turned away, and the other device leaving (`core/beam-core/tests/signal_room_live.rs`).

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
