use beeper_api::{
    client::{extract_state_from_index_db, BeeperClient},
    server::{build_router, serve_http},
};
use beeper_core::{
    config::{CentralConfig, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
    platform::{current_desktop_data_dir, current_original_beeper_data_dir},
    store::{BeeperStore, SharedStore},
};
use beeper_ui::run_native_gui;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug)]
enum Command {
    Gui,
    Serve,
    Info,
    AuthExtract,
}

struct CliArgs {
    host: String,
    port: u16,
    profile: Option<String>,
    data_dir: Option<PathBuf>,
    import_from: Option<PathBuf>,
    headless: bool,
    base_url: String,
    command: Command,
}

fn parse_args() -> CliArgs {
    let mut host = std::env::var("BEEPER_PAS_HOST").unwrap_or_else(|_| DEFAULT_PAS_HOST.to_string());
    let mut port =
        std::env::var("BEEPER_PAS_PORT").ok().and_then(|v| v.parse::<u16>().ok()).unwrap_or(DEFAULT_PAS_PORT);
    let mut profile: Option<String> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut import_from: Option<PathBuf> = None;
    let mut headless = false;
    let mut base_url = format!("http://{DEFAULT_PAS_HOST}:{DEFAULT_PAS_PORT}");
    let mut command = Command::Gui;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "serve" => command = Command::Serve,
            "info" => command = Command::Info,
            "auth-extract" => command = Command::AuthExtract,
            "--headless" => headless = true,
            "--host" | "--pas-host" => host = args.next().expect("missing value for --host"),
            "--port" | "--pas-port" => {
                port = args.next().expect("missing value for --port").parse::<u16>().expect("invalid port number")
            }
            "--profile" => profile = Some(args.next().expect("missing value for --profile")),
            "--data-dir" => data_dir = Some(PathBuf::from(args.next().expect("missing value for --data-dir"))),
            "--import-from" => import_from = Some(PathBuf::from(args.next().expect("missing value for --import-from"))),
            "--base-url" => base_url = args.next().expect("missing value for --base-url"),
            "--version" | "-V" => {
                println!("{APP_NAME} {APP_VERSION}");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: beeper-rs [OPTIONS] [COMMAND]\n\nCommands:\n  serve\n  info\n  auth-extract\n\nOptions:\n  --host <HOST>\n  --port <PORT>\n  --profile <PROFILE>\n  --data-dir <DIR>\n  --import-from <DIR>\n  --base-url <URL>\n  --headless"
                );
                std::process::exit(0);
            }
            other => panic!("unexpected argument: {other}"),
        }
    }

    CliArgs { host, port, profile, data_dir, import_from, headless, base_url, command }
}

fn main() {
    let cli = parse_args();
    let data_dir = cli.data_dir.unwrap_or_else(|| current_desktop_data_dir(cli.profile.as_deref()));
    let source_beeper_dir = cli
        .import_from
        .or_else(|| Some(current_original_beeper_data_dir(cli.profile.as_deref())).filter(|d| d.exists()));
    let cfg =
        CentralConfig::new(data_dir.clone(), cli.host, cli.port, cli.profile).with_source_beeper_dir(source_beeper_dir);

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build tokio runtime");

    match cli.command {
        Command::Info => {
            let info = rt.block_on(BeeperClient::new(cli.base_url, "").info());
            println!("{}", serde_json::to_string_pretty(&info).unwrap());
            return;
        }
        Command::AuthExtract => {
            let (state, bridges) = extract_state_from_index_db(&data_dir);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "beeperState": state,
                    "bridgeAccounts": bridges
                }))
                .unwrap()
            );
            return;
        }
        Command::Serve | Command::Gui => {}
    }

    let host = cfg.host.clone();
    let bind_addr = format!("{}:{}", cfg.host, cfg.port);
    let store: SharedStore = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    let app_router = build_router(store.clone());

    if cli.headless || matches!(cli.command, Command::Serve) {
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
            serve_http(listener, app_router).await;
        });
    } else {
        let store_for_server = store.clone();
        std::thread::spawn(move || {
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind(&bind_addr)
                    .await
                    .or_else(|_| {
                        std::net::TcpListener::bind(format!("{host}:0")).and_then(|std_l| {
                            std_l.set_nonblocking(true)?;
                            tokio::net::TcpListener::from_std(std_l)
                        })
                    })
                    .expect("bind local PAS port");
                let actual_port = listener.local_addr().unwrap().port();
                {
                    let mut st = store_for_server.write().unwrap();
                    st.config.port = actual_port;
                    st.settings.pas_port = actual_port;
                }
                serve_http(listener, app_router).await;
            });
        });
        drop(run_native_gui(store));
    }
}
