use beeper_rs::{
    client::{extract_state_from_index_db, BeeperClient},
    config::{
        CentralConfig, APP_BUNDLE_ID, APP_NAME, APP_VERSION, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT,
    },
    platform::{build_tray_icon_rgba, current_desktop_data_dir, current_original_beeper_data_dir},
    server::{build_router, serve_http, SharedStore},
    store::BeeperStore,
    ui::BeeperGuiApp,
};
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
    let mut host =
        std::env::var("BEEPER_PAS_HOST").unwrap_or_else(|_| DEFAULT_PAS_HOST.to_string());
    let mut port = std::env::var("BEEPER_PAS_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PAS_PORT);
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
                port = args
                    .next()
                    .expect("missing value for --port")
                    .parse::<u16>()
                    .expect("invalid port number")
            }
            "--profile" => profile = Some(args.next().expect("missing value for --profile")),
            "--data-dir" => {
                data_dir = Some(PathBuf::from(
                    args.next().expect("missing value for --data-dir"),
                ))
            }
            "--import-from" => {
                import_from = Some(PathBuf::from(
                    args.next().expect("missing value for --import-from"),
                ))
            }
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

    CliArgs {
        host,
        port,
        profile,
        data_dir,
        import_from,
        headless,
        base_url,
        command,
    }
}

#[tokio::main]
async fn main() {
    let cli = parse_args();
    let data_dir = cli
        .data_dir
        .unwrap_or_else(|| current_desktop_data_dir(cli.profile.as_deref()));
    let source_beeper_dir = cli
        .import_from
        .or_else(|| Some(current_original_beeper_data_dir(cli.profile.as_deref())));
    let cfg = CentralConfig::new(data_dir.clone(), cli.host, cli.port, cli.profile)
        .with_source_beeper_dir(source_beeper_dir);

    match cli.command {
        Command::Info => {
            let info = BeeperClient::new(cli.base_url, "").info().await;
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
        let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
        serve_http(listener, app_router).await;
    } else {
        let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(l) => l,
            Err(_) => {
                let fallback = tokio::net::TcpListener::bind(format!("{host}:0"))
                    .await
                    .expect("bind local ephemeral port");
                let actual_port = fallback.local_addr().unwrap().port();
                let mut st = store.write().unwrap();
                st.config.port = actual_port;
                st.settings.pas_port = actual_port;
                fallback
            }
        };
        tokio::spawn(async move {
            serve_http(listener, app_router).await;
        });
        let (icon_rgba, icon_w, icon_h) = {
            let st = store.read().unwrap();
            build_tray_icon_rgba(st.total_unread_chats(), false)
        };
        let native_options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title(APP_NAME)
                .with_inner_size([1240.0, 820.0])
                .with_min_inner_size([720.0, 480.0])
                .with_icon(eframe::egui::IconData {
                    rgba: icon_rgba,
                    width: icon_w,
                    height: icon_h,
                }),
            ..Default::default()
        };
        eframe::run_native(
            APP_BUNDLE_ID,
            native_options,
            Box::new(move |_cc| Ok(Box::new(BeeperGuiApp::new(store, true)))),
        )
        .unwrap();
    }
}
