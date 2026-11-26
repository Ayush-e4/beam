# Performance

Goal: transfers limited by the physical link, not by Beam. Every technique below is validated by
benchmarks (see `testing.md`); anything that doesn't measurably help is removed.

## Link ceilings (realistic)

| Link                  | Typical real throughput |
| --------------------- | ----------------------- |
| Wi-Fi 5 (ac)          | ~300–600 Mbps           |
| Wi-Fi 6/6E (ax)       | ~600–1,200 Mbps         |
| Wi-Fi 7 (close range) | ~1.5–2.5+ Gbps          |
| USB-C (USB 3.x)       | ~3–10 Gbps              |

## 1. Raise the ceiling

- **Direct Wi-Fi for phone ↔ phone:** through a router every packet crosses the air twice;
  a Wi-Fi Direct / Wi-Fi Aware link can nearly double throughput.
- **Band awareness:** prefer 5/6 GHz and wide channels; warn when stuck on 2.4 GHz.
- **Cable mode:** USB network interface for multi-gigabit transfers.
- **Wi-Fi performance lock:** `WIFI_MODE_FULL_LOW_LATENCY` during transfers to stop
  power-save from throttling the radio.

## 2. Transport

- **QUIC** (`quinn`) with **Cubic** congestion control, large flow-control windows and large
  socket buffers (8 MiB asked for each way; systems may grant less).
- **UDP batching:** GSO/GRO and `sendmmsg`/`recvmmsg` to cut syscalls per packet.
- **Path MTU discovery** to use the largest packets the path allows.
- **TCP alternative:** kernel TCP can beat userspace QUIC on phone CPUs at gigabit speeds.
  Both are implemented and the faster one is chosen per link type from benchmark data.
- **Parallel streams** to keep the pipe full and hide per-stream stalls.

### Congestion control

Measured on a Galaxy S21 FE and a Mac on shared office Wi-Fi (RTT about 18 ms idle), 64 MiB
per run, runs alternated because the network's own speed swings by 2x within minutes
(2026-11-26):

| Phone sending to the browser | Speed         | RTT under load | Packets lost |
| ---------------------------- | ------------- | -------------- | ------------ |
| quinn BBR                    | 2.1–3.4 MiB/s | 164–375 ms     | 15–48%       |
| quinn Cubic                  | 2.0–4.0 MiB/s | 30–45 ms       | under 0.2%   |
| raw TCP, same minutes        | 3.0–5.4 MiB/s |                |              |

quinn's BBR ignores loss: it kept sending at two to three times what the link carried, the
queue in front of it grew to hundreds of milliseconds and up to half its packets were dropped
(native runs counted packets at both ends: real loss, not reordering). Cubic is quinn's
default and its most tested controller; it is as fast or faster, keeps latency low and doesn't
flood a network other people share. Android's default UDP receive buffer (about 320 KiB) also
overflowed under load, hundreds of drops per 64 MiB; 8 MiB buffers removed them.

Still open: the browser sending to the phone reaches about a third of raw TCP (1.2–2.1 MiB/s
against 4.7–5.7). Chromium's QUIC runs that sender, so the phone only sees its retransmissions,
not why it slows down.

## 3. CPU

- **Hardware crypto:** AES-GCM via ARMv8 crypto extensions (ChaCha20-Poly1305 fallback on
  devices without them). Noise uses `ring` for its ciphers (`snow` feature `ring-accelerated`);
  the default pure-Rust backend does not use the AES/PMULL instructions. Session throughput,
  seal and open in one process, 1 MiB frames (2026-11-25):

  | Device                      | Backend   | AES-256-GCM | ChaCha20-Poly1305 |
  | --------------------------- | --------- | ----------- | ----------------- |
  | Galaxy S21 FE (Exynos 2100) | pure Rust | ~95 MB/s    | ~240 MB/s         |
  | Galaxy S21 FE (Exynos 2100) | `ring`    | ~380 MB/s   | ~285 MB/s         |
  | Mac (Apple Silicon)         | pure Rust | ~140 MB/s   | ~420 MB/s         |
  | Mac (Apple Silicon)         | `ring`    | ~2,800 MB/s | ~1,380 MB/s       |

- **BLAKE3** hashing with NEON SIMD.
- **Pipelined, multi-core stages:** read → hash → (compress) → encrypt → send run concurrently
  with bounded queues for backpressure.
- **Big-core affinity** for hot threads during active transfers.
- **Thermal awareness:** watch thermal status and pace to avoid hard throttling.
- **Zero-copy where possible:** reuse buffers from a pool; avoid intermediate copies.

## 4. Storage

- Raw file descriptors with `pread`/`pwrite` and large aligned buffers.
- **Pre-allocate** destination files (`fallocate`) to avoid fragmentation.
- Avoid FUSE overhead on shared storage: write to app storage or use passthrough-friendly
  paths, then move into place.
- **Small-file packing** into one stream; parallel metadata reads.
- **Adaptive compression:** zstd only when sampled data is compressible.

## 5. Browser side

- **WebTransport** (HTTP/3) first: typically much faster than WebRTC data channels. The phone's
  self-signed certificate is pinned by the hash it sends during pairing.
- **WebRTC tuning** as fallback: multiple data channels, tuned message size, buffered-amount
  thresholds for flow control.
- **Stream to disk:** File System Access API on Chromium; streamed service-worker downloads on
  Safari and Firefox. Never hold large files in memory.
- Workers for hashing and decryption so the UI stays at 60+ fps.

## 6. Android system integration

- Foreground service with progress notification for every active transfer.
- Partial wake lock only while bytes are moving.
- Battery-aware behaviour: skip speed boosts when battery is low unless the user opts in.

## Targets (to validate by benchmark)

| Scenario                    | Typical apps | Beam target                    |
| --------------------------- | ------------ | ------------------------------ |
| Phone → phone via router    | ~20–40 MB/s  | ~50–80 MB/s                    |
| Phone → phone, direct Wi-Fi | n/a          | ~80–150 MB/s                   |
| Phone → browser             | ~10–30 MB/s  | ~50–100+ MB/s                  |
| USB-C cable                 | n/a          | ~200–500+ MB/s                 |
| 10,000 small photos         | minutes      | close to large-file throughput |

## Measurement

- Built-in diagnostics screen: link type, band, negotiated transport, live MB/s, CPU and
  thermal state.
- Benchmark harness in `tools/` compares Beam against LocalSend and Quick Share on the same
  network and devices; results are published.
