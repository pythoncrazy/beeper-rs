# beeper-rs

`beeper-rs` is an independent, unofficial open-source Rust desktop application, HTTP REST server, WebSocket event server, and client SDK that implements the public Beeper Desktop API protocol.

## Legal Disclaimer and Non-Affiliation Notice

`beeper-rs` is an independent open-source software project.
This project is **not** affiliated with, endorsed by, sponsored by, approved by, or associated with **Beeper Inc.**, **Automattic Inc.**, **Texts.com**, or any of their subsidiaries or affiliates.

"Beeper", "Texts", "Automattic", "iMessage", "WhatsApp", "Signal", "Telegram", "Slack", "Discord", "Instagram", "Messenger", "LinkedIn", "Google Messages", "Google Voice", "Matrix", "Bluesky", and all other third-party product names, trademarks, service marks, and trade names referenced in this repository are the property of their respective owners.
All references to third-party names, protocols, APIs, and trademarks in this repository are strictly for interoperability and identification purposes (nominative fair use) and do not imply any endorsement, sponsorship, or official status.

This software is provided "AS IS", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and non-infringement.
In no event shall the authors or copyright holders be liable for any claim, damages, account suspension, data loss, or other liability, whether in an action of contract, tort, or otherwise, arising from, out of, or in connection with the software or the use or other dealings in the software.

## Features

- Provides a native GUI with `eframe` and `egui`.
- Renders a 32x32 RGBA system tray icon with unread and Do Not Disturb indicators.
- Persists application state in `BeeperTexts/index.db` with SQLite Write-Ahead Logging (WAL).
- Indexes chats, messages, participants, voice transcripts, and settings in an in-memory inverted index.
- Implements the Beeper Desktop HTTP REST API (`/v1/*` and `/oauth/*`) and WebSocket endpoint (`/v1/ws`).
- Supports OAuth2 PKCE (`S256`), email login, E2EE SAS verification, recovery keys, and bridge login sessions.
- Compiles Beeper v4 CSS themes (`variables.css`, `texts.css`, `dracula.css`, `nocturne.css`, `dark-purple.css`, and `fix-lockdown-mode-emojis.css`).
- Generates autostart manifests for Linux, macOS, and Windows.

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
curl -s -H "Authorization: Bearer beeper-local-dev-token" http://127.0.0.1:23373/v1/accounts | jq
curl -s -H "Authorization: Bearer beeper-local-dev-token" http://127.0.0.1:23373/v1/chats | jq
```

## Development Workflow

1. Create a feature branch from `main` for each change.
2. Write commit messages that follow the Conventional Commits specification.
3. Keep each commit message to one concise sentence in ASD-STE100 Simplified Technical English.
4. Do not include automated tool attribution trailers in commit messages.
5. Open a pull request against `main` and verify that all checks pass.

## License

This project is licensed under the Unlicense, MIT License, or Apache License 2.0 at your option.
See `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE` for details.
