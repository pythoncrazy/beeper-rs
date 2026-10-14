use beeper_api::{
    client::{extract_state_from_index_db, BeeperClient},
    http::{read_message, StatusCode},
    server::serve_http,
};
use beeper_core::{
    config::{is_original_beeper_dir, CentralConfig, APP_BUNDLE_ID, DEFAULT_RECOVERY_CODE},
    models::*,
    platform::{
        build_tray_icon_rgba, file_url_to_path, generate_os_autostart_manifest, open_url_or_path_command,
        path_to_file_url, resolve_desktop_data_dir, resolve_original_beeper_data_dir, DisplayServerKind, OsKind,
        SystemTrayState,
    },
    store::{BeeperStore, SharedStore, NOTE_CHAT_ID},
    theme::{parse_css_px, ResolvedTheme},
};
use beeper_ui::{
    login::{LoginMsg, Outcome, Screen},
    search_settings_catalog,
    style::{fw, icon, ALL_FONTS},
    write_rgba_png, BeeperGuiApp, InboxTab, SettingsTab, UiMessage,
};
use iced::font::Weight;
use iced::{Size, Theme};
use iced_test::{emulator, simulator, Emulator, Instruction};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("remove stale test dir");
    }
    dir
}

async fn wait_for(path: &std::path::Path) {
    for _ in 0..50 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{} did not appear", path.display());
}

async fn fake_beeper_cloud() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                while let Some(m) = read_message(&mut stream, &mut buf).await {
                    let path = m.start_line.split(' ').nth(1).unwrap().to_string();
                    let b: Value = serde_json::from_slice(&m.body).unwrap_or(Value::Null);
                    let auth = m.header("authorization").unwrap_or("");
                    let guest = auth == "Bearer BEEPER-PRIVATE-API-PLEASE-DONT-USE";
                    let (code, reply) = match (path.as_str(), b["response"].as_str()) {
                        ("/user/login", _) if guest => (201, json!({ "request": "req_1", "type": ["email"] })),
                        ("/user/login/email", _) if guest && b["request"] == "req_1" && b["supportsOTP"] == true => {
                            (200, json!({}))
                        }
                        ("/user/login/response", Some("123456")) if guest => (200, json!({ "token": "jwt_alex" })),
                        ("/user/login/response", Some("654321")) if guest => {
                            (200, json!({ "leadToken": "lead_1", "usernameSuggestions": ["newbie"] }))
                        }
                        ("/user/login/response", _) => (401, json!({ "error": "Invalid code" })),
                        ("/user/register", _)
                            if guest && b["leadToken"] == "lead_1" && b["userLoginRequestId"] == "req_1" =>
                        {
                            (200, json!({ "token": format!("jwt_{}", b["username"].as_str().unwrap()) }))
                        }
                        ("/_matrix/client/v3/login", _) if b["type"] == "org.matrix.login.jwt" => {
                            let user = b["token"].as_str().unwrap().trim_start_matches("jwt_");
                            let id = format!("@{user}:beeper.com");
                            (200, json!({ "user_id": id, "access_token": format!("syt_{user}"), "device_id": "RSDEV" }))
                        }
                        ("/whoami", _) if auth.starts_with("Bearer syt_") => {
                            let user = auth.trim_start_matches("Bearer syt_");
                            let info = json!({ "fullName": format!("{user} Real"), "username": user, "email": format!("{user}@example.com") });
                            (200, json!({ "userInfo": info }))
                        }
                        _ => (404, json!({ "error": format!("unexpected {path}") })),
                    };
                    let body = reply.to_string();
                    let head = format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(format!("{head}{body}").as_bytes()).await.unwrap();
                }
            });
        }
    });
    url
}

