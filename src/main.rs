use beeper_rs::{
    client::{extract_state_from_index_db, BeeperClient},
    config::{CentralConfig, APP_BUNDLE_ID, APP_NAME, DEFAULT_PAS_HOST, DEFAULT_PAS_PORT},
    platform::current_desktop_data_dir,
    server::{build_router, SharedStore},
    store::BeeperStore,
    ui::BeeperGuiApp,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Parser)]
#[command(name = "beeper-rs", version = "4.3.176")]
struct Cli {
    #[arg(long, env = "BEEPER_PAS_PORT", default_value_t = DEFAULT_PAS_PORT)]
    pas_port: u16,

    #[arg(long)]
    profile: Option<String>,

    #[arg(long)]
    data_dir: Option<PathBuf>,

    #[arg(long)]
    headless: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Serve,
    Info {
        #[arg(long, default_value = "http://127.0.0.1:23373")]
        base_url: String,
    },
    AuthExtract,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let data_dir = cli
        .data_dir
        .unwrap_or_else(|| current_desktop_data_dir(cli.profile.as_deref()));
    let cfg = CentralConfig::new(
        data_dir.clone(),
        DEFAULT_PAS_HOST.to_string(),
        cli.pas_port,
        cli.profile.clone(),
    );

    match cli.command {
        Some(Commands::Info { base_url }) => {
            let info = BeeperClient::new(base_url, "").info().await;
            println!("{}", serde_json::to_string_pretty(&info).unwrap());
            return;
        }
        Some(Commands::AuthExtract) => {
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
        Some(Commands::Serve) | None => {}
    }

    let bind_addr = format!("{}:{}", cfg.host, cfg.port);
    let store: SharedStore = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    let app_router = build_router(store.clone());

    if cli.headless || matches!(cli.command, Some(Commands::Serve)) {
        let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
        axum::serve(listener, app_router).await.unwrap();
    } else {
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
            axum::serve(listener, app_router).await.unwrap();
        });
        let native_options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title(APP_NAME)
                .with_inner_size([1240.0, 820.0])
                .with_min_inner_size([720.0, 480.0]),
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
