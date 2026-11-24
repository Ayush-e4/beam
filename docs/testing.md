# Testing

Reliability is the product. Every layer is tested on its own and end to end, and nothing merges
to `master` unless all checks pass.

## Layers

### beam-core (Rust)
- **Unit tests** for every module (framing, manifest, chunking, hashing, packing, resume journal).
- **Property-based tests** (`proptest`) for round-trips: pack → unpack, frame → parse.
- **Fuzzing** (`cargo-fuzz`) for the frame parser, manifest parser and pack format.
- **Loopback integration tests:** two in-process peers over QUIC and TCP, with injected packet
  loss, latency and disconnects.
- `clippy` with warnings as errors; `rustfmt` check.

### Android app
- **Unit tests** for ViewModels, repositories and use cases.
- **Compose UI tests** for each screen and key flows (pairing, send, receive, history).
- **Instrumented tests** on emulators for services, permissions and storage.
- **Real-device test matrix** (manual and scripted over adb): Samsung, Pixel, Xiaomi, OnePlus,
  plus at least one low-end device. Covers Wi-Fi Direct/Aware, USB cable mode, background
  limits and OEM battery killers.
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

| Scenario | Checks |
|---|---|
| Phone → browser, 1 file | Integrity, speed, UI states |
| Browser → phone, folder | Structure preserved, names sanitized |
| Phone → phone | Both transports, discovery, acceptance |
| 50 GB single file | Streaming to disk, memory stays flat |
| 10,000 small files | Packing, throughput, no per-file stalls |
| Wi-Fi drop mid-transfer | Resume from verified ranges, no corruption |
| App backgrounded / screen locked | Transfer continues via foreground service |
| Storage full | Clean failure with a clear message |
| Unknown sender | Approval prompt, rejection works |
| Clipboard sync | Text and image both directions, no echo loops |
| Offline mode | Local server path works without internet |

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
