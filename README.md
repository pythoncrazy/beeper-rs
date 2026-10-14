# beeper-rs

`beeper-rs` is an independent, unofficial open-source Rust desktop application, HTTP/1.1 REST server, RFC 6455 WebSocket server, and client SDK that implements the public Beeper Desktop API protocol.

## Isolated Data Directory Safety

`beeper-rs` never modifies or locks the original Beeper Desktop directory (`BeeperTexts` or `BeeperTexts-<profile>`):

- `beeper-rs` stores all database and media files in an isolated directory (`~/.config/BeeperRs` on Linux, `~/Library/Application Support/BeeperRs` on macOS, or `%APPDATA%\BeeperRs` on Windows).
- On first startup, `beeper-rs` copies `index.db` and `media/` from the original `BeeperTexts` directory (or a directory specified with `--import-from <DIR>`) into `BeeperRs` and skips active lock and cache files.
- `beeper-rs` reads only the Matrix session (`beeperState`) from the copied `index.db`. `beeper-rs` writes its own data to `beeperRs:*` keys and `beeper_rs_*` tables.
- If `beeper-rs` stops with the message "written by an older beeper-rs build", delete the `BeeperRs` directory and start `beeper-rs` again.

## Sign-In

- When no Beeper session is available, `beeper-rs` opens the Beeper sign-in window: "Continue with Email", the email screen, the six-digit code screen, "Resend", and "Pick a Username" for new accounts.
- `beeper-rs` sends the sign-in requests to the Beeper cloud API (`https://api.beeper.com`) and then signs in to the Matrix homeserver (`https://matrix.beeper.com`) with the returned JWT. Set `BEEPER_API_URL` and `BEEPER_MATRIX_URL` to use different servers.
- The Desktop API routes `POST /v1/app/setup/start`, `/email`, `/response`, and `/register` use the same cloud flow and return the same response shapes as Beeper Desktop.
- "Continue with Google" is not available. Use email.
- `beeper-rs` does not sync Matrix chats yet. After sign-in, the inbox shows only "Note to self".
- Use `--demo` to start with the bundled demo account and demo chats.
- `CentralConfig` and `BeeperStore` enforce an assertion that aborts if the configured working directory points to `BeeperTexts`.

## Cross-Platform and Rust-Native Architecture

- **Cargo workspace**: `crates/beeper-core` contains the Beeper cloud sign-in client, data models, configuration, SQLite store, search index, platform helpers, and CSS theme compiler. `crates/beeper-api` contains the Beeper Desktop API server and client. `crates/beeper-ui` contains the iced interface, fonts, icons, and network logos. The root `beeper-rs` crate contains the binary and the end-to-end tests.
- **Native Iced 0.14 GUI**: Uses `iced` (`0.14.0`) with `wgpu` (`27.0.1`) GPU acceleration and `tiny-skia` software fallback, adapted from the `navify` architecture (`https://github.com/pythoncrazy/navify`).
- **Bundled typography, icons, and network logos**: Embeds `Inter` (`Regular`, `Medium`, `SemiBold`, `Bold`), a `Lucide` icon subset, and CC0 `Simple Icons` network logos directly in the binary.
- **Beeper Desktop 4.3 layout**: Renders the `68px` account rail, the compact Inbox sidebar (`190px` default, drag-resizable) with pinned chats, the rounded conversation card with a floating chat header, date dividers, link previews, quoted replies, voice notes, files, polls, and reactions, the Beeper composer, the Chat Info panel, and a separate Settings window with the Beeper preference groups.
- **Native cross-platform support**: Runs natively on Linux (Wayland and X11), macOS (AppKit / Metal), and Windows (Win32 / DirectX 12 / Vulkan) with OS-specific directory resolution, `file://` URL handling, autostart manifests, and protocol handlers.
- **Fast compilation and linking**: Configured in `.cargo/config.toml` to use `sccache` (`rustc-wrapper = "sccache"`) across all platforms, `mold` v3 on Linux and Windows GNU targets, `rust-lld` on Windows MSVC targets, and unpacked debug info on macOS targets.
- **Rust-native protocols and primitives**: Implements HTTP/1.1, RFC 6455 WebSockets, SHA-256, SHA-1, Base64, URL percent-encoding, RFC 3339 timestamps, CLI argument parsing, and 32x32 RGBA system tray icon rendering directly in Rust.
- **SQLite WAL and inverted search index**: Persists state in `BeeperRs/index.db` using SQLite Write-Ahead Logging (WAL) and indexes chats, messages, and settings in an in-memory inverted index.
- **CSS theme compiler**: Compiles Beeper v4 CSS themes (`variables.css`, `texts.css`, `dracula.css`, `nocturne.css`, `dark-purple.css`, and `fix-lockdown-mode-emojis.css`).

## Local Testing Guide

### 1. Run the Automated End-to-End Test Suite

Run the following command to execute the end-to-end test suite (including `iced_test::Simulator`, `iced_test::Emulator`, headless Wayland via `weston --backend=headless`, and headless X11 via `Xvfb :99`):

```bash
cargo test --test e2e -- --nocapture
```

### 2. Run the Native Desktop GUI and Local Server

Run the following command on your host machine to launch the desktop GUI. The GUI starts the local API server on `127.0.0.1:23374` only when **Settings > Enable Desktop API** is on (default: off). Restart `beeper-rs` after you change this setting:

```bash
cargo run --release
```

### 3. Run the Headless API Server and Query Endpoints

Start the headless server in one terminal:

```bash
cargo run -- serve --demo --host 127.0.0.1 --port 23374
```

In a second terminal, query the server with the CLI or `curl`:

```bash
cargo run -- info --host 127.0.0.1 --port 23374
curl -s http://127.0.0.1:23374/v1/info | jq
curl -s -H "Authorization: Bearer bp_tok_beeper_rs_default_secret" http://127.0.0.1:23374/v1/accounts | jq
curl -s -H "Authorization: Bearer bp_tok_beeper_rs_default_secret" http://127.0.0.1:23374/v1/chats | jq
```

## Development Workflow

1. Create a feature branch from `main` for each change.
2. Write commit messages that follow the Conventional Commits specification.
3. Keep each commit message to one concise sentence in ASD-STE100 Simplified Technical English.
4. Do not include automated tool attribution trailers in commit messages.
5. Open a pull request against `main` and verify that all checks pass.

## License

This project is licensed under the MIT License.
See `LICENSE` and `NOTICE` for details.

## AI Notice

This software project was built using **Gemini 4 Argon**.

## Legal Disclaimer and Non-Affiliation Notice

`beeper-rs` is an independent open-source software project.
This project is **not** affiliated with, endorsed by, sponsored by, approved by, or associated with **Beeper Inc.**, **Automattic Inc.**, **Texts.com**, or any of their subsidiaries or affiliates.

"Beeper", "Texts", "Automattic", "iMessage", "WhatsApp", "Signal", "Telegram", "Slack", "Discord", "Instagram", "Messenger", "LinkedIn", "Google Messages", "Google Voice", "Matrix", "Bluesky", and all other third-party product names, trademarks, service marks, and trade names referenced in this repository are the property of their respective owners.
All references to third-party names, protocols, APIs, and trademarks in this repository are strictly for interoperability and identification purposes (nominative fair use) and do not imply any endorsement, sponsorship, or official status.

This software is provided "AS IS", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and non-infringement.
In no event shall the authors or copyright holders be liable for any claim, damages, account suspension, data loss, or other liability, whether in an action of contract, tort, or otherwise, arising from, out of, or in connection with the software or the use or other dealings in the software.
