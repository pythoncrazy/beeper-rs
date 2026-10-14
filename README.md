# beeper-rs

`beeper-rs` is an independent, unofficial open-source Rust desktop application, HTTP/1.1 REST server, RFC 6455 WebSocket server, and client SDK that implements the public Beeper Desktop API protocol.

## AI Notice

This software project was built using **Gemini 4 Argon**.

## Legal Disclaimer and Non-Affiliation Notice

`beeper-rs` is an independent open-source software project.
This project is **not** affiliated with, endorsed by, sponsored by, approved by, or associated with **Beeper Inc.**, **Automattic Inc.**, **Texts.com**, or any of their subsidiaries or affiliates.

"Beeper", "Texts", "Automattic", "iMessage", "WhatsApp", "Signal", "Telegram", "Slack", "Discord", "Instagram", "Messenger", "LinkedIn", "Google Messages", "Google Voice", "Matrix", "Bluesky", and all other third-party product names, trademarks, service marks, and trade names referenced in this repository are the property of their respective owners.
All references to third-party names, protocols, APIs, and trademarks in this repository are strictly for interoperability and identification purposes (nominative fair use) and do not imply any endorsement, sponsorship, or official status.

This software is provided "AS IS", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and non-infringement.
In no event shall the authors or copyright holders be liable for any claim, damages, account suspension, data loss, or other liability, whether in an action of contract, tort, or otherwise, arising from, out of, or in connection with the software or the use or other dealings in the software.

## Isolated Data Directory Safety

`beeper-rs` never modifies or locks the original Beeper Desktop directory (`BeeperTexts` or `BeeperTexts-<profile>`):

- `beeper-rs` stores all database and media files in an isolated directory (`~/.config/BeeperRs` on Linux, `~/Library/Application Support/BeeperRs` on macOS, or `%APPDATA%\BeeperRs` on Windows).
- On first startup, `beeper-rs` copies `index.db` and `media/` from the original `BeeperTexts` directory (or a directory specified with `--import-from <DIR>`) into `BeeperRs` and skips active lock files.
- `CentralConfig` and `BeeperStore` enforce an assertion that aborts if the configured working directory points to `BeeperTexts`.

## Rust-Native Architecture

- **Minimal dependency footprint**: Depends directly on only 5 crates (`tokio`, `serde`, `serde_json`, `rusqlite`, and `eframe`).
- **Rust-native protocols and primitives**: Implements HTTP/1.1, RFC 6455 WebSockets, SHA-256, SHA-1, Base64, URL percent-encoding, RFC 3339 timestamps, CLI argument parsing, and 32x32 RGBA system tray icon rendering directly in Rust.
- **Native 4-pane GUI**: Uses `eframe` and `egui` with keyboard shortcuts (`Ctrl+K` command palette, `Ctrl+,` settings, `Ctrl+D` Do Not Disturb, `Space` Quick Look, and `Escape` modal dismissal).
- **SQLite WAL and inverted search index**: Persists state in `BeeperRs/index.db` using SQLite Write-Ahead Logging (WAL) and indexes chats, messages, and settings in an in-memory inverted index.
- **CSS theme compiler**: Compiles Beeper v4 CSS themes (`variables.css`, `texts.css`, `dracula.css`, `nocturne.css`, `dark-purple.css`, and `fix-lockdown-mode-emojis.css`).

## Local Testing Guide

### 1. Run the Automated End-to-End Test Suite

Run the following command to execute the end-to-end test suite:

```bash
cargo test --test e2e -- --nocapture
```

### 2. Run the Native Desktop GUI and Local Server

Run the following command on your host machine to launch the 4-pane desktop GUI and local API server on `127.0.0.1:23373`:

```bash
cargo run --release
```

### 3. Run the Headless API Server and Query Endpoints

Start the headless server in one terminal:

```bash
cargo run -- serve --host 127.0.0.1 --port 23373
```

In a second terminal, query the server with the CLI or `curl`:

```bash
cargo run -- info --base-url http://127.0.0.1:23373
curl -s http://127.0.0.1:23373/v1/info | jq
curl -s -H "Authorization: Bearer bp_tok_beeper_rs_default_secret" http://127.0.0.1:23373/v1/accounts | jq
curl -s -H "Authorization: Bearer bp_tok_beeper_rs_default_secret" http://127.0.0.1:23373/v1/chats | jq
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