async fn spawn_e2e_beeper_instance(
    profile_suffix: &str,
    cloud: Option<&str>,
) -> (PathBuf, u16, SharedStore, BeeperClient) {
    let data_dir = temp_dir(&format!("BeeperRs_e2e_{profile_suffix}"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = CentralConfig::new(data_dir.clone(), "127.0.0.1".to_string(), port, None);
    cfg.demo = cloud.is_none();
    if let Some(url) = cloud {
        (cfg.api_url, cfg.homeserver) = (url.to_string(), url.to_string());
    }

    let store: SharedStore = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    let token = store.read().unwrap().beeper_state.access_token.clone();
    tokio::spawn(serve_http(listener, store.clone()));

    let client = BeeperClient::new(format!("http://127.0.0.1:{port}"), token);
    (data_dir, port, store, client)
}

#[tokio::test]
async fn e2e_full_beeper_lifecycle_api_ws_auth_messages_search_themes_os_and_gui() {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");

    let fs = iced_test::renderer::graphics::text::font_system();
    {
        let mut w = fs.write().expect("font system");
        for bytes in ALL_FONTS {
            w.load_font(bytes.into());
        }
        let raw = w.raw();
        let db = raw.db();
        let face = db.faces().find(|f| f.families.iter().any(|(n, _)| n == "lucide")).expect("lucide font face");
        let font = raw.get_font(face.id, face.weight).expect("lucide font");
        let cmap = font.as_swash().charmap();
        for ch in [
            icon::INBOX,
            icon::LIST_RESTART,
            icon::SEND_HORIZONTAL,
            icon::MESSAGE_CIRCLE_PLUS,
            icon::CHEVRONS_UP_DOWN,
            icon::SQUARE_PEN,
            icon::AUDIO_LINES,
            icon::PAINTBRUSH,
            icon::PUZZLE,
            icon::X,
        ] {
            assert_ne!(cmap.map(ch as u32), 0, "missing lucide glyph for {ch:?}");
        }
    }

    let mut env_vars = HashMap::new();
    let home = PathBuf::from("/Users/alex");
    assert_eq!(
        resolve_desktop_data_dir(OsKind::MacOs, None, &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/BeeperRs")
    );
    assert_eq!(
        resolve_desktop_data_dir(OsKind::MacOs, Some("dev"), &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/BeeperRs-dev")
    );
    assert_eq!(
        resolve_original_beeper_data_dir(OsKind::MacOs, None, &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts")
    );
    assert!(is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts")));
    assert!(is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts-dev")));
    assert!(!is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/BeeperRs")));
    let blocked_orig = std::panic::catch_unwind(|| {
        CentralConfig::new(
            PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts"),
            "127.0.0.1".to_string(),
            23373,
            None,
        )
    });
    assert!(blocked_orig.is_err(), "CentralConfig::new must refuse original BeeperTexts path");

    env_vars.insert("APPDATA".to_string(), "C:\\Users\\alex\\AppData\\Roaming".to_string());
    assert_eq!(
        resolve_desktop_data_dir(OsKind::Windows, None, &env_vars, &home),
        PathBuf::from("C:\\Users\\alex\\AppData\\Roaming/BeeperRs")
    );
    assert_eq!(
        resolve_original_beeper_data_dir(OsKind::Windows, None, &env_vars, &home),
        PathBuf::from("C:\\Users\\alex\\AppData\\Roaming/BeeperTexts")
    );
    env_vars.insert("XDG_CONFIG_HOME".to_string(), "/home/alex/.config".to_string());
    assert_eq!(
        resolve_desktop_data_dir(OsKind::Linux, Some("nightly"), &env_vars, &home),
        PathBuf::from("/home/alex/.config/BeeperRs-nightly")
    );
    assert_eq!(
        resolve_original_beeper_data_dir(OsKind::Linux, Some("nightly"), &env_vars, &home),
        PathBuf::from("/home/alex/.config/BeeperTexts-nightly")
    );
    assert_eq!(OsKind::MacOs.detect_display_server(&env_vars), DisplayServerKind::Quartz);
    assert_eq!(OsKind::Windows.detect_display_server(&env_vars), DisplayServerKind::Win32);
    env_vars.insert("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string());
    assert_eq!(OsKind::Linux.detect_display_server(&env_vars), DisplayServerKind::Wayland);
    env_vars.remove("WAYLAND_DISPLAY");
    env_vars.insert("DISPLAY".to_string(), ":0".to_string());
    assert_eq!(OsKind::Linux.detect_display_server(&env_vars), DisplayServerKind::X11);
    let unix_url = path_to_file_url(&PathBuf::from("/home/alex/.config/BeeperRs/media/a.bin"));
    assert_eq!(unix_url, "file:///home/alex/.config/BeeperRs/media/a.bin");
    assert_eq!(file_url_to_path(&unix_url).unwrap(), PathBuf::from("/home/alex/.config/BeeperRs/media/a.bin"));
    let win_url = path_to_file_url(&PathBuf::from("C:\\Users\\alex\\AppData\\Roaming\\BeeperRs\\media\\a.bin"));
    assert_eq!(win_url, "file:///C:/Users/alex/AppData/Roaming/BeeperRs/media/a.bin");
    assert_eq!(
        file_url_to_path(&win_url).unwrap(),
        PathBuf::from("C:/Users/alex/AppData/Roaming/BeeperRs/media/a.bin")
    );
    let (win_cmd, win_args) = open_url_or_path_command(OsKind::Windows, "beeper://chat/123");
    assert_eq!(win_cmd, "cmd");
    assert_eq!(win_args[1], "start");

    let (orig_dir, isolated_copy_dir) = (temp_dir("BeeperTexts-orig"), temp_dir("BeeperRs-isolated"));
    std::fs::create_dir_all(orig_dir.join("media")).unwrap();
    std::fs::create_dir_all(orig_dir.join("GPUCache")).unwrap();
    std::fs::write(orig_dir.join("SingletonLock"), b"locked_by_electron").unwrap();
    std::fs::write(orig_dir.join("GPUCache").join("data_0"), b"gpu_cache_blob").unwrap();
    std::fs::write(orig_dir.join("media").join("orig_sample.bin"), b"ORIG_MEDIA").unwrap();
    {
        let orig_conn = Connection::open(orig_dir.join("index.db")).unwrap();
        orig_conn
            .execute_batch(
                "CREATE TABLE key_values ('key' VARCHAR NOT NULL, 'value' JSON NOT NULL, PRIMARY KEY(key));
                 CREATE TABLE messages (event_id TEXT PRIMARY KEY, room_id TEXT NOT NULL, body TEXT);",
            )
            .unwrap();
        let real_beeper_state_json = r#"{"access_token":"bp_tok_from_original_beeper","device_id":"ORIG_DEV_01","user_id":"@imported_from_original:beeper.com","homeserver":"https://matrix.beeper.com","first_sync_done":true,"initialized":true,"secret_storage":true,"cross_signing":true,"verified":true,"key_backup":true,"has_backed_up_code":true,"secrets":{"master_key":"mk_orig","self_signing_key":"ssk_orig","user_signing_key":"usk_orig","megolm_backup_key":"mbk_orig","recovery_code":"EsTc orig"}}"#;
        let real_bridges_json = r#"[{"bridgeID":"whatsapp","accountID":"wa_1","displayName":"WhatsApp","bridge":{"provider":"cloud","type":"whatsapp","state":"RUNNING"},"network":"WhatsApp","profile":{},"state":{"state":"CONNECTED"}},{"bridgeID":"local-signal","accountID":"sig_1","displayName":"Signal","bridge":{"provider":"local","type":"signal","state":"RUNNING"},"network":"Signal","profile":{},"state":{"state":"CONNECTED"}}]"#;
        orig_conn
            .execute(
                "INSERT INTO key_values (key, value) VALUES (?1, ?2)",
                params!["beeperState", real_beeper_state_json],
            )
            .unwrap();
        orig_conn
            .execute(
                "INSERT INTO key_values (key, value) VALUES (?1, ?2)",
                params!["bridgeAccounts", real_bridges_json],
            )
            .unwrap();
    }
    let orig_db_snapshot = std::fs::read(orig_dir.join("index.db")).unwrap();

    let (ext_orig_state, ext_orig_bridges) = extract_state_from_index_db(&orig_dir);
    assert_eq!(ext_orig_state["user_id"], "@imported_from_original:beeper.com");
    assert_eq!(ext_orig_bridges[1]["bridgeID"], "local-signal");

    let isolated_cfg = CentralConfig::new(isolated_copy_dir.clone(), "127.0.0.1".to_string(), 23379, None)
        .with_source_beeper_dir(Some(orig_dir.clone()));
    let mut isolated_store = BeeperStore::from_config(isolated_cfg);
    assert_eq!(isolated_store.copied_files_from_source, 2);
    assert!(!isolated_copy_dir.join("SingletonLock").exists());
    assert!(!isolated_copy_dir.join("GPUCache").exists());
    assert!(isolated_copy_dir.join("media").join("orig_sample.bin").exists());
    assert_eq!(isolated_store.beeper_state.user_id, "@imported_from_original:beeper.com");
    assert_eq!(isolated_store.beeper_state.access_token, "bp_tok_from_original_beeper");
    assert!(isolated_store.valid_tokens.contains("bp_tok_from_original_beeper"));
    assert!(isolated_store.bridge_accounts.is_empty(), "Beeper bridgeAccounts are not imported");
    assert_eq!(isolated_store.chats.len(), 1, "an imported account starts with only Note to self");
    let (_, sent_id, _) =
        isolated_store.send_message(NOTE_CHAT_ID, Some("Mutating isolated copy only".to_string()), None, None).unwrap();
    let orig_db_after = std::fs::read(orig_dir.join("index.db")).unwrap();
    assert_eq!(orig_db_snapshot, orig_db_after, "original BeeperTexts/index.db must never be modified");
    isolated_store.delete_message(NOTE_CHAT_ID, &sent_id).unwrap();
    drop(isolated_store);
    let mut reopened =
        BeeperStore::from_config(CentralConfig::new(isolated_copy_dir.clone(), "127.0.0.1".into(), 23379, None));
    let (_, after_restart_id, _) =
        reopened.send_message(NOTE_CHAT_ID, Some("After restart".to_string()), None, None).unwrap();
    assert_ne!(sent_id, after_restart_id, "message IDs stay unique across restarts");
    let eric_keys: Vec<&str> =
        reopened.chat_msg_indices[NOTE_CHAT_ID].iter().map(|&i| reopened.messages[i].sort_key.as_str()).collect();
    assert!(eric_keys.windows(2).all(|w| w[0] < w[1]), "new messages sort after stored ones: {eric_keys:?}");
    drop(reopened);
    std::fs::remove_dir_all(&orig_dir).unwrap();
    std::fs::remove_dir_all(&isolated_copy_dir).unwrap();

    let mac_manifest = generate_os_autostart_manifest(
        OsKind::MacOs,
        &home,
        &PathBuf::from("/Applications/Beeper.app/Contents/MacOS/beeper-rs"),
    );
    assert!(mac_manifest.content.contains(APP_BUNDLE_ID));
    let linux_manifest = generate_os_autostart_manifest(
        OsKind::Linux,
        &PathBuf::from("/home/alex"),
        &PathBuf::from("/usr/local/bin/beeper-rs"),
    );
    assert!(linux_manifest.desktop_entry_content.as_ref().unwrap().contains("x-scheme-handler/beeper"));
    let win_manifest = generate_os_autostart_manifest(
        OsKind::Windows,
        &home,
        &PathBuf::from("C:\\Program Files\\Beeper\\beeper-rs.exe"),
    );
    assert!(win_manifest.content.contains("beeper-rs.exe"));

    let (data_dir, port, store, mut client) = spawn_e2e_beeper_instance("master", None).await;
    assert_eq!(store.read().unwrap().beeper_state.user_id, "@alex:beeper.com");
    assert_eq!(store.read().unwrap().bridge_accounts.len(), 6);

    let info = client.info().await;
    assert_eq!(info.app.bundle_id, APP_BUNDLE_ID);
    assert_eq!(info.server.port, port);
    assert_eq!(info.endpoints.ws_events, "/v1/ws");

    let oauth_token = client.oauth_pkce_login("Beeper Raycast E2E", "super_secret_pkce_verifier_2026_beeper_rs").await;
    assert!(oauth_token.starts_with("bp_oauth_tok_"));

    assert_eq!(client.setup_step("start", json!({})).await.0, StatusCode::CONFLICT, "demo account is signed in");
    let cloud = fake_beeper_cloud().await;
    for (name, code, user) in [("login", "123456", "alex"), ("register", "654321", "newbie")] {
        let (dir, _, st, c) = spawn_e2e_beeper_instance(name, Some(&cloud)).await;
        assert!(!st.read().unwrap().signed_in() && st.read().unwrap().chats.is_empty(), "fresh install is signed out");
        let (status, start) = c.setup_step("start", json!({})).await;
        assert_eq!((status, &start["signInMethods"]), (StatusCode::OK, &json!(["email"])));
        let id = start["setupRequestID"].clone();
        let email = json!({ "setupRequestID": id, "email": format!("{user}@example.com") });
        assert_eq!(c.setup_step("email", email).await.0, StatusCode::NO_CONTENT);
        let (bad, err) = c.setup_step("response", json!({ "setupRequestID": id, "response": "000000" })).await;
        assert_eq!((bad, err["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_sign_in_code")));
        let (_, mut done) = c.setup_step("response", json!({ "setupRequestID": id, "response": code })).await;
        if name == "register" {
            assert_eq!(
                (done["registrationRequired"].as_bool(), &done["usernameSuggestions"][0]),
                (Some(true), &json!(user))
            );
            let body =
                json!({ "setupRequestID": id, "leadToken": done["leadToken"], "username": user, "acceptTerms": true });
            done = c.setup_step("register", body).await.1;
        }
        assert_eq!(done["matrix"]["userID"], format!("@{user}:beeper.com"));
        assert_eq!(done["matrix"]["accessToken"], format!("syt_{user}"));
        assert_eq!(c.setup_step("start", json!({})).await.0, StatusCode::CONFLICT);
        assert_eq!(st.read().unwrap().me().full_name.as_deref(), Some(format!("{user} Real").as_str()));
        drop((st, c));
        let mut cfg = CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23380, None);
        cfg.api_url.clone_from(&cloud);
        let again = BeeperStore::from_config(cfg);
        assert_eq!(again.beeper_state.access_token, format!("syt_{user}"), "sign-in survives a restart");
        assert!(again.valid_tokens.contains(&format!("syt_{user}")) && again.chats.len() == 1);
        drop(again);
        std::fs::remove_dir_all(dir).unwrap();
    }

    let (login_dir, _, login_store, _) = spawn_e2e_beeper_instance("gui_login", Some(&cloud)).await;
    let mut login = BeeperGuiApp::new(login_store.clone(), false);
    assert!(login.selected_chat_id.is_none());
    login.login_window = Some(iced::window::Id::unique());
    let shows = |app: &BeeperGuiApp, labels: &[&str]| {
        let mut sim = simulator(app.view_login());
        for label in labels {
            assert!(sim.find(*label).is_ok(), "login window shows {label}");
        }
    };
    shows(&login, &["Continue with Google", "Continue with Email"]);
    let _ = login.update(UiMessage::Login(LoginMsg::Show(Screen::Email)));
    let _ = login.update(UiMessage::Login(LoginMsg::SubmitEmail));
    shows(&login, &["Enter your Email", "Next", "Please enter your email."]);
    let _ = login.update(UiMessage::Login(LoginMsg::Email("alex@example.com".into())));
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Requested("req_1".into())))));
    assert_eq!(login.login.screen, Screen::Code);
    shows(&login, &["Enter Code", "Check alex@example.com\nfor your login code", "Open Email"]);
    let _ = login.update(UiMessage::Login(LoginMsg::Code("12 34 56".into())));
    assert_eq!(login.login.code, "123456");
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Err("Invalid code".into()))));
    shows(&login, &["Invalid code"]);
    let gui_cloud = cloud.clone();
    let (session, profile) = tokio::task::spawn_blocking(move || {
        beeper_core::cloud::Cloud::new(&gui_cloud, &gui_cloud).finish("jwt_alex").expect("fake cloud sign-in")
    })
    .await
    .unwrap();
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Session(session, profile)))));
    assert!(login.login_window.is_none() && login.main_window.is_some());
    assert_eq!(login.selected_chat_id.as_deref(), Some(NOTE_CHAT_ID));
    assert!(simulator(login.view_main()).find("Note to self").is_ok());
    drop((login, login_store));
    std::fs::remove_dir_all(login_dir).unwrap();

    let sas_verified = client.verify_device_sas().await;
    assert_eq!(sas_verified["session"]["state"], "ready");
    assert_eq!(sas_verified["session"]["e2ee"]["verified"], true);
    assert_eq!(sas_verified["verification"]["state"], "verified");

    let rk_verified = client.verify_recovery_key(DEFAULT_RECOVERY_CODE).await;
    assert_eq!(rk_verified["session"]["e2ee"]["verified"], true);

    let bridge_sess = client.connect_bridge_session("linkedin", "user_input_login", "on_device").await;
    assert_eq!(bridge_sess.step.step_type, "complete");
    let accounts = client.list_accounts().await;
    assert!(accounts.iter().any(|a| a.account_id == "acc_linkedin"));

    let mut ws_rx = client.connect_ws(vec!["*".to_string()]).await;

    let chats_page = client.list_chats().await;
    assert!(chats_page.items.len() >= 6);

    let (st1, _) = client.patch_chat_draft("!wa_eric:beeper.com", Some("First draft text")).await;
    assert_eq!(st1, StatusCode::OK);
    let (st2, _) = client.patch_chat_draft("!wa_eric:beeper.com", Some("Overwrite attempt without clear")).await;
    assert_eq!(st2, StatusCode::CONFLICT);
    let (st3, cleared_chat) = client.patch_chat_draft("!wa_eric:beeper.com", None).await;
    assert_eq!(st3, StatusCode::OK);
    assert!(cleared_chat.get("draft").is_none() || cleared_chat["draft"].is_null());

    let mut focus_rx = store.read().unwrap().focus_tx.subscribe();
    assert_eq!(client.focus(Some("!wa_eric:beeper.com"), None, Some("Focus draft")).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap().as_deref(), Some("!wa_eric:beeper.com"));
    let eric_idx = store.read().unwrap().chat_idx_by_id["!wa_eric:beeper.com"];
    let focus_draft = store.read().unwrap().chats[eric_idx].draft.clone().unwrap().text;
    assert_eq!(focus_draft.as_deref(), Some("Focus draft"));
    assert_eq!(client.patch_chat_draft("!wa_eric:beeper.com", None).await.0, StatusCode::OK);
    assert_eq!(client.focus(None, None, None).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap(), None);
    assert_eq!(client.focus(Some("!missing:beeper.com"), None, None).await, StatusCode::NOT_FOUND);
    assert_eq!(client.focus(None, Some("msg_missing"), None).await, StatusCode::NOT_FOUND);
    assert_eq!(client.focus(None, None, Some("No chat")).await, StatusCode::BAD_REQUEST);

    let merged_resp = client
        .merge_chats(
            "Unified Eric + Sarah",
            vec!["!wa_eric:beeper.com".to_string(), "!im_sarah:beeper.com".to_string()],
            Some("!im_sarah:beeper.com".to_string()),
        )
        .await;
    let merged_chat_id = merged_resp["chatID"].as_str().unwrap().to_string();

    let send_resp = client.send_message(&merged_chat_id, Some("Routed message via merged chat"), None, None).await;
    assert_eq!(send_resp["chatID"], "!im_sarah:beeper.com");
    let sent_msg_id = send_resp["pendingMessageID"].as_str().unwrap().to_string();
    assert_eq!(client.focus(None, Some(&sent_msg_id), None).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap().as_deref(), Some("!im_sarah:beeper.com"));

    let ws_ev = timeout(Duration::from_secs(2), ws_rx.recv()).await.unwrap().unwrap();
    assert!(!ws_ev.event_type.is_empty());

    let edit_resp =
        client.edit_message("!im_sarah:beeper.com", &sent_msg_id, "Routed message via merged chat (edited v2)").await;
    assert_eq!(edit_resp["success"], true);

    let sarah_msgs = client.list_messages(&merged_chat_id).await;
    let edited_msg = sarah_msgs.items.iter().find(|m| m.id == sent_msg_id).unwrap();
    assert_eq!(edited_msg.text.as_deref(), Some("Routed message via merged chat (edited v2)"));
    assert_eq!(edited_msg.edit_history.len(), 1);
    assert_eq!(edited_msg.edit_history[0].text, "Routed message via merged chat");

    let rx_add = client.add_reaction("!im_sarah:beeper.com", &sent_msg_id, "🔥").await;
    assert_eq!(rx_add["success"], true);
    let rx_del = client.remove_reaction("!im_sarah:beeper.com", &sent_msg_id, "🔥").await;
    assert_eq!(rx_del["success"], true);

    let payload_bytes = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_BEEPER_ASSET";
    let uploaded = client.upload_asset_base64("diagram.png", "image/png", payload_bytes).await;
    let mxc_url = uploaded["srcURL"].as_str().unwrap();
    let (range_status, slice) = client.serve_asset_range(mxc_url, Some("bytes=10-15")).await;
    assert_eq!(range_status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&slice, b"ABCDEF");

    let search_res = client.search_messages("edited", None).await;
    assert!(search_res.items.iter().any(|m| m.id == sent_msg_id));

    let audio_res = client.search_messages("", Some("audio")).await;
    assert!(!audio_res.items.is_empty());

    let unified = client.unified_search("rust").await;
    assert!(!unified["results"]["in_groups"].as_array().unwrap().is_empty());

    let settings_hits = search_settings_catalog("transparency");
    assert_eq!(settings_hits.len(), 2);

    let del_resp = client.delete_message("!im_sarah:beeper.com", &sent_msg_id).await;
    assert_eq!(del_resp["success"], true);
    let search_after_del = client.search_messages("edited", None).await;
    assert!(!search_after_del.items.iter().any(|m| m.id == sent_msg_id));

    let unmerge_chat = store.write().unwrap().patch_chat(&merged_chat_id, false, None, Some(None)).unwrap();
    assert!(unmerge_chat.merge.is_none());

    for choice in [
        ThemeChoice::TextsLight,
        ThemeChoice::TextsDark,
        ThemeChoice::Dracula,
        ThemeChoice::Nocturne,
        ThemeChoice::DarkPurple,
        ThemeChoice::Custom,
    ] {
        let s = BeeperSettings {
            theme: choice,
            no_transparency: true,
            reduce_transparency: true,
            fix_lockdown_emojis: true,
            custom_css_source: ":root { --color-primary: #ff007f; }".to_string(),
            ..Default::default()
        };
        let compiled = ResolvedTheme::compile(&s);
        let px = |name: &str| parse_css_px(&compiled.vars[name], &compiled.vars);
        assert_eq!(px("--header-height"), 48.0);
        assert_eq!(px("--border-radius-conversation-bubble"), 17.0);
        assert_eq!(compiled.brand_primary, [255, 0, 127, 255]);
        assert!(compiled.vars.get("--font-family").unwrap().contains("Twemoji"));
        assert!(compiled.selector_rules.contains_key(".no-transparency"));
    }

    let (rgba_unread, w, h) = build_tray_icon_rgba(5, false);
    assert_eq!((w, h), (32, 32));
    let (rgba_clear, _, _) = build_tray_icon_rgba(0, true);
    assert_ne!(rgba_unread, rgba_clear);

    let mut tray_state = SystemTrayState::new(3, false, false);
    assert!(tray_state.tooltip.contains("3 unread"));
    tray_state.update(0, true);
    assert!(tray_state.tooltip.contains("DND"));

    let mut gui = BeeperGuiApp::new(store.clone(), false);
    assert_eq!(gui.selected_chat_id.as_deref(), Some("!note_self:beeper.com"));
    gui.select_chat("!wa_eric:beeper.com");
    {
        let mut sim = simulator(gui.view_main());
        for label in ["Inbox", "Eric Migicovsky", "Message Eric Migicovsky on WhatsApp", "Note to self", "Feedback?"] {
            assert!(sim.find(label).is_ok(), "main window shows {label}");
        }
        sim.click(iced::widget::Id::new("rail-unread")).expect("unread rail button");
        for m in sim.into_messages() {
            let _ = gui.update(m);
        }
        assert_eq!(gui.active_tab, InboxTab::Unread);
    }

    let _ = gui.update(UiMessage::SelectTab(InboxTab::AllChats));
    let _ = gui.update(UiMessage::ComposerChanged("Hello from Iced Simulator E2E!".to_string()));
    let _ = gui.update(UiMessage::SubmitComposer);
    let _ = gui.update(UiMessage::OpenSettings(SettingsTab::Appearance));
    assert!(gui.settings_window.is_some());
    {
        let mut sim = simulator(gui.view_settings());
        for label in ["Settings", "Search Settings", "Accounts", "Beeper Plus", "Theme", "Custom CSS"] {
            assert!(sim.find(label).is_ok(), "settings window shows {label}");
        }
    }
    let _ = gui.update(UiMessage::SettingsQueryChanged("theme".to_string()));
    assert!(simulator(gui.view_settings()).find("Results").is_ok());
    let _ = gui.update(UiMessage::CloseModals);

    let final_wa_msgs = client.list_messages("!wa_eric:beeper.com").await;
    assert!(final_wa_msgs.items.iter().any(|m| m.text.as_deref() == Some("Hello from Iced Simulator E2E!")));

    let emu_store = store.clone();
    std::thread::spawn(move || {
        let app_prog = iced::daemon(
            move || {
                let (mut app, open) = BeeperGuiApp::boot(emu_store.clone());
                let _ = app.update(UiMessage::SelectTheme(ThemeChoice::TextsDark));
                (app, open)
            },
            BeeperGuiApp::update,
            BeeperGuiApp::view,
        )
        .default_font(fw(Weight::Normal));

        let (tx, _rx) = iced_test::futures::futures::channel::mpsc::channel(64);
        let mut emu = Emulator::new(tx, &app_prog, emulator::Mode::Immediate, Size::new(1264.0, 893.0));
        emu.run(&app_prog, Instruction::parse("expect \"Note to self\"").unwrap());
        let shot = emu.screenshot(&app_prog, &Theme::Dark, 1.0);
        assert_eq!((shot.size.width, shot.size.height), (1264, 893));
        let emu_png = PathBuf::from("/tmp/beeper_rs_iced_dark.png");
        write_rgba_png(&emu_png, shot.size.width, shot.size.height, &shot.rgba);
        assert!(emu_png.exists());
    })
    .join()
    .unwrap();

    let bin_exe = env!("CARGO_BIN_EXE_beeper-rs");
    let (wl_png, wl_settings_png, x_png) =
        ("/tmp/beeper_rs_wayland_window.png", "/tmp/beeper_rs_wayland_settings.png", "/tmp/beeper_rs_xvfb_window.png");
    for png in [wl_png, wl_settings_png, x_png].into_iter().filter(|p| std::path::Path::new(p).exists()) {
        std::fs::remove_file(png).unwrap();
    }
    let run_gui = |port: &str, hide: &str, envs: &[(&str, &str)]| {
        let data = temp_dir(&format!("BeeperRs_gui_{port}"));
        let status = std::process::Command::new(bin_exe)
            .args(["--data-dir", data.to_str().unwrap(), "--port", port])
            .args(envs.iter().any(|e| e.0 == "WAYLAND_DISPLAY").then_some("--demo"))
            .env_remove(hide)
            .env("ICED_BACKEND", "tiny-skia")
            .envs(envs.iter().copied())
            .status()
            .expect("run beeper-rs GUI");
        std::fs::remove_dir_all(&data).expect("remove GUI data dir");
        status
    };
    let spawn_quiet = |cmd: &str, args: &[&str], envs: &[(&str, &str)]| {
        let mut c = std::process::Command::new(cmd);
        c.args(args).envs(envs.iter().copied()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        c.spawn().unwrap_or_else(|e| panic!("spawn {cmd}: {e}"))
    };
    let stop = |mut child: std::process::Child| {
        child.kill().expect("stop display server");
        child.wait().expect("reap display server");
    };

    let wl_runtime = temp_dir("beeper_wl");
    std::fs::create_dir_all(&wl_runtime).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wl_runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let xdg = ("XDG_RUNTIME_DIR", wl_runtime.to_str().unwrap());
    let weston_args = ["--backend=headless", "--socket=beeper-wl-e2e", "--width=1440", "--height=900", "--no-config"];
    let weston = spawn_quiet("weston", &weston_args, &[xdg]);
    wait_for(&wl_runtime.join("beeper-wl-e2e")).await;
    let wl_status = run_gui(
        "23398",
        "DISPLAY",
        &[
            xdg,
            ("WAYLAND_DISPLAY", "beeper-wl-e2e"),
            ("BEEPER_SCREENSHOT_EXIT", wl_png),
            ("BEEPER_SCREENSHOT_SETTINGS", wl_settings_png),
        ],
    );
    stop(weston);
    assert!(wl_status.success(), "beeper-rs GUI on headless Wayland exited cleanly");

    let fb_dir = temp_dir("beeper_xvfb");
    std::fs::create_dir_all(&fb_dir).unwrap();
    let xvfb_args =
        [":99", "-screen", "0", "1440x900x24", "-ac", "-nolisten", "tcp", "-fbdir", fb_dir.to_str().unwrap()];
    let xvfb = spawn_quiet("Xvfb", &xvfb_args, &[]);
    wait_for(&fb_dir.join("Xvfb_screen0")).await;
    let x_status = run_gui("23399", "WAYLAND_DISPLAY", &[("DISPLAY", ":99"), ("BEEPER_SCREENSHOT_EXIT", x_png)]);
    stop(xvfb);
    assert!(x_status.success(), "beeper-rs GUI on Xvfb :99 exited cleanly");

    for (png, min) in [(wl_png, 25_000), (wl_settings_png, 15_000), (x_png, 15_000)] {
        assert!(std::fs::metadata(png).expect("screenshot PNG is written").len() > min, "{png} is too small");
    }
    for dir in [wl_runtime, fb_dir, data_dir] {
        std::fs::remove_dir_all(dir).expect("remove test dir");
    }
}
