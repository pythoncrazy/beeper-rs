#![recursion_limit = "256"]
use beeper_api::{client::BeeperClient, server::serve_http};
use beeper_core::{
    config::{migrate_legacy_data_dir, CentralConfig, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
    platform::current_desktop_data_dir,
    store::{BeeperStore, SharedStore, StoreLock},
};
use std::path::PathBuf;

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
  beeper://...  Open a Beeper link, in the running instance if there is one

Environment:
  BEEPER_API_URL      Beeper cloud API (default https://api.beeper.com)
  BEEPER_MATRIX_URL   Matrix homeserver (default https://matrix.beeper.com)";

#[cfg(target_os = "linux")]
mod display {
    use wayland_client::globals::GlobalListContents as G;
    use wayland_client::protocol::wl_registry::{Event as E, WlRegistry as R};
    use wayland_client::{Connection as C, Dispatch, QueueHandle as Q};
    use x11rb::connection::RequestConnection;

    struct Probe;

    impl Dispatch<R, G> for Probe {
        fn event(_: &mut Self, _: &R, _: E, _: &G, _: &C, _: &Q<Self>) {}
    }

    /// Picks the window system and the renderer. It sets environment variables, so call it before any thread starts.
    /// It uses X11 when the Wayland compositor does not draw window frames. It uses tiny-skia when the display
    /// cannot show GPU frames, because wgpu then loads every Vulkan driver and renders on the CPU with lavapipe.
    pub fn setup() {
        let wayland = C::connect_to_env().ok().and_then(|c| {
            let (globals, _) = wayland_client::globals::registry_queue_init::<Probe>(&c).ok()?;
            Some(globals.contents().with_list(|l| l.iter().map(|g| g.interface.clone()).collect::<Vec<_>>()))
        });
        let has = |interface: &str| wayland.iter().flatten().any(|g| g.starts_with(interface));
        let set = ["DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"].map(std::env::var_os);
        let x11 = set.map(|v| v.is_some()) == [true, true, false] && !has("zxdg_decoration");
        if x11 {
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        let gpu_display = match wayland.is_some() && !x11 {
            true => has("zwp_linux_dmabuf_v1"),
            false => x11rb::connect(None).is_ok_and(|(c, _)| {
                ["DRI3", "NV-GLX"].iter().any(|e| c.extension_information(e).is_ok_and(|i| i.is_some()))
            }),
        };
        let render_node = std::fs::read_dir("/dev/dri")
            .is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")));
        if std::env::var_os("ICED_BACKEND").is_none() && !(gpu_display && render_node) {
            unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
        }
    }
}

enum Command {
    Gui,
    Serve,
    Info,
}

fn main() {
    let mut launch_host = std::env::var("BEEPER_PAS_HOST").ok();
    let mut port: u16 =
        std::env::var("BEEPER_PAS_PORT").map_or(DEFAULT_PAS_PORT, |v| v.parse().expect("BEEPER_PAS_PORT is a port"));
    let (mut profile, mut data_dir, mut command, mut demo, mut link) = (None, None, Command::Gui, false, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("missing value for {arg}"));
        match arg.as_str() {
            "serve" | "--headless" => command = Command::Serve,
            "info" => command = Command::Info,
            "--host" | "--pas-host" | "--pas-hostname" => launch_host = Some(value()),
            "--port" | "--pas-port" => port = value().parse().expect("--port is a port number"),
            "--profile" => profile = Some(value()),
            "--data-dir" => data_dir = Some(PathBuf::from(value())),
            "--demo" => demo = true,
            "--version" | "-V" => return println!("{APP_NAME} {APP_VERSION}"),
            "--help" | "-h" => return println!("{HELP}"),
            l if l.starts_with("beeper://") => link = Some(arg.clone()),
            other => panic!("unexpected argument: {other}"),
        }
    }
    #[cfg(target_os = "linux")]
    if let Command::Gui = command {
        display::setup();
    }
    let host = launch_host.clone().unwrap_or_else(|| DEFAULT_PAS_HOST.to_string());
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
    #[cfg(target_os = "linux")]
    let first = match std::env::var("ALLOW_MULTIPLE_INSTANCES").as_deref() {
        Ok("true") => None,
        _ => Some(beeper_core::platform::single_instance(&data_dir, &std::env::args().skip(1).collect::<Vec<_>>())),
    };
    #[cfg(target_os = "linux")]
    if let Some(None) = first {
        return eprintln!("Quitting because another instance is running");
    }
    let mut cfg = CentralConfig::new(data_dir, host.clone(), port, profile);
    (cfg.demo, cfg.launch_host, cfg.link) = (demo, launch_host.is_some(), link);
    if let Ok(url) = std::env::var("BEEPER_API_URL") {
        cfg.api_url = url;
    }
    if let Ok(url) = std::env::var("BEEPER_MATRIX_URL") {
        cfg.homeserver = url;
    }
    let store: SharedStore = StoreLock::shared(BeeperStore::from_config(cfg));
    #[cfg(target_os = "linux")]
    if let Some(Some(first)) = first {
        beeper_core::platform::forward_launches(first, store.read().launch_tx.clone());
    }
    let host = {
        let mut st = store.write();
        st.config.remote = !st.config.launch_host && st.settings.remote_access;
        match st.config.remote {
            true => (st.config.host = CentralConfig::external_host(), "0.0.0.0".to_string()).1,
            false => host,
        }
    };
    if !store.read().config.demo {
        rt.spawn(beeper_core::matrix::run(store.clone()));
    }
    let bind = move |rt: &tokio::runtime::Runtime| {
        rt.block_on(tokio::net::TcpListener::bind((host.as_str(), port))).map_err(|e| {
            format!("bind the Desktop API to {host}:{port}: {e}. Another app uses this port. Use --port <PORT>.")
        })
    };
    if let Command::Serve = command {
        return rt.block_on(serve_http(bind(&rt).unwrap_or_else(|e| panic!("{e}")), store));
    }
    let mut on = {
        let st = store.read();
        st.api_on.send_replace(st.settings.enable_pas_api);
        st.api_on.subscribe()
    };
    let server_store = store.clone();
    std::thread::spawn(move || loop {
        rt.block_on(on.wait_for(|on| *on)).expect("the store outlives the server");
        match bind(&rt) {
            Ok(listener) => rt.block_on(async {
                tokio::select! {
                    () = serve_http(listener, server_store.clone()) => {}
                    off = on.wait_for(|on| !*on) => drop(off.expect("the store outlives the server")),
                }
            }),
            Err(e) => {
                eprintln!("{e}");
                rt.block_on(on.wait_for(|on| !*on)).expect("the store outlives the server");
            }
        }
    });
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
