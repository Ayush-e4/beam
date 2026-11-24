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

- **QUIC** (`quinn`) with **BBR** congestion control, large flow-control windows and large
  socket buffers.
- **UDP batching:** GSO/GRO and `sendmmsg`/`recvmmsg` to cut syscalls per packet.
- **Path MTU discovery** to use the largest packets the path allows.
- **TCP alternative:** kernel TCP can beat userspace QUIC on phone CPUs at gigabit speeds.
  Both are implemented and the faster one is chosen per link type from benchmark data.
- **Parallel streams** to keep the pipe full and hide per-stream stalls.

## 3. CPU

- **Hardware crypto:** AES-GCM via ARMv8 crypto extensions (ChaCha20-Poly1305 fallback on
  devices without them).
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
