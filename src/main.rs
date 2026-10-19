#![recursion_limit = "256"]
use beeper_api::{client::BeeperClient, server::serve_http};
use beeper_core::{
    config::{migrate_legacy_data_dir, CentralConfig, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
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
  --demo        Start with demo chats instead of a signed-out account
  --headless

Environment:
  BEEPER_API_URL      Beeper cloud API (default https://api.beeper.com)
  BEEPER_MATRIX_URL   Matrix homeserver (default https://matrix.beeper.com)";

#[cfg(target_os = "linux")]
mod frame {
    use wayland_client::globals::{Global, GlobalListContents as G};
    use wayland_client::protocol::wl_registry::{Event as E, WlRegistry as R};
    use wayland_client::{Connection as C, Dispatch, QueueHandle as Q};

    struct Probe;

    impl Dispatch<R, G> for Probe {
        fn event(_: &mut Self, _: &R, _: E, _: &G, _: &C, _: &Q<Self>) {}
    }

    pub fn x11() -> bool {
        let set = ["DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"].map(std::env::var_os);
        let ssd = || {
            let c = C::connect_to_env().ok()?;
            let (globals, _) = wayland_client::globals::registry_queue_init::<Probe>(&c).ok()?;
            let has = |l: &[Global]| l.iter().any(|g| g.interface.starts_with("zxdg_decoration"));
            Some(globals.contents().with_list(has))
        };
        set.map(|v| v.is_some()) == [true, true, false] && ssd().is_none_or(|s| !s)
    }
}

enum Command {
    Gui,
    Serve,
    Info,
}

fn main() {
    #[cfg(target_os = "linux")]
    if frame::x11() {
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
    let mut host = std::env::var("BEEPER_PAS_HOST").unwrap_or_else(|_| DEFAULT_PAS_HOST.to_string());
    let mut port: u16 =
        std::env::var("BEEPER_PAS_PORT").map_or(DEFAULT_PAS_PORT, |v| v.parse().expect("BEEPER_PAS_PORT is a port"));
    let (mut profile, mut data_dir, mut command, mut demo) = (None, None, Command::Gui, false);
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
    let store: SharedStore = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    if !store.read().unwrap().config.demo {
        rt.spawn(beeper_core::matrix::run(store.clone()));
    }
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
    if beeper_ui::RELAUNCH.load(std::sync::atomic::Ordering::Relaxed) {
        let (exe, args) = (std::env::current_exe().expect("the program path"), std::env::args_os().skip(1));
        #[cfg(unix)]
        panic!("relaunch: {}", std::os::unix::process::CommandExt::exec(std::process::Command::new(exe).args(args)));
        #[cfg(not(unix))]
        #[expect(clippy::zombie_processes, reason = "the new process outlives this one, which exits now")]
        std::process::Command::new(exe).args(args).spawn().expect("relaunch");
    }
}
