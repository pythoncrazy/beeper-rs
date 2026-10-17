use beeper_api::{
    client::BeeperClient,
    http::{read_message, StatusCode},
    server::serve_http,
};
use beeper_core::{
    config::{is_original_beeper_dir, CentralConfig, APP_BUNDLE_ID, DEFAULT_RECOVERY_CODE},
    import::read_beeper_data,
    matrix::{ChatFlag, Outgoing},
    models::*,
    platform::{
        build_tray_icon_rgba, file_url_to_path, generate_os_autostart_manifest, open_url_or_path_command,
        path_to_file_url, resolve_desktop_data_dir, resolve_original_beeper_data_dir, DisplayServerKind, OsKind,
        SystemTrayState,
    },
    store::{BeeperStore, Setup, SharedStore, NOTE_CHAT_ID},
};
use beeper_ui::{
    login::{Login, LoginMsg, Outcome, Screen},
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
    let dir = std::env::temp_dir().join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("remove stale test dir");
    }
    dir
}

#[cfg(target_os = "linux")]
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
                            let bridges = json!({
                                "whatsapp": { "bridgeState": { "bridgeType": "whatsapp", "stateEvent": "RUNNING" }, "remoteState": {
                                    "wa1": { "state_event": "CONNECTED", "remote_id": "wa1" } } },
                                "slackgo": { "bridgeState": { "bridgeType": "slackgo", "stateEvent": "RUNNING" }, "remoteState": {
                                    "T1-U1": { "state_event": "CONNECTED" }, "T2-U2": { "state_event": "TRANSIENT_DISCONNECT" } } },
                                "discordgo": { "bridgeState": { "bridgeType": "discordgo", "stateEvent": "STOPPED" }, "remoteState": {
                                    "d1": { "state_event": "BAD_CREDENTIALS" } } },
                                "telegram": { "bridgeState": { "bridgeType": "telegram", "stateEvent": "RUNNING" }, "remoteState": {
                                    "t1": { "state_event": "UNCONFIGURED" } } },
                                "signal": { "bridgeState": { "bridgeType": "signal", "stateEvent": "RUNNING" } },
                                "local-whatsapp": { "bridgeState": { "bridgeType": "whatsapp", "stateEvent": "RUNNING" }, "remoteState": {
                                    "lw": { "state_event": "CONNECTED" } } },
                                "sh-gmessages": { "bridgeState": { "bridgeType": "gmessages", "isSelfHosted": true }, "remoteState": {
                                    "g1": { "state_event": "CONNECTED" } } }
                            });
                            (200, json!({ "userInfo": info, "user": { "bridges": bridges, "hungryserv": {} } }))
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
        let face = db.faces().find(|f| f.families.iter().any(|(n, _)| n == "icons")).expect("icons font face");
        let font = raw.get_font(face.id, face.weight).expect("icons font");
        let cmap = font.as_swash().charmap();
        for ch in [
            icon::INBOX,
            icon::LOW_PRIORITY_SPACE,
            icon::SCHEDULE_SEND,
            icon::MAPS_UGC,
            icon::UNFOLD_MORE,
            icon::EDIT_SQUARE,
            icon::DOWNLOAD,
            icon::AUDIO_LINES,
            icon::PAINTBRUSH,
            icon::PUZZLE,
            icon::CLOSE,
        ] {
            assert_ne!(cmap.map(ch as u32), 0, "missing icon glyph for {ch:?}");
        }
    }

    let mut env_vars = HashMap::new();
    let home = PathBuf::from("/Users/alex");
    assert_eq!(
        resolve_desktop_data_dir(OsKind::MacOs, None, &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/beeper-rs")
    );
    assert_eq!(
        resolve_desktop_data_dir(OsKind::MacOs, Some("dev"), &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/beeper-rs-dev")
    );
    assert_eq!(
        resolve_original_beeper_data_dir(OsKind::MacOs, None, &env_vars, &home),
        PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts")
    );
    assert!(is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts")));
    assert!(is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/BeeperTexts-dev")));
    assert!(!is_original_beeper_dir(&PathBuf::from("/Users/alex/Library/Application Support/beeper-rs")));
    let keys: Vec<String> = [-12, -5, 0, 7, 1_000_000].map(beeper_core::store::sort_key).into();
    assert!(keys.is_sorted(), "sort keys keep numeric order: {keys:?}");
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
        PathBuf::from("C:\\Users\\alex\\AppData\\Roaming/beeper-rs")
    );
    assert_eq!(
        resolve_original_beeper_data_dir(OsKind::Windows, None, &env_vars, &home),
        PathBuf::from("C:\\Users\\alex\\AppData\\Roaming/BeeperTexts")
    );
    env_vars.insert("XDG_CONFIG_HOME".to_string(), "/home/alex/.config".to_string());
    assert_eq!(
        resolve_desktop_data_dir(OsKind::Linux, Some("nightly"), &env_vars, &home),
        PathBuf::from("/home/alex/.config/beeper-rs-nightly")
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

    let (orig_dir, rs_dir) = (temp_dir("BeeperTexts-orig"), temp_dir("BeeperRs-import"));
    std::fs::create_dir_all(orig_dir.join("media")).unwrap();
    std::fs::write(orig_dir.join("SingletonLock"), b"locked_by_electron").unwrap();
    {
        let index = Connection::open(orig_dir.join("index.db")).unwrap();
        index
            .execute_batch(
                "CREATE TABLE key_values ('key' VARCHAR NOT NULL, 'value' JSON NOT NULL, PRIMARY KEY(key));
                 CREATE TABLE threads ('threadID' VARCHAR NOT NULL, 'accountID' VARCHAR NOT NULL, 'thread' JSON NOT NULL,
                     'timestamp' INTEGER, is_label INTEGER NOT NULL DEFAULT 0, room_type TEXT, PRIMARY KEY(threadID));
                 CREATE TABLE messages ('messageID' VARCHAR NOT NULL, 'accountID' VARCHAR NOT NULL, 'timestamp' INTEGER NOT NULL,
                     'threadID' VARCHAR NOT NULL, 'message' JSON NOT NULL, PRIMARY KEY(accountID, messageID));
                 CREATE TABLE participants (account_id TEXT NOT NULL, room_id TEXT NOT NULL, id TEXT NOT NULL, full_name TEXT,
                     nickname TEXT, img_url TEXT, is_verified INTEGER, cannot_message INTEGER, is_self INTEGER,
                     is_network_bot INTEGER, added_by TEXT, is_admin INTEGER, is_pending INTEGER, has_exited INTEGER);
                 CREATE TABLE participant_identifiers (account_id TEXT, participant_id TEXT, identifier TEXT, identifier_type TEXT);
                 CREATE TABLE mx_room_messages (eventID TEXT, roomID TEXT, timestamp INTEGER, type TEXT, isDeleted INTEGER,
                     hsOrder INTEGER, message JSON);
                 CREATE TABLE mx_reactions (roomID TEXT, reactionID TEXT, eventID TEXT, senderID TEXT, description TEXT,
                     \"order\" INTEGER, isDeleted INTEGER);
                 CREATE TABLE mx_read_receipts (roomID TEXT, userID TEXT, timestamp INTEGER, relatesToID TEXT, lastVisibleEventID TEXT);
                 INSERT INTO mx_read_receipts VALUES ('!wa:beeper.local', '@wa_ana:beeper.local', 1760000003000, '$m2', '$m2');
                 INSERT INTO mx_room_messages VALUES ('$x1', '!mx:beeper.local', 1760000301000, 'TEXT', 0, 900,
                     '{\"senderID\":\"@emile:beeper.com\",\"text\":\"from matrix\",\"isSender\":true}');
                 INSERT INTO mx_room_messages VALUES ('$x2', '!mx:beeper.local', 1760000302000, 'REACTION', 0, 901, '{}');
                 INSERT INTO mx_reactions VALUES ('!mx:beeper.local', '$x2', '$x1', '@a:beeper.com', '❤️', 1, 0);
                 INSERT INTO key_values VALUES ('beeperState', '{\"user_id\":\"\",\"access_token\":\"\"}');
                 INSERT INTO key_values VALUES ('nextSeq', 1234);
                 INSERT INTO key_values VALUES ('ad:com.beeper.favourites_order', '{\"Inbox\":[\"!dc:beeper.local\"]}');
                 INSERT INTO key_values VALUES ('ad:com.beeper.chat_list_rules', '{\"visibility\":{}}');",
            )
            .unwrap();
        let thread = |id: &str, acc: &str, ts: i64, label: i64, json: Value| {
            index
                .execute(
                    "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![id, acc, json.to_string(), ts, label],
                )
                .unwrap();
        };
        thread(
            "!wa:beeper.local",
            "local-whatsapp_ba_Xq9",
            1_760_000_000_000,
            0,
            json!({"type": "single", "unreadCount": 2}),
        );
        thread(
            "!dc:beeper.local",
            "discordgo",
            1_760_000_100_000,
            0,
            json!({"title": "Rust Café ☕", "type": "group", "isPinned": true, "mutedUntil": "forever"}),
        );
        thread(
            "!sl:beeper.local",
            "slackgo.T0123",
            1_760_000_200_000,
            0,
            json!({"type": "group", "isArchived": true, "extra": {"bridge": {"com.beeper.bridge_name": "slackgo"}}}),
        );
        thread(
            "!mx:beeper.local",
            "hungryserv",
            1_760_000_300_000,
            0,
            json!({
                "title": "",
                "type": "single",
                "isLowPriority": 1,
                "partialLastMessage": {
                    "id": "$x2",
                    "senderID": "@a:beeper.com",
                    "sortKey": 901,
                    "timestamp": "2025-10-09T08:58:22.000Z",
                    "action": {"reactionKey": "❤️"},
                    "linkedMessage": {"text": "from matrix", "isSender": true}
                }
            }),
        );
        thread(
            "!zz:beeper.local",
            "brandnewgo",
            1_760_000_400_000,
            0,
            json!({"title": "Unknown Network", "type": "single"}),
        );
        thread("label_inbox", "hungryserv", 1_760_000_500_000, 1, json!({"title": "Label"}));
        let person = |room: &str, id: &str, name: Option<&str>, nick: Option<&str>, me: i64, exited: i64| {
            index
                .execute(
                    "INSERT INTO participants (account_id, room_id, id, full_name, nickname, is_self, cannot_message, has_exited)
                     VALUES ('x', ?1, ?2, ?3, ?4, ?5, 0, ?6)",
                    params![room, id, name, nick, me, exited],
                )
                .unwrap();
        };
        person("!wa:beeper.local", "@emile:beeper.com", Some("Émile Zola"), None, 1, 0);
        person("!wa:beeper.local", "@wa_ana:beeper.local", Some("Ana Núñez"), None, 0, 0);
        person("!dc:beeper.local", "@dc_bo:beeper.local", None, Some("Bo"), 0, 0);
        person("!dc:beeper.local", "@dc_cy:beeper.local", Some("Cy"), None, 0, 0);
        person("!dc:beeper.local", "@dc_gone:beeper.local", Some("Gone"), None, 0, 1);
        person("!sl:beeper.local", "@sl_x:beeper.local", None, None, 0, 0);
        let message = |id: &str, room: &str, ts: i64, json: Value| {
            index
                .execute("INSERT INTO messages VALUES (?1, 'acc', ?2, ?3, ?4)", params![id, ts, room, json.to_string()])
                .unwrap();
        };
        message(
            "$m1",
            "!wa:beeper.local",
            1_760_000_001_000,
            json!({"senderID": "@wa_ana:beeper.local", "text": "¡Hola! https://example.com/a"}),
        );
        message(
            "$m2",
            "!wa:beeper.local",
            1_760_000_002_000,
            json!({"senderID": "@emile:beeper.com", "isSender": true, "text": "Salut", "linkedMessageID": "$missing"}),
        );
        message(
            "$m3",
            "!dc:beeper.local",
            1_760_000_101_000,
            json!({"senderID": "@dc_bo:beeper.local", "attachments": [{"type": "audio", "isVoiceNote": true}]}),
        );
        message(
            "$m4",
            "!dc:beeper.local",
            1_760_000_102_000,
            json!({"senderID": "@dc_stranger:beeper.local", "text": "who am I", "linkedMessageID": "$m3", "reactions": [{"id": "r1", "participantID": "@dc_cy:beeper.local", "reactionKey": "👍"}]}),
        );
        message(
            "$m5",
            "!sl:beeper.local",
            1_760_000_201_000,
            json!({
                "senderID": "@sl_x:beeper.local",
                "extra": {"unresolvedMedia": "{\"kind\":\"permanent\"}"},
                "attachments": [
                    {"type": "unknown"},
                    {"type": "img", "srcURL": "mxc://beeper.local/reel_thumb", "size": {"width": 64, "height": 64}},
                    {"type": "video", "srcURL": "mxc://beeper.local/reel_vid", "posterImg": "mxc://beeper.local/reel_poster", "extra": {"duration": 15000}}
                ]
            }),
        );
        message(
            "$m6",
            "!sl:beeper.local",
            1_760_000_202_000,
            json!({"senderID": "@sl_x:beeper.local", "text": "hidden", "isHidden": true}),
        );
        message(
            "$m7",
            "label_inbox",
            1_760_000_501_000,
            json!({"senderID": "@x:beeper.local", "text": "label message"}),
        );
    }
    let originals = |d: &PathBuf| std::fs::read(d.join("index.db")).unwrap();
    let before = originals(&orig_dir);
    assert!(read_beeper_data(&rs_dir).is_err(), "a dir without Beeper Desktop data has nothing to import");

    let fresh = BeeperStore::from_config(CentralConfig::new(rs_dir.clone(), "127.0.0.1".into(), 23379, None));
    assert!(fresh.setup.is_none() && !fresh.signed_in() && fresh.chats.is_empty(), "a new data dir starts empty");
    assert!(!rs_dir.join("index.db").exists() && !rs_dir.join("account.db").exists(), "no Beeper files are copied");
    let shared: SharedStore = Arc::new(RwLock::new(fresh));
    let mut gui = BeeperGuiApp::new(shared.clone(), false);
    gui.login_window = Some(iced::window::Id::unique());
    gui.login = Login::new(Some(orig_dir.clone()));
    {
        let mut sim = simulator(gui.view_login());
        assert!(sim.find("Copy from Beeper Desktop").is_ok() && sim.find("Start Fresh").is_ok());
    }
    let data = Arc::new(read_beeper_data(&orig_dir).expect("read the Beeper Desktop fixture"));
    assert!(data.crypto.is_none(), "a Desktop without an account has no keys");
    let _ = gui.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Read(data.clone())))));
    assert_eq!(gui.login.screen, Screen::Welcome, "the import asks to sign in as a new device");
    assert!(shared.read().unwrap().setup.is_none(), "nothing is imported before the sign-in");
    let session = MatrixSession {
        user_id: "@emile:beeper.com".into(),
        access_token: "syt_emile".into(),
        device_id: "NEWDEVICE".into(),
    };
    let profile = Profile {
        full_name: Some("Émile Zola".into()),
        username: Some("emile".into()),
        email: Some("emile@example.com".into()),
    };
    let _ = gui.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Imported(data, session, profile, Vec::new())))));
    assert_eq!(before, originals(&orig_dir), "Beeper Desktop files must never be modified");
    assert_eq!(gui.login.screen, Screen::Accounts, "an imported session shows the chat accounts");
    let _ = gui.update(UiMessage::CloudAccounts(Err("offline".into())));
    assert_eq!(gui.status_banner, "Could not load your chat accounts: offline");
    let bridges = json!({ "signal": { "bridgeState": { "bridgeType": "signal" }, "remoteState": { "s1": { "state_event": "CONNECTED",
        "remote_profile": { "username": "alex", "phone": "+1 415-555-0142", "avatar": "mxc://beeper.com/a" } } } } });
    let cloud = beeper_core::cloud::cloud_accounts(&bridges).expect("parse the cloud bridges");
    assert_eq!(cloud[0].display_text().as_deref(), Some("+1 415-555-0142"), "the phone comes before the username");
    let _ = gui.update(UiMessage::CloudAccounts(Ok(cloud)));
    assert_eq!(shared.read().unwrap().accounts.iter().map(|a| a.account_id.as_str()).collect::<Vec<_>>(), ["signal"]);
    for bad in [
        json!([]),
        json!({ "whatsapp": { "bridgeState": {}, "remoteState": { "w": { "state_event": "CONNECTED" } } } }),
        json!({ "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "a": { "state_event": "CONNECTED" }, "b": { "state_event": "CONNECTED" } } } }),
    ] {
        assert_eq!(beeper_core::cloud::cloud_accounts(&bad).unwrap_err().status, 502, "{bad} is rejected");
    }
    let _ = gui.update(UiMessage::Login(LoginMsg::Continue));
    assert!(gui.login_window.is_none() && gui.main_window.is_some(), "Continue opens the app");
    let chats = {
        let st = shared.read().unwrap();
        assert_eq!(st.setup, Some(Setup::Imported));
        assert_eq!(st.beeper_state.user_id, "@emile:beeper.com");
        assert!(st.valid_tokens.contains("syt_emile"));
        assert_eq!(st.me().email.as_deref(), Some("emile@example.com"));
        assert_eq!(st.me().full_name.as_deref(), Some("Émile Zola"));
        let title = |id: &str| st.chats[st.chat_idx_by_id[id]].title.as_str();
        assert_eq!(title("!wa:beeper.local"), "Ana Núñez", "a single chat without a title uses the other person");
        assert_eq!(title("!sl:beeper.local"), "@sl_x:beeper.local");
        assert_eq!(title("!mx:beeper.local"), "Unnamed chat");
        assert!(!st.chat_idx_by_id.contains_key("label_inbox"), "label threads are not chats");
        let network =
            |id: &str| beeper_core::config::account_network(&st.chats[st.chat_idx_by_id[id]].account_id).bridge_id;
        assert_eq!(
            ["!wa:beeper.local", "!dc:beeper.local", "!sl:beeper.local", "!mx:beeper.local", "!zz:beeper.local"]
                .map(network),
            ["whatsapp", "discord", "slack", "hungryserv", "hungryserv"]
        );
        let dc = &st.chats[st.chat_idx_by_id["!dc:beeper.local"]];
        assert!(dc.is_pinned == Some(true) && dc.is_muted == Some(true) && dc.participants.items.len() == 2);
        assert_eq!(st.pin_rank.get("!dc:beeper.local"), Some(&0));
        assert_eq!(st.chats[st.chat_idx_by_id["!sl:beeper.local"]].is_hidden_from_inbox, Some(true));
        assert_eq!(
            st.chats[st.chat_idx_by_id["!mx:beeper.local"]].preview.as_ref().and_then(|p| p.text.as_deref()),
            Some("loved \"from matrix\"")
        );
        let msg = |id: &str| &st.messages[st.msg_idx_by_id[id]];
        assert_eq!(msg("$m3").sender_name.as_deref(), Some("Bo"));
        assert_eq!(msg("$m3").attachments[0].file_name.as_deref(), Some("Voice message"));
        assert_eq!(msg("$m4").sender_name.as_deref(), Some("@dc_stranger:beeper.local"));
        assert_eq!(msg("$m4").linked_message_id.as_deref(), Some("$m3"));
        assert_eq!(msg("$m2").linked_message_id, None, "links to messages that were not imported are dropped");
        assert_eq!(msg("$m2").seen["@wa_ana:beeper.local"], "2025-10-09T08:53:23.000Z");
        assert_eq!(msg("$m5").unresolved_media.as_deref(), Some("{\"kind\":\"permanent\"}"));
        assert_eq!(msg("$m5").attachments[1].src_url.as_deref(), Some("mxc://beeper.local/reel_thumb"));
        assert_eq!(msg("$m5").attachments[2].poster_img.as_deref(), Some("mxc://beeper.local/reel_poster"));
        assert_eq!(msg("$m5").attachments[2].duration, Some(15.0));
        assert_eq!(msg("$m1").timestamp, "2025-10-09T08:53:21.000Z");
        assert!(!st.msg_idx_by_id.contains_key("$m6") && !st.msg_idx_by_id.contains_key("$m7"));
        assert_eq!(msg("$x1").text.as_deref(), Some("from matrix"));
        assert_eq!(msg("$x1").reactions[0].reaction_key, "❤️");
        assert!(!st.msg_idx_by_id.contains_key("$x2"), "reaction events are not messages");
        st.chats.clone()
    };
    assert_eq!(chats.len(), 6);
    for c in &chats {
        gui.select_chat(&c.id);
        assert!(simulator(gui.view_main()).find(beeper_ui::short_title(c)).is_ok(), "chat {} renders", c.title);
    }
    gui.select_chat("!sl:beeper.local");
    assert!(simulator(gui.view_main()).find("Click to load").is_ok());
    assert!(simulator(gui.view_settings()).find("emile@example.com").is_ok());
    drop((gui, shared));
    let mut reopened = BeeperStore::from_config(CentralConfig::new(rs_dir.clone(), "127.0.0.1".into(), 23379, None));
    assert_eq!(reopened.setup, Some(Setup::Imported));
    assert_eq!(reopened.pin_rank.get("!dc:beeper.local"), Some(&0));
    assert!(reopened.signed_in() && reopened.chats.len() == 6 && reopened.messages.len() == 6);
    let (_, sent_id, _) = reopened.send_message(NOTE_CHAT_ID, Some("After restart".into()), None, None).unwrap();
    let keys: Vec<&str> = reopened.messages.iter().map(|m| m.sort_key.as_str()).collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "new messages sort after imported ones: {keys:?}");
    reopened.delete_message(NOTE_CHAT_ID, &sent_id).unwrap();
    drop(reopened);
    let mut fresh_gui = BeeperGuiApp::new(
        Arc::new(RwLock::new(BeeperStore::from_config(CentralConfig::new(
            temp_dir("BeeperRs-fresh"),
            "127.0.0.1".into(),
            23379,
            None,
        )))),
        false,
    );
    fresh_gui.login = Login::new(Some(orig_dir.clone()));
    let _ = fresh_gui.update(UiMessage::Login(LoginMsg::Fresh));
    assert_eq!(fresh_gui.login.screen, Screen::Welcome);
    assert_eq!(fresh_gui.store.read().unwrap().setup, Some(Setup::Fresh));
    let fresh_dir = fresh_gui.store.read().unwrap().data_dir.clone();
    drop(fresh_gui);
    std::fs::remove_dir_all(&orig_dir).unwrap();
    std::fs::remove_dir_all(&rs_dir).unwrap();
    std::fs::remove_dir_all(&fresh_dir).unwrap();

    let root = temp_dir("legacy-root");
    let (legacy, current) = (root.join("BeeperRs-dev"), root.join("beeper-rs-dev"));
    let asset_url = {
        let mut old = BeeperStore::from_config(CentralConfig::new(legacy.clone(), "127.0.0.1".into(), 23379, None));
        old.finish_setup(Setup::Fresh);
        old.store_uploaded_asset("a b.txt".into(), "text/plain".into(), b"kept").mxc_url
    };
    std::fs::write(legacy.join("index.db"), b"stale copy of Beeper Desktop").unwrap();
    {
        let db = Connection::open(legacy.join("beeper-rs.db")).unwrap();
        let old_path = legacy.join("media").join("x").display().to_string();
        db.execute(
            "UPDATE assets SET json = json_set(json_remove(json, '$.disk_name'), '$.local_file_path',
                 replace(?1, 'x', upload_id || '_a_b.txt'))",
            [old_path],
        )
        .unwrap();
        db.execute_batch("PRAGMA user_version = 1;").unwrap();
    }
    beeper_core::config::migrate_legacy_data_dir(&current, Some("dev"));
    assert!(!legacy.exists() && !current.join("index.db").exists(), "the old folder and the stale copy are gone");
    let moved = BeeperStore::from_config(CentralConfig::new(current.clone(), "127.0.0.1".into(), 23379, None));
    assert_eq!(moved.setup, Some(Setup::Fresh), "the beeper-rs database moves to the new folder");
    let (path, _) = moved.resolve_asset_path(&asset_url).expect("the uploaded asset is still known");
    assert_eq!(std::fs::read(path).unwrap(), b"kept");
    drop(moved);
    beeper_core::config::migrate_legacy_data_dir(&current, Some("dev"));
    std::fs::remove_dir_all(&root).unwrap();

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

    let (_, port, store, mut client) = spawn_e2e_beeper_instance("master", None).await;
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
    }

    let (_, _, login_store, _) = spawn_e2e_beeper_instance("gui_login", Some(&cloud)).await;
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
    let (session, profile, accounts) = tokio::task::spawn_blocking(move || {
        beeper_core::cloud::Cloud::new(&gui_cloud, &gui_cloud).finish("jwt_alex").expect("fake cloud sign-in")
    })
    .await
    .unwrap();
    let mut ids: Vec<(&str, &str)> = accounts.iter().map(|a| (a.account_id.as_str(), a.state.as_str())).collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            ("discordgo", "BAD_CREDENTIALS"),
            ("slackgo.T1-U1", "CONNECTED"),
            ("slackgo.T2-U2", "TRANSIENT_DISCONNECT"),
            ("whatsapp", "CONNECTED")
        ]
    );
    assert!(accounts.iter().all(|a| a.mode == BridgeMode::Cloud && a.network_name != "Beeper"));
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Session(session, profile, accounts)))));
    assert_eq!(login.login.screen, Screen::Accounts);
    assert_eq!(login_store.read().unwrap().accounts.len(), 4);
    let _ = login.update(UiMessage::StoreChanged);
    assert!(login.login_window.is_some(), "the onboarding stays open until Continue");
    shows(
        &login,
        &["Connect Chat Accounts", "WhatsApp", "Discord", "Bluesky", "6 free accounts remaining.", "Continue"],
    );
    let _ = login.update(UiMessage::Login(LoginMsg::Query("sig".into())));
    shows(&login, &["Signal"]);
    assert!(simulator(login.view_login()).find("WhatsApp").is_err(), "search filters the networks");
    let _ = login.update(UiMessage::Login(LoginMsg::Query("zzz".into())));
    shows(&login, &["No networks found"]);
    let _ = login.update(UiMessage::Login(LoginMsg::Continue));
    assert!(login.login_window.is_none() && login.main_window.is_some());
    assert_eq!(login.selected_chat_id.as_deref(), Some(NOTE_CHAT_ID));
    assert!(simulator(login.view_main()).find("Note").is_ok());
    drop((login, login_store));

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

    let settings_hits = search_settings_catalog("notifications");
    assert!(
        settings_hits.len() > 5
            && settings_hits.iter().all(|(page, _)| *page == "Notifications" || page.contains("otif"))
    );
    assert_eq!(search_settings_catalog("emoji autocomplete")[0].1.key, "emojiAutocomplete");

    let del_resp = client.delete_message("!im_sarah:beeper.com", &sent_msg_id).await;
    assert_eq!(del_resp["success"], true);
    let search_after_del = client.search_messages("edited", None).await;
    assert!(!search_after_del.items.iter().any(|m| m.id == sent_msg_id));

    let unmerge_chat = store.write().unwrap().patch_chat(&merged_chat_id, false, None, Some(None)).unwrap();
    assert!(unmerge_chat.merge.is_none());

    let legacy = |name: &str| serde_json::from_value::<ThemeChoice>(json!(name)).unwrap();
    assert_eq!(
        ["TextsLight", "TextsDark", "Dracula", "Nocturne", "DarkPurple", "Custom", "System"].map(legacy),
        [
            ThemeChoice::Light,
            ThemeChoice::Dark,
            ThemeChoice::Dark,
            ThemeChoice::Dark,
            ThemeChoice::Dark,
            ThemeChoice::System,
            ThemeChoice::System
        ]
    );
    assert_eq!(BeeperSettings::default().theme, ThemeChoice::System);
    assert_eq!(
        ThemeChoice::ALL.map(|t| (t.is_dark(false), t.is_dark(true))),
        [(false, false), (true, true), (false, true)]
    );

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
        for label in ["Inbox", "Eric", "Message Eric Migicovsky on WhatsApp", "Feedback?"] {
            assert!(sim.find(label).is_ok(), "main window shows {label}");
        }
        sim.click(iced::widget::Id::new("rail-requests")).expect("requests rail button");
        for m in sim.into_messages() {
            let _ = gui.update(m);
        }
        assert_eq!(gui.active_tab, InboxTab::Requests);
    }

    let _ = gui.update(UiMessage::SelectTab(InboxTab::AllChats));
    let _ = gui.update(UiMessage::FilterButton);
    let _ = gui.update(UiMessage::ToggleFilterMenu);
    {
        let mut sim = simulator(gui.view_main());
        for label in ["Unread", "Ctrl Shift Y", "Low Priority"] {
            assert!(sim.find(label).is_ok(), "filter menu shows {label}");
        }
    }
    let _ = gui.update(UiMessage::PickFilter(1));
    assert!(gui.filters == 0 && !gui.show_filter_menu, "picking the only active filter clears it");
    let _ = gui.update(UiMessage::ToggleCommandPalette);
    {
        let mut sim = simulator(gui.view_main());
        for label in ["Commands", "Accounts", "Message type", "Recent chats"] {
            assert!(sim.find(label).is_ok(), "search modal shows {label}");
        }
    }
    let _ = gui.update(UiMessage::ToggleNewChat);
    {
        let mut sim = simulator(gui.view_main());
        for label in ["New Chat", "All accounts", "New Group", "Join a Matrix room", "In your contacts"] {
            assert!(sim.find(label).is_ok(), "new chat panel shows {label}");
        }
    }
    let _ = gui.update(UiMessage::ToggleNewChat);
    let _ = gui.update(UiMessage::ComposerChanged("Hello from Iced Simulator E2E!".to_string()));
    let _ = gui.update(UiMessage::SubmitComposer);
    let _ = gui.update(UiMessage::OpenSettings(SettingsTab::Appearance));
    assert!(gui.settings_window.is_some());
    {
        let mut sim = simulator(gui.view_settings());
        for label in
            ["Settings", "Search Settings", "Accounts", "Beeper Plus", "Chat List", "Show message previews", "Theme"]
        {
            assert!(sim.find(label).is_ok(), "settings window shows {label}");
        }
        assert!(["Light", "Dark", "System"].iter().all(|t| sim.find(*t).is_ok()), "the theme cards are shown");
    }
    let _ = gui.update(UiMessage::SelectTheme(ThemeChoice::Dark));
    assert!(gui.dark);
    {
        let st = store.read().unwrap();
        assert!(st.settings.on("showMessagePreviews") && st.settings.pref("timeFormat") == 0);
    }
    let _ = gui.update(UiMessage::SetPref("timeFormat", 1));
    let _ = gui.update(UiMessage::SetPref("showMessagePreviews", 0));
    for (step, zoom) in [(1, 1.1), (1, 1.25), (-1, 1.1), (0, 1.0), (-1, 0.9)] {
        let _ = gui.update(UiMessage::Zoom(step));
        assert_eq!(gui.zoom, zoom);
    }
    let _ = gui.update(UiMessage::Zoom(0));
    {
        let st = store.read().unwrap();
        assert!(!st.settings.on("showMessagePreviews") && st.settings.pref("timeFormat") == 1);
        let stored: BeeperSettings = serde_json::from_value(serde_json::to_value(&st.settings).unwrap()).unwrap();
        assert_eq!(stored.prefs, st.settings.prefs, "prefs survive a round trip");
        let mut bad = st.settings.clone();
        for (key, v) in [("timeFormat", 2), ("showSidebar", 2), ("noSuchPref", 0)] {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bad.set_pref(key, v)));
            assert!(r.is_err(), "{key} = {v} is rejected");
        }
    }
    for page in [SettingsTab::General, SettingsTab::Notifications] {
        let _ = gui.update(UiMessage::SelectSettingsTab(page));
        let mut sim = simulator(gui.view_settings());
        for label in
            ["App language", "Time Format", "Share Analytics", "Delay notifications for", "Sound effect volume"]
        {
            let tab = if label.starts_with("Delay") || label.starts_with("Sound") {
                SettingsTab::Notifications
            } else {
                SettingsTab::General
            };
            assert_eq!(sim.find(label).is_ok(), tab == page, "{label} is on {tab:?}");
        }
    }
    let _ = gui.update(UiMessage::SelectSettingsTab(SettingsTab::Accounts));
    {
        let n = beeper_ui::login::FREE_ACCOUNTS - store.read().unwrap().bridge_accounts.len();
        let mut sim = simulator(gui.view_settings());
        for label in ["Beeper", "+1 415-555-0142", "Beeper · alex@beeper.com", &format!("{n} Free Accounts Remaining")]
        {
            assert!(sim.find(label).is_ok(), "the accounts page shows {label}");
        }
        assert!(sim.find("Facebook").is_err(), "the network grid starts hidden");
        let _ = sim.click("Add an Account");
        assert!(matches!(sim.into_messages().last(), Some(UiMessage::ToggleAddAccount)));
    }
    let _ = gui.update(UiMessage::ToggleAddAccount);
    assert!(simulator(gui.view_settings()).find("Facebook").is_ok(), "Add an Account shows the network grid");
    let _ = gui.update(UiMessage::SetPref("timeFormat", 0));
    let _ = gui.update(UiMessage::SetPref("showMessagePreviews", 1));
    let ctrl_k = || UiMessage::KeyPressed(iced::keyboard::Key::Character("k".into()), iced::keyboard::Modifiers::CTRL);
    let _ = gui.update(UiMessage::ToggleHotkey("Search"));
    let _ = gui.update(ctrl_k());
    assert!(!gui.show_command_palette, "a disabled hotkey does nothing");
    let _ = gui.update(UiMessage::RevertHotkeys);
    let _ = gui.update(ctrl_k());
    assert!(gui.show_command_palette, "Revert all enables the hotkey again");
    let _ = gui.update(UiMessage::ToggleCommandPalette);
    let mods = iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::ALT;
    let rail = |d: &str| UiMessage::KeyPressed(iced::keyboard::Key::Character(d.into()), mods);
    let _ = gui.update(rail("2"));
    assert_eq!(gui.active_tab, InboxTab::Archived, "Ctrl Alt 2 opens Archive");
    let _ = gui.update(rail("8"));
    assert_eq!(gui.selected_account_filter.as_deref(), Some("hungryserv"), "Ctrl Alt 8 opens the first account");
    let _ = gui.update(rail("1"));
    assert!(gui.active_tab == InboxTab::AllChats && gui.selected_account_filter.is_none());
    {
        let mut st = store.write().unwrap();
        let account_id = st.accounts[0].account_id.clone();
        st.network_spaces.push(beeper_core::store::NetworkSpace { id: "g1".into(), account_id, name: "Guild".into() });
    }
    let _ = gui.update(UiMessage::SelectAccount(Some("g1".into())));
    {
        let mut sim = simulator(gui.view_main());
        assert!(sim.find("Guild").is_ok(), "the header shows the server name");
        assert!(sim.find("You have no messages").is_ok(), "a server without chats is empty");
    }
    {
        let mut st = store.write().unwrap();
        let i = st.chats.iter().position(|c| c.account_id == st.accounts[0].account_id).expect("a chat of the account");
        st.chats[i].network_id = Some("g1".into());
        let shown: Vec<_> = gui.visible_chats(&st).iter().map(|c| c.id.clone()).collect();
        assert_eq!(shown, [st.chats[i].id.clone()], "the server shows only its chats");
    }
    {
        let cid = store.read().unwrap().chats[0].id.clone();
        let was = store.read().unwrap().chats[0].is_muted == Some(true);
        store.write().unwrap().config.demo = false;
        let _ = gui.update(UiMessage::ToggleFlag(cid.clone(), ChatFlag::Muted));
        let mut st = store.write().unwrap();
        st.config.demo = true;
        assert_eq!(st.chats[0].is_muted, Some(!was), "the mute toggle changes the chat");
        let queued =
            matches!(st.outbox.as_slice(), [(c, Outgoing::Flag(ChatFlag::Muted, on))] if *c == cid && *on != was);
        assert!(queued, "the mute toggle goes to the homeserver");
        st.outbox.clear();
        st.chats[0].is_pinned = Some(true);
        assert!(!st.set_chat_flag(0, ChatFlag::Archived, None), "pinned chats can't be archived");
        assert!(st.outbox.is_empty());
    }
    let _ = gui.update(rail("1"));
    let _ = gui.update(UiMessage::SelectTheme(ThemeChoice::System));
    for (mode, dark) in [(iced::theme::Mode::Dark, true), (iced::theme::Mode::Light, false)] {
        let _ = gui.update(UiMessage::SystemTheme(mode));
        assert_eq!(gui.dark, dark);
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
                let _ = app.update(UiMessage::SelectTheme(ThemeChoice::Dark));
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
        let emu_png = std::env::temp_dir().join("beeper_rs_iced_dark.png");
        write_rgba_png(&emu_png, shot.size.width, shot.size.height, &shot.rgba);
        assert!(emu_png.exists());
    })
    .join()
    .unwrap();

    #[cfg(target_os = "linux")]
    {
        let bin_exe = env!("CARGO_BIN_EXE_beeper-rs");
        let (wl_png, wl_settings_png, x_png) = (
            "/tmp/beeper_rs_wayland_window.png",
            "/tmp/beeper_rs_wayland_settings.png",
            "/tmp/beeper_rs_xvfb_window.png",
        );
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
            c.args(args)
                .envs(envs.iter().copied())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            c.spawn().unwrap_or_else(|e| panic!("spawn {cmd}: {e}"))
        };
        let stop = |mut child: std::process::Child| {
            child.kill().expect("stop display server");
            child.wait().expect("reap display server");
        };

        let wl_runtime = temp_dir("beeper_wl");
        std::fs::create_dir_all(&wl_runtime).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wl_runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let xdg = ("XDG_RUNTIME_DIR", wl_runtime.to_str().unwrap());
        let weston_args = [
            "--backend=headless",
            "--shell=kiosk",
            "--socket=beeper-wl-e2e",
            "--width=1440",
            "--height=900",
            "--no-config",
        ];
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
        for dir in [wl_runtime, fb_dir] {
            std::fs::remove_dir_all(dir).expect("remove test dir");
        }
    }
}
