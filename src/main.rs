#![recursion_limit = "256"]
mod cli;

use beeper_api::{client::BeeperClient, server::serve_http};
use beeper_core::{
    config::{migrate_legacy_data_dir, CentralConfig, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT, SERVER_ENVS},
    platform::current_desktop_data_dir,
    store::{BeeperStore, SharedStore, StoreLock},
};
use std::{ops::RangeInclusive, path::PathBuf};
use tokio::{net::TcpListener, runtime::Runtime};

const BIN: &str = env!("CARGO_BIN_NAME");
const USAGE: &str = concat!("USAGE\n  $ ", env!("CARGO_BIN_NAME"), " [COMMAND] [FLAGS] [LINK]");
const ARGUMENTS: &str = "ARGUMENTS
  LINK  A beeper:// link to open, in the running instance if there is one";
const ENVIRONMENT: &str = "ENVIRONMENT
  BEEPER_API_URL          Beeper cloud API (default: https://api.beeper.com)
  BEEPER_MATRIX_URL       Matrix homeserver (default: https://matrix.beeper.com)
  BEEPER_PROFILE          Profile name, like --profile
  BEEPER_SERVER_DATA_DIR  Data folder of a managed Server target of the CLI, where serve runs
  BEEPER_USER_DATA_DIR    Data folder, like --data-dir";

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Command {
    #[default]
    Gui,
    Serve,
    Info,
    Help,
    Version,
}

#[derive(Debug, Default, PartialEq)]
struct Args {
    command: Command,
    host: Option<String>,
    port: Option<u16>,
    profile: Option<String>,
    data_dir: Option<PathBuf>,
    /// The domain of the Beeper cloud that `--server-env` selects.
    domain: Option<&'static str>,
    demo: bool,
    /// A `beeper://` link to open, in the running instance if there is one.
    link: Option<String>,
    /// Chromium's `--ozone-platform` or `--ozone-platform-hint` of Desktop: `x11`, `wayland` or `auto`.
    ozone: Option<String>,
    /// The first switch that no flag knows. Chromium ignores it for the app, and oclif rejects it for a command.
    unknown: Option<String>,
}

const COMMANDS: [(&str, Command, &str); 3] = [
    ("help", Command::Help, "Show this help"),
    ("info", Command::Info, "Print /v1/info from the server at --host and --port"),
    ("serve", Command::Serve, "Run only the Desktop API server (the app runs it only when Enable Desktop API is on)"),
];

type Set = fn(&mut Args, &str) -> Result<(), String>;

/// The flags: the names, whether the flag takes a value, the help, and the setter. The first long name shows in the
/// help. Desktop's launchers give a value as `--name=value`, and people give it as `--name value`. Both are valid.
#[rustfmt::skip]
#[expect(clippy::unit_arg, reason = "a setter assigns the field in its `Ok`")]
const FLAGS: [(&[&str], bool, &str, Set); 9] = [
    (&["-h", "--help"], false, "Show this help", |a, _| Ok(a.command = Command::Help)),
    (&["-V", "--version"], false, "Show the version", |a, _| Ok(a.command = Command::Version)),
    (&["--data-dir"], true, "Data folder (default: beeper-rs[-<profile>] in the config folder)", |a, v| Ok(a.data_dir = Some(v.into()))),
    (&["--demo"], false, "Start with demo chats instead of a signed-out account", |a, _| Ok(a.demo = true)),
    (&["--headless"], false, "Same as serve", |a, _| Ok(a.command = Command::Serve)),
    (&["--host", "--pas-host", "--pas-hostname"], true, "Desktop API host (default: 127.0.0.1)", |a, v| Ok(a.host = Some(v.into()))),
    (&["--port", "--pas-port"], true, "Desktop API port (default: the first free port from 23373 to 23423)", |a, v| {
        let port = v.parse().ok().filter(|p| *p > 0);
        Ok(a.port = Some(port.ok_or(format!("Parsing --port: Expected an integer from 1 to 65535 but received: {v}"))?))
    }),
    (&["--profile"], true, "Profile name, which selects the data folder", |a, v| Ok(a.profile = Some(v.into()))),
    (&["--server-env"], true, "Beeper cloud: production, staging, dev, or local (default: production)", |a, v| {
        let names = SERVER_ENVS.map(|e| e.0).join(", ");
        let domain = SERVER_ENVS.iter().find(|e| e.0 == v).map(|e| e.1);
        Ok(a.domain = Some(domain.ok_or(format!("Expected --server-env={v} to be one of: {names}"))?))
    }),
];

fn flag(name: &str) -> Option<&'static (&'static [&'static str], bool, &'static str, Set)> {
    FLAGS.iter().find(|f| f.0.contains(&name))
}

/// Reads the command line like oclif: a usage error exits with 2, and an unknown command exits with 127.
fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, (String, i32)> {
    let mut args = Args::default();
    let mut argv = argv.into_iter().peekable();
    while let Some(arg) = argv.next() {
        if arg.starts_with("beeper://") {
            args.link = Some(arg);
            continue;
        }
        if !arg.starts_with('-') {
            let command = COMMANDS.iter().find(|c| c.0 == arg).map(|c| c.1);
            match (args.command, command) {
                (Command::Gui, Some(command)) => args.command = command,
                (Command::Gui, None) => {
                    let words: Vec<String> =
                        std::iter::once(arg).chain(argv.take_while(|a| !a.starts_with('-'))).collect();
                    return Err((words.join(" "), 127));
                }
                _ => return Err((format!("Unexpected argument: {arg}"), 2)),
            }
            continue;
        }
        let (name, inline) = arg.split_once('=').map_or((arg.as_str(), None), |(n, v)| (n, Some(v.to_string())));
        let Some(&(_, takes_value, _, set)) = flag(name) else {
            if matches!(name, "--ozone-platform" | "--ozone-platform-hint") {
                args.ozone = inline.clone();
            }
            match name.starts_with("--") && args.command == Command::Gui {
                true => args.unknown = args.unknown.or(Some(arg)),
                false => return Err((format!("Nonexistent flag: {arg}"), 2)),
            }
            continue;
        };
        let value = match (takes_value, inline) {
            (false, None) => String::new(),
            (false, Some(_)) => return Err((format!("Nonexistent flag: {arg}"), 2)),
            (true, inline) => inline
                .or_else(|| argv.next_if(|v| flag(v.split_once('=').map_or(v, |p| p.0)).is_none()))
                .filter(|v| !v.is_empty())
                .ok_or((format!("Flag {name} expects a value"), 2))?,
        };
        set(&mut args, &value).map_err(|e| (e, 2))?;
    }
    match (&args.unknown, args.command) {
        (Some(arg), command) if command != Command::Gui => Err((format!("Nonexistent flag: {arg}"), 2)),
        _ => Ok(args),
    }
}

/// The help in oclif's layout, with the client commands and topics.
fn help() -> String {
    let rows = COMMANDS.iter().map(|(name, _, text)| (name.to_string(), text.to_string()));
    let mut rows: Vec<(String, String)> = rows.chain(cli::commands::root_rows()).collect();
    rows.sort();
    let width = rows.iter().map(|r| r.0.len()).max().expect("a command");
    let commands = rows.iter().map(|(name, text)| format!("  {name:<width$}  {text}")).collect::<Vec<_>>().join("\n");
    let flags = FLAGS.map(|(names, value, text, _)| {
        let (short, long): (Vec<&str>, Vec<&str>) = names.iter().partition(|n| !n.starts_with("--"));
        let short = short.first().map_or("    ".into(), |s| format!("{s}, "));
        let head = format!("{short}{}{}", long[0], if value { "=<value>" } else { "" });
        let aliases = match long[1..] {
            [] => String::new(),
            ref aliases => format!("\n{:28}Aliases: {}", "", aliases.join(", ")),
        };
        format!("  {head:<26}{text}{aliases}")
    });
    let flags = flags.join("\n");
    format!(
        "Beeper chat app and Desktop API server\n\n{USAGE}\n\n{ARGUMENTS}\n\nCOMMANDS\n{commands}\n\nFLAGS\n{flags}\n\n{ENVIRONMENT}"
    )
}

/// The version in oclif's layout, with the Node names of the platform and the CPU.
fn version() -> String {
    let node = |s: &'static str| {
        let names = [("macos", "darwin"), ("windows", "win32"), ("x86_64", "x64"), ("aarch64", "arm64")];
        names.iter().find(|n| n.0 == s).map_or(s, |n| n.1)
    };
    format!("{BIN}/{APP_VERSION} {}-{}", node(std::env::consts::OS), node(std::env::consts::ARCH))
}

/// Desktop's `getPort`: binds the first free port of `ports`, else a random free port.
fn bind_free(rt: &Runtime, host: &str, ports: RangeInclusive<u16>) -> std::io::Result<TcpListener> {
    let bind = |port| rt.block_on(TcpListener::bind((host, port)));
    let busy = |r: &std::io::Result<_>| {
        r.as_ref()
            .is_err_and(|e| matches!(e.kind(), std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied))
    };
    ports.map(bind).find(|r| !busy(r)).unwrap_or_else(|| bind(0))
}

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

    /// Picks the renderer and the cursor, and starts opening GTK for the title bar. It sets environment variables,
    /// so call it before any thread starts.
    ///
    /// Like Desktop's Electron, it runs on Wayland when there is a compositor, and draws the window frame itself
    /// when the compositor does not (GNOME): under XWayland the scale and the cursor follow X resources that GNOME
    /// sets late or not at all, so the cursor turns small. It uses tiny-skia when the display cannot show GPU
    /// frames, because wgpu then loads every Vulkan driver and renders on the CPU with lavapipe.
    pub fn setup(ozone: Option<&str>) {
        let wayland = C::connect_to_env().ok().and_then(|c| {
            let (globals, _) = wayland_client::globals::registry_queue_init::<Probe>(&c).ok()?;
            Some(globals.contents().with_list(|l| l.iter().map(|g| g.interface.clone()).collect::<Vec<_>>()))
        });
        // Chromium's `--ozone-platform=x11` keeps the app on X11, under XWayland on a Wayland desktop.
        let wayland = wayland.filter(|_| ozone != Some("x11") || std::env::var_os("DISPLAY").is_none());
        if wayland.is_none() && ozone == Some("x11") {
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        let has = |interface: &str| wayland.iter().flatten().any(|g| g == interface);
        if wayland.is_some() && !has("wp_cursor_shape_manager_v1") {
            cursor_theme();
        }
        let gpu_display = match wayland.is_some() {
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
        // Without server-side decorations the window draws its own title bar, with the GTK theme like Electron's.
        if wayland.is_some() && !has("zxdg_decoration_manager_v1") {
            // GTK only paints the frame off screen, so its accessibility bridge would register on a bus that nothing
            // pumps.
            if std::env::var_os("NO_AT_BRIDGE").is_none() {
                unsafe { std::env::set_var("NO_AT_BRIDGE", "1") };
            }
            // The launcher's activation token belongs to the first window, which takes the focus with it, and not
            // to the GTK that draws the frames.
            let var = |k: &&str| std::env::var(k).ok().filter(|v| !v.is_empty());
            let token = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"].iter().find_map(var);
            if let Some(token) = token {
                sctk_adwaita::set_activation_token(token);
                unsafe { std::env::remove_var("XDG_ACTIVATION_TOKEN") };
                unsafe { std::env::remove_var("DESKTOP_STARTUP_ID") };
            }
            sctk_adwaita::preload();
            // The first frames draw from the cached theme; once GTK opens, and whenever its settings change, the
            // windows draw again with it.
            sctk_adwaita::on_ready(|| beeper_ui::FRAME_READY.notify_one());
        }
    }

    /// A compositor without the cursor-shape protocol (GNOME before 48) leaves the cursor to the app, and winit
    /// draws it from `XCURSOR_THEME` and `XCURSOR_SIZE`, which GNOME does not set. GTK and Chromium read the
    /// cursor of the desktop settings, so this passes those on.
    fn cursor_theme() {
        let get = |key: &str| {
            let out =
                std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).output();
            let out = out.ok().filter(|o| o.status.success())?;
            let value = String::from_utf8(out.stdout).ok()?.trim().trim_matches('\'').to_string();
            Some(value).filter(|v| !v.is_empty())
        };
        let unset = [("XCURSOR_THEME", "cursor-theme"), ("XCURSOR_SIZE", "cursor-size")]
            .into_iter()
            .filter(|(var, _)| std::env::var_os(var).is_none());
        for (var, value) in unset.filter_map(|(var, key)| Some((var, get(key)?))) {
            unsafe { std::env::set_var(var, value) };
        }
    }
}

/// Tokio starts one worker for each CPU. A chat client does not need more than a few.
const MAX_WORKERS: usize = 4;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().is_some_and(|word| cli::is_cli(word)) {
        std::process::exit(cli::run(&argv));
    }
    // `help <command>` is the help of a client command, like oclif.
    if let ["help", words @ ..] = argv.as_slice().iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        if words.first().is_some_and(|word| cli::is_cli(word)) {
            let words: Vec<String> = words.iter().map(|w| w.to_string()).chain(["--help".into()]).collect();
            std::process::exit(cli::run(&words));
        }
    }
    let env = |key: &str, flag: &str| std::env::var(key).ok().map(|v| format!("{flag}={v}"));
    let argv = [env("BEEPER_PAS_HOST", "--host"), env("BEEPER_PAS_PORT", "--port")].into_iter().flatten();
    let Args { command, host: launch_host, port, profile, data_dir, domain, demo, link, ozone, .. } =
        parse(argv.chain(std::env::args().skip(1))).unwrap_or_else(|(error, code)| {
            match code {
                127 => drop(cli::not_a_command(&error, None)),
                _ => eprintln!("Error: {error}\nSee more help with --help\n\n{USAGE}"),
            }
            std::process::exit(code)
        });
    match command {
        Command::Help => return println!("{}", help()),
        Command::Version => return println!("{}", version()),
        Command::Gui | Command::Serve | Command::Info => {}
    }
    // The CLI's managed targets: Beeper Server serves headless in `BEEPER_SERVER_DATA_DIR`, and Desktop reads
    // `BEEPER_PROFILE` and `BEEPER_USER_DATA_DIR`.
    let var = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let server_dir = var("BEEPER_SERVER_DATA_DIR");
    let command = if command == Command::Gui && server_dir.is_some() { Command::Serve } else { command };
    let profile = profile.or_else(|| var("BEEPER_PROFILE"));
    let data_dir = data_dir.or_else(|| server_dir.or_else(|| var("BEEPER_USER_DATA_DIR")).map(PathBuf::from));
    // Only Linux has a choice of display server.
    #[cfg(target_os = "linux")]
    if let Command::Gui = command {
        display::setup(ozone.as_deref());
    }
    #[cfg(not(target_os = "linux"))]
    let _ = ozone;
    let host = launch_host.clone().unwrap_or_else(|| DEFAULT_PAS_HOST.to_string());
    let ports = port.map_or(DEFAULT_PAS_PORT..=DEFAULT_PAS_PORT + 50, |p| p..=p);
    let data_dir = data_dir.unwrap_or_else(|| {
        let dir = current_desktop_data_dir(profile.as_deref());
        migrate_legacy_data_dir(&dir, profile.as_deref());
        dir
    });
    let workers = std::thread::available_parallelism().expect("count the CPUs").get().min(MAX_WORKERS);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(workers).enable_all().build();
    // The GUI, the Desktop API and Matrix share this runtime until the process exits.
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(rt.expect("build tokio runtime")));
    if let Command::Info = command {
        let client = BeeperClient::new(format!("http://{host}:{}", ports.start()), "");
        match rt.block_on(client.call("GET", "/v1/info", None)) {
            Ok(info) => return println!("{}", serde_json::to_string_pretty(&info).expect("JSON values serialize")),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
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
    let mut cfg = CentralConfig::new(data_dir, host.clone(), *ports.start(), profile);
    (cfg.demo, cfg.launch_host, cfg.link) = (demo, launch_host.is_some(), link);
    if let Some(domain) = domain {
        (cfg.api_url, cfg.homeserver) = (format!("https://api.{domain}"), format!("https://matrix.{domain}"));
    }
    if let Ok(url) = std::env::var("BEEPER_API_URL") {
        cfg.api_url = url;
    }
    if let Ok(url) = std::env::var("BEEPER_MATRIX_URL") {
        cfg.homeserver = url;
    }
    let store: SharedStore = StoreLock::shared(BeeperStore::open(cfg));
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
    let bind = move |store: &SharedStore| {
        let listener =
            bind_free(rt, &host, ports.clone()).map_err(|e| format!("bind the Desktop API to {host}: {e}"))?;
        store.write().config.port = listener.local_addr().expect("a bound listener has an address").port();
        Ok::<_, String>(listener)
    };
    // `serve` answers the info and the spec at once. The other requests and Matrix wait for the lock of the load.
    let server = matches!(command, Command::Serve).then(|| {
        let listener = bind(&store).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1)
        });
        serve_http(listener, store.clone())
    });
    let mut loading = store.write();
    let server = server.map(|server| rt.spawn(server));
    if !loading.config.demo {
        rt.spawn(beeper_core::matrix::run(store.clone()));
    }
    // The GUI loads its fonts while the chats load. The join comes before the GUI sets its environment variables.
    let fonts = server.is_none().then(|| std::thread::spawn(beeper_ui::style::ensure_fonts_loaded));
    loading.load_chats();
    drop(loading);
    if let Some(fonts) = fonts {
        fonts.join().expect("load the fonts");
    }
    if let Some(server) = server {
        return rt.block_on(server).expect("serve the Desktop API");
    }
    let mut on = {
        let st = store.read();
        st.api_on.send_replace(st.settings.enable_pas_api);
        st.api_on.subscribe()
    };
    let server_store = store.clone();
    std::thread::spawn(move || loop {
        rt.block_on(on.wait_for(|on| *on)).expect("the store outlives the server");
        match bind(&server_store) {
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
    let _rt = rt.enter();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(argv: &[&str]) -> Result<Args, (String, i32)> {
        parse(argv.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parse_reads_both_value_forms() {
        let want = Args {
            command: Command::Serve,
            host: Some("127.0.0.1".into()),
            port: Some(23471),
            data_dir: Some("/tmp/d".into()),
            domain: Some("beeper-staging.com"),
            ..Args::default()
        };
        let spaced =
            ["serve", "--host", "127.0.0.1", "--port", "23471", "--data-dir", "/tmp/d", "--server-env", "staging"];
        assert_eq!(run(&spaced), Ok(want));
        let server = ["--headless", "--host=127.0.0.1", "--port=23471", "--data-dir=/tmp/d", "--server-env=staging"];
        assert_eq!(run(&server).map(|a| (a.port, a.domain)), Ok((Some(23471), Some("beeper-staging.com"))));
        let desktop = run(&["--pas-hostname=localhost", "--pas-port=23472", "--demo"]).unwrap();
        assert_eq!((desktop.host.as_deref(), desktop.port, desktop.demo), (Some("localhost"), Some(23472), true));
        assert_eq!(run(&["help"]).map(|a| a.command), Ok(Command::Help));
        assert_eq!(run(&["beeper://chat/x"]).map(|a| a.link), Ok(Some("beeper://chat/x".into())));
        assert_eq!(run(&["--port=1", "--port", "2"]).map(|a| a.port), Ok(Some(2)), "the last value wins");
    }

    /// Desktop is Electron: Chromium ignores the switches that it does not know, and `--ozone-platform` picks X11 or
    /// Wayland.
    #[test]
    fn the_app_ignores_switches_like_chromium() {
        let app = run(&["--enable-features=UseOzonePlatform", "--ozone-platform=x11", "--no-tls-verify", "--demo"]);
        assert_eq!(
            app.as_ref().map(|a| (a.command, a.demo, a.ozone.as_deref())),
            Ok((Command::Gui, true, Some("x11")))
        );
        assert_eq!(run(&["--ozone-platform-hint=wayland"]).map(|a| a.ozone), Ok(Some("wayland".into())));
    }

    #[test]
    fn parse_rejects_bad_command_lines() {
        let port = |v: &str| format!("Parsing --port: Expected an integer from 1 to 65535 but received: {v}");
        let cases = [
            (&["serve", "--bogus"][..], "Nonexistent flag: --bogus".into(), 2),
            (&["--bogus", "serve"], "Nonexistent flag: --bogus".into(), 2),
            (&["-v"], "Nonexistent flag: -v".into(), 2),
            (&["--demo=1"], "Nonexistent flag: --demo=1".into(), 2),
            (&["chatz"], "chatz".into(), 127),
            (&["chatz", "list", "--json"], "chatz list".into(), 127),
            (&["serve", "extra"], "Unexpected argument: extra".into(), 2),
            (&["--port"], "Flag --port expects a value".into(), 2),
            (&["--port", "--data-dir", "/tmp/d"], "Flag --port expects a value".into(), 2),
            (&["--host="], "Flag --host expects a value".into(), 2),
            (&["--port", "abc"], port("abc"), 2),
            (&["--pas-port=0"], port("0"), 2),
            (&["--port=65536"], port("65536"), 2),
            (
                &["--server-env=prod"],
                "Expected --server-env=prod to be one of: production, staging, dev, local".into(),
                2,
            ),
        ];
        for (argv, error, code) in cases {
            assert_eq!(run(argv), Err((error, code)), "{argv:?}");
        }
    }

    #[test]
    fn help_and_version_follow_oclif() {
        let help = help();
        assert!(help.lines().all(|l| l.len() <= 100), "oclif wraps the help at 100 columns:\n{help}");
        assert!(help.contains("\n      --port=<value>        Desktop API port"), "{help}");
        assert!(help.contains("\n  -h, --help                Show this help"), "{help}");
        assert!(version().starts_with(&format!("beeper-rs/{APP_VERSION} ")));
    }

    #[test]
    fn bind_free_takes_the_next_free_port_then_a_random_port() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap();
        let _busy = std::net::TcpListener::bind(("127.0.0.1", 23473)).expect("the test port 23473 is free");
        let port = |ports| bind_free(&rt, "127.0.0.1", ports).unwrap().local_addr().unwrap().port();
        assert_eq!(port(23473..=23474), 23474);
        assert!(![0, 23473].contains(&port(23473..=23473)), "a busy port gives a random port");
    }
}
