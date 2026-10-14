# beeper-rs

`beeper-rs` is a native Rust desktop application, HTTP REST server, WebSocket event server, and client SDK for Beeper Desktop.

## Features

- Provides a native GUI with `eframe` and `egui`.
- Renders a 32x32 RGBA system tray icon with unread and Do Not Disturb indicators.
- Persists application state in `BeeperTexts/index.db` with SQLite Write-Ahead Logging (WAL).
- Indexes chats, messages, participants, voice transcripts, and settings in an in-memory inverted index.
- Implements the Beeper Desktop HTTP REST API (`/v1/*` and `/oauth/*`) and WebSocket endpoint (`/v1/ws`).
- Supports OAuth2 PKCE (`S256`), email login, E2EE SAS verification, recovery keys, and bridge login sessions.
- Compiles Beeper v4 CSS themes (`variables.css`, `texts.css`, `dracula.css`, `nocturne.css`, `dark-purple.css`, and `fix-lockdown-mode-emojis.css`).
- Generates autostart manifests for Linux, macOS, and Windows.

## Build and Run

Run the following command to build the project:

```bash
cargo build --release
```

Run the following command to start the server and GUI:

```bash
cargo run --
```

Run the following command to start the headless API server:

```bash
cargo run -- serve --host 127.0.0.1 --port 23373
```

Run the following command to run the end-to-end test suite:

```bash
cargo test --test e2e
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
