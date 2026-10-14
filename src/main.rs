use beeper_api::{
    client::{extract_state_from_index_db, BeeperClient},
    server::serve_http,
};
use beeper_core::{
    config::{CentralConfig, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
    platform::{current_desktop_data_dir, current_original_beeper_data_dir},
    store::{BeeperStore, SharedStore},
};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

const HELP: &str = "Usage: beeper-rs [OPTIONS] [COMMAND]

Commands:
  serve         Run only the Desktop API server (same as --headless)
  info          Print /v1/info from the server at --host and --port
  auth-extract  Print beeperState and bridgeAccounts from index.db

Options:
  --host <HOST>
  --port <PORT>
  --profile <PROFILE>
  --data-dir <DIR>
  --import-from <DIR>
  --demo        Start with demo chats instead of a signed-out account
  --headless

Environment:
  BEEPER_API_URL      Beeper cloud API (default https://api.beeper.com)
  BEEPER_MATRIX_URL   Matrix homeserver (default https://matrix.beeper.com)";

enum Command {
    Gui,
    Serve,
    Info,
    AuthExtract,
}

fn main() {
    let mut host = std::env::var("BEEPER_PAS_HOST").unwrap_or_else(|_| DEFAULT_PAS_HOST.to_string());
    let mut port: u16 =
        std::env::var("BEEPER_PAS_PORT").map_or(DEFAULT_PAS_PORT, |v| v.parse().expect("BEEPER_PAS_PORT is a port"));
    let (mut profile, mut data_dir, mut import_from, mut command, mut demo) = (None, None, None, Command::Gui, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("missing value for {arg}"));
        match arg.as_str() {
            "serve" | "--headless" => command = Command::Serve,
            "info" => command = Command::Info,
            "auth-extract" => command = Command::AuthExtract,
            "--host" | "--pas-host" => host = value(),
            "--port" | "--pas-port" => port = value().parse().expect("--port is a port number"),
            "--profile" => profile = Some(value()),
            "--data-dir" => data_dir = Some(PathBuf::from(value())),
            "--import-from" => import_from = Some(PathBuf::from(value())),
            "--demo" => demo = true,
            "--version" | "-V" => return println!("{APP_NAME} {APP_VERSION}"),
            "--help" | "-h" => return println!("{HELP}"),
            other => panic!("unexpected argument: {other}"),
        }
    }
    let data_dir = data_dir.unwrap_or_else(|| current_desktop_data_dir(profile.as_deref()));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build tokio runtime");
    let pretty = |v: serde_json::Value| println!("{}", serde_json::to_string_pretty(&v).unwrap());
    match command {
        Command::Info => {
            let info = rt.block_on(BeeperClient::new(format!("http://{host}:{port}"), "").info());
            return pretty(serde_json::to_value(info).unwrap());
        }
        Command::AuthExtract => {
            let (state, bridges) = extract_state_from_index_db(&data_dir);
            return pretty(serde_json::json!({ "beeperState": state, "bridgeAccounts": bridges }));
        }
        Command::Gui | Command::Serve => {}
    }
    let source =
        import_from.or_else(|| Some(current_original_beeper_data_dir(profile.as_deref())).filter(|d| d.exists()));
    let mut cfg = CentralConfig::new(data_dir, host.clone(), port, profile).with_source_beeper_dir(source);
    cfg.demo = demo;
    if let Ok(url) = std::env::var("BEEPER_API_URL") {
        cfg.api_url = url;
    }
    if let Ok(url) = std::env::var("BEEPER_MATRIX_URL") {
        cfg.homeserver = url;
    }
    let store: SharedStore = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    let listener = rt.block_on(tokio::net::TcpListener::bind((host.as_str(), port))).expect("bind local PAS port");
    if let Command::Serve = command {
        return rt.block_on(serve_http(listener, store));
    }
    let server_store = store.clone();
    std::thread::spawn(move || rt.block_on(serve_http(listener, server_store)));
    beeper_ui::run_native_gui(store).expect("run native GUI");
}
