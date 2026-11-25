# Beam

Fast, private connectivity between your phone and every other device.

## Docs

| Doc                                  | Contents                                                  |
| ------------------------------------ | --------------------------------------------------------- |
| [Product](docs/product.md)           | Vision, users, v1 scope, roadmap, monetization, decisions |
| [Architecture](docs/architecture.md) | Components, connection modes, repository layout           |
| [Protocol](docs/protocol.md)         | Pairing, discovery, session framing, transfers, resume    |
| [Performance](docs/performance.md)   | Speed engineering and benchmark targets                   |
| [Security](docs/security.md)         | Threat model, cryptography, privacy                       |
| [Testing](docs/testing.md)           | Test strategy, end-to-end scenarios, CI                   |
| [Milestones](docs/milestones.md)     | Build order with exit criteria                            |

## Building the Android app

The app embeds the Rust engine, so a Gradle build also compiles `core/beam-ffi` for each ABI
and generates its Kotlin bindings. You need:

- JDK 21 (Android Studio's bundled one works) and the Android SDK.
- Android NDK `30.0.16248370`: `sdkmanager --install "ndk;30.0.16248370"`.
- Rust via rustup; `rust-toolchain.toml` pins the version and the Android targets.
- cargo-ndk: `cargo install cargo-ndk --version 4.1.2 --locked`.

Then `cd android && ./gradlew assembleDebug`. To build only one ABI while iterating, add
`-Pbeam.rustAbis=arm64-v8a`.

Working rules for this repository are in [CLAUDE.md](CLAUDE.md).
