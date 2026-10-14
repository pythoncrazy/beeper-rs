use beeper_api::{client::BeeperClient, server::serve_http};
use beeper_core::{
    config::{migrate_legacy_data_dir, CentralConfig, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
    import::read_beeper_data,
    platform::current_desktop_data_dir,
    store::{BeeperStore, SharedStore},
};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

const HELP: &str = "Usage: beeper-rs [OPTIONS] [COMMAND]

Commands:
  serve         Run only the Desktop API server (same as --headless)
                The GUI starts the Desktop API only when Settings > Enable Desktop API is on
  info          Print /v1/info from the server at --host and --port

Options:
  --host <HOST>
  --port <PORT>
  --profile <PROFILE>
  --data-dir <DIR>
  --import-from <DIR>  Copy chats and the session from a Beeper Desktop data dir on first start
  --demo        Start with demo chats instead of a signed-out account
  --headless

Environment:
  BEEPER_API_URL      Beeper cloud API (default https://api.beeper.com)
  BEEPER_MATRIX_URL   Matrix homeserver (default https://matrix.beeper.com)";

enum Command {
    Gui,
    Serve,
    Info,
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
    let data_dir = data_dir.unwrap_or_else(|| {
        let dir = current_desktop_data_dir(profile.as_deref());
        migrate_legacy_data_dir(&dir, profile.as_deref());
        dir
    });
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build tokio runtime");
    if let Command::Info = command {
        let info = rt.block_on(BeeperClient::new(format!("http://{host}:{port}"), "").info());
        return println!("{}", serde_json::to_string_pretty(&info).unwrap());
    }
    let mut cfg = CentralConfig::new(data_dir, host.clone(), port, profile);
    cfg.demo = demo;
    if let Ok(url) = std::env::var("BEEPER_API_URL") {
        cfg.api_url = url;
    }
    if let Ok(url) = std::env::var("BEEPER_MATRIX_URL") {
        cfg.homeserver = url;
    }
    let mut store = BeeperStore::from_config(cfg);
    if let Some(dir) = import_from {
        assert!(
            store.setup.is_none(),
            "--import-from needs a new data dir, and {} is already set up",
            store.data_dir.display()
        );
        store.import_beeper(read_beeper_data(&dir).unwrap_or_else(|e| panic!("{e}")));
    }
    let store: SharedStore = Arc::new(RwLock::new(store));
    let serve = matches!(command, Command::Serve);
    if serve || store.read().unwrap().settings.enable_pas_api {
        let listener = rt.block_on(tokio::net::TcpListener::bind((host.as_str(), port))).unwrap_or_else(|e| {
            panic!("bind the Desktop API to {host}:{port}: {e}. Another app uses this port. Use --port <PORT>.")
        });
        let server_store = store.clone();
        if serve {
            return rt.block_on(serve_http(listener, server_store));
        }
        std::thread::spawn(move || rt.block_on(serve_http(listener, server_store)));
    }
    beeper_ui::run_native_gui(store).expect("run native GUI");
}
