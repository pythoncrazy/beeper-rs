mod common;

use beeper_api::{
    client::BeeperClient,
    http::{query_string, read_message, StatusCode},
    server::serve_http,
};
use beeper_core::{
    config::{is_original_beeper_dir, CentralConfig, APP_BUNDLE_ID, DEFAULT_RECOVERY_CODE},
    import::read_beeper_data,
    matrix::Outgoing,
    models::*,
    platform::{
        file_url_to_path, open_url_or_path_command, path_to_file_url, resolve_desktop_data_dir,
        resolve_original_beeper_data_dir, DisplayServerKind, OsKind,
    },
    store::{BeeperStore, Setup, SharedStore, StoreLock, NOTE_CHAT_ID},
    testing::temp_dir,
};
use beeper_ui::{
    login::{Login, LoginMsg, Outcome, Screen},
    search_settings,
    style::{ensure_fonts_loaded, fw, icon},
    write_rgba_png, BeeperGuiApp, InboxTab, SearchFilters, SearchPick, SettingsHit, SettingsTab, UiMessage,
};
use common::TestClient;
use iced::font::Weight;
use iced::{Size, Theme};
use iced_test::{emulator, simulator, Emulator, Instruction};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Duration};

/// The queued changes for the homeserver, which leave the outbox as if the homeserver took them.
fn take_outbox(st: &mut BeeperStore) -> Vec<(String, Outgoing)> {
    let out = std::mem::take(&mut st.outbox);
    out.into_iter()
        .map(|(row, chat, o)| {
            st.pushed(row, None, None);
            (chat, o)
        })
        .collect()
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
                while let Ok(m) = read_message(&mut stream, &mut buf, |_| usize::MAX).await {
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
                        ("/user", _)
                            if m.start_line.starts_with("PATCH ")
                                && auth.starts_with("Bearer syt_")
                                && (b["analyticsDisabled"].is_boolean() || b["email"].is_string()) =>
                        {
                            (200, json!({}))
                        }
                        ("/user", _) if m.start_line.starts_with("DELETE ") && auth.starts_with("Bearer syt_") => {
                            (200, json!({}))
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

    let store: SharedStore = StoreLock::shared(BeeperStore::from_config(cfg));
    let token = store.read().beeper_state.access_token.clone();
    tokio::spawn(serve_http(listener, store.clone()));

    let client = BeeperClient::new(format!("http://127.0.0.1:{port}"), token);
    (data_dir, port, store, client)
}

/// Runs on a thread with the 8 MiB stack of the app's main thread: the debug build of `BeeperGuiApp::update` needs more
/// than the 2 MiB of a test thread.
#[test]
fn e2e_full_beeper_lifecycle_api_ws_auth_messages_search_themes_os_and_gui() {
    let run = || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime").block_on(lifecycle())
    };
    let test = std::thread::Builder::new().stack_size(8 << 20).spawn(run).expect("start the test thread");
    if let Err(panic) = test.join() {
        std::panic::resume_unwind(panic);
    }
}

async fn lifecycle() {
    let fs = iced_test::renderer::graphics::text::font_system();
    ensure_fonts_loaded();
    {
        let mut w = fs.write().expect("font system");
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
                 CREATE TABLE thread_props ('accountID' VARCHAR, 'threadID' VARCHAR NOT NULL, 'props' JSON NOT NULL,
                     PRIMARY KEY (accountID, threadID));
                 INSERT INTO thread_props VALUES ('local-whatsapp_ba_Xq9', '!wa:beeper.local', '{\"note\":\"Call back\"}');
                 CREATE TABLE messages ('messageID' VARCHAR NOT NULL, 'accountID' VARCHAR NOT NULL, 'timestamp' INTEGER NOT NULL,
                     'threadID' VARCHAR NOT NULL, 'message' JSON NOT NULL, PRIMARY KEY(accountID, messageID));
                 CREATE TABLE participants (account_id TEXT NOT NULL, room_id TEXT NOT NULL, id TEXT NOT NULL, full_name TEXT,
                     nickname TEXT, img_url TEXT, is_verified INTEGER, cannot_message INTEGER, is_self INTEGER,
                     is_network_bot INTEGER, added_by TEXT, is_admin INTEGER, is_pending INTEGER, has_exited INTEGER);
                 CREATE TABLE participant_identifiers (account_id TEXT, participant_id TEXT, identifier TEXT, identifier_type TEXT);
                 CREATE TABLE mx_room_messages (eventID TEXT, roomID TEXT, timestamp INTEGER, type TEXT, isDeleted INTEGER,
                     hsOrder INTEGER, message JSON, threadRootEventID TEXT);
                 CREATE TABLE mx_threads (roomID TEXT, rootEventID TEXT, timestamp INTEGER, following INTEGER, lastReadHsOrder INTEGER);
                 INSERT INTO mx_threads VALUES ('!mx:beeper.local', '$x1', 1760000301500, 0, 900);
                 CREATE TABLE mx_reactions (roomID TEXT, reactionID TEXT, eventID TEXT, senderID TEXT, description TEXT,
                     \"order\" INTEGER, isDeleted INTEGER);
                 CREATE TABLE mx_read_receipts (roomID TEXT, userID TEXT, timestamp INTEGER, relatesToID TEXT, lastVisibleEventID TEXT);
                 INSERT INTO mx_read_receipts VALUES ('!wa:beeper.local', '@wa_ana:beeper.local', 1760000003000, '$m2', '$m2');
                 INSERT INTO mx_room_messages VALUES ('$x1', '!mx:beeper.local', 1760000301000, 'TEXT', 0, 900,
                     '{\"senderID\":\"@emile:beeper.com\",\"text\":\"from matrix\",\"isSender\":true}', NULL);
                 INSERT INTO mx_room_messages VALUES ('$x3', '!mx:beeper.local', 1760000300500, 'TEXT', 0, 899,
                     '{\"senderID\":\"@a:beeper.com\",\"text\":\"in thread\",\"extra\":{\"isReplyFallback\":true}}', '$x1');
                 INSERT INTO mx_room_messages VALUES ('$x2', '!mx:beeper.local', 1760000302000, 'REACTION', 0, 901, '{}', NULL);
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
    let shared: SharedStore = StoreLock::shared(fresh);
    let mut gui = BeeperGuiApp::new(shared.clone());
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
    assert!(shared.read().setup.is_none(), "nothing is imported before the sign-in");
    let session = MatrixSession {
        user_id: "@emile:beeper.com".into(),
        access_token: "syt_emile".into(),
        device_id: "NEWDEVICE".into(),
        jwt: None,
    };
    let profile = Profile {
        full_name: Some("Émile Zola".into()),
        username: Some("emile".into()),
        email: Some("emile@example.com".into()),
        ..Profile::default()
    };
    let _ =
        gui.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Imported(data, session, profile.clone(), Vec::new())))));
    assert_eq!(before, originals(&orig_dir), "Beeper Desktop files must never be modified");
    let none = beeper_core::matrix::E2eeSetup { cross_signing: true, verified: true, devices: Vec::new() };
    let _ = gui.update(UiMessage::Login(LoginMsg::E2ee(Ok(none))));
    assert_eq!(gui.login.screen, Screen::Accounts, "an imported session shows the chat accounts");
    let _ = gui.update(UiMessage::CloudAccounts(Err("offline".into())));
    assert_eq!(gui.status_banner, "Could not load your chat accounts: offline");
    let bridges = json!({ "signal": { "bridgeState": { "bridgeType": "signal" }, "remoteState": { "s1": { "state_event": "CONNECTED",
        "remote_profile": { "username": "alex", "phone": "+1 415-555-0142", "avatar": "mxc://beeper.com/a" } } } } });
    let cloud = beeper_core::cloud::cloud_accounts(&bridges).expect("parse the cloud bridges");
    assert_eq!(cloud[0].display_text().as_deref(), Some("+1 415-555-0142"), "the phone comes before the username");
    let profile = Profile { email: Some("zola@example.com".into()), ..profile };
    let _ = gui.update(UiMessage::CloudAccounts(Ok((profile, cloud))));
    assert_eq!(shared.read().accounts.iter().map(|a| a.account_id.as_str()).collect::<Vec<_>>(), ["signal"]);
    for bad in [
        json!([]),
        json!({ "whatsapp": { "bridgeState": {}, "remoteState": { "w": { "state_event": "CONNECTED" } } } }),
    ] {
        assert_eq!(beeper_core::cloud::cloud_accounts(&bad).unwrap_err().status, 502, "{bad} is rejected");
    }
    let _ = gui.update(UiMessage::Login(LoginMsg::Continue));
    assert!(gui.login_window.is_none() && gui.main_window.is_some(), "Continue opens the app");
    let chats = {
        let st = shared.read();
        assert_eq!(st.setup, Some(Setup::Imported));
        assert_eq!(st.beeper_state.user_id, "@emile:beeper.com");
        assert_eq!(st.beeper_state.access_token, "syt_emile");
        assert_eq!(st.me().email.as_deref(), Some("zola@example.com"), "whoami updates the imported email");
        assert_eq!(st.me().full_name.as_deref(), Some("Émile Zola"));
        let title = |id: &str| st.chat(id).title.as_str();
        assert_eq!(title("!wa:beeper.local"), "Ana Núñez", "a single chat without a title uses the other person");
        assert_eq!(st.chat("!wa:beeper.local").note.as_deref(), Some("Call back"));
        assert_eq!(title("!sl:beeper.local"), "@sl_x:beeper.local");
        assert_eq!(title("!mx:beeper.local"), "Unnamed chat");
        assert!(st.find_chat("label_inbox").is_none(), "label threads are not chats");
        let network = |id: &str| beeper_core::config::account_network(&st.chat(id).account_id).bridge_id;
        assert_eq!(
            ["!wa:beeper.local", "!dc:beeper.local", "!sl:beeper.local", "!mx:beeper.local", "!zz:beeper.local"]
                .map(network),
            ["whatsapp", "discord", "slack", "hungryserv", "hungryserv"]
        );
        let dc = st.chat("!dc:beeper.local");
        assert!(dc.is_pinned == Some(true) && dc.is_muted == Some(true) && dc.participants.items.len() == 2);
        assert_eq!(st.pin_rank.get("!dc:beeper.local"), Some(&0));
        assert_eq!(st.chat("!sl:beeper.local").is_hidden_from_inbox, Some(true));
        assert_eq!(
            st.chat("!mx:beeper.local").preview.as_ref().and_then(|p| p.text.as_deref()),
            Some("loved \"from matrix\"")
        );
        let msg = |id: &str| st.message(id);
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
        assert!(st.loaded("$m6").is_none() && st.loaded("$m7").is_none());
        assert_eq!(msg("$x1").text.as_deref(), Some("from matrix"));
        assert_eq!(msg("$x1").reactions[0].reaction_key, "❤️");
        assert!(st.loaded("$x2").is_none(), "reaction events are not messages");
        assert_eq!(
            (msg("$x3").thread_root.as_deref(), msg("$x3").fallback, msg("$x3").broadcast),
            (Some("$x1"), true, false)
        );
        let mx = st.chat("!mx:beeper.local");
        assert_eq!(
            (mx.thread_reads["$x1"].as_str(), mx.follows.unfollowed.as_slice()),
            (beeper_core::store::sort_key(900).as_str(), ["$x1".to_string()].as_slice())
        );
        st.chats.clone()
    };
    assert_eq!(chats.len(), 6);
    for c in &chats {
        gui.select_chat(&c.id);
        assert!(simulator(gui.view_main()).find(beeper_ui::short_title(c)).is_ok(), "chat {} renders", c.title);
    }
    gui.select_chat("!sl:beeper.local");
    assert!(simulator(gui.view_main()).find("Click to load").is_ok());
    assert!(simulator(gui.view_settings()).find("zola@example.com").is_ok());
    drop((gui, shared));
    let mut reopened = BeeperStore::from_config(CentralConfig::new(rs_dir.clone(), "127.0.0.1".into(), 23379, None));
    assert_eq!(reopened.setup, Some(Setup::Imported));
    assert_eq!(reopened.pin_rank.get("!dc:beeper.local"), Some(&0));
    assert!(reopened.signed_in() && reopened.chats.len() == 6 && reopened.stored("1", []).len() == 7);
    let (_, sent_id, _) = reopened.send_message(NOTE_CHAT_ID, Some("After restart".into()), None, None).unwrap();
    let last = reopened.stored("1 ORDER BY sort_key DESC LIMIT 1", []).pop().expect("a message");
    assert_eq!(last.id, sent_id, "new messages sort after imported ones");
    reopened.delete_message(NOTE_CHAT_ID, &sent_id).unwrap();
    drop(reopened);
    let mut fresh_gui = BeeperGuiApp::new(StoreLock::shared(BeeperStore::from_config(CentralConfig::new(
        temp_dir("BeeperRs-fresh"),
        "127.0.0.1".into(),
        23379,
        None,
    ))));
    fresh_gui.login = Login::new(Some(orig_dir.clone()));
    let _ = fresh_gui.update(UiMessage::Login(LoginMsg::Fresh));
    assert_eq!(fresh_gui.login.screen, Screen::Welcome);
    assert_eq!(fresh_gui.store.read().setup, Some(Setup::Fresh));
    let fresh_dir = fresh_gui.store.read().config.data_dir.clone();
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
        let old_path = legacy.join("media").join("{id}").display().to_string();
        db.execute(
            "UPDATE assets SET json = json_set(json_remove(json, '$.disk_name'), '$.local_file_path',
                 replace(?1, '{id}', upload_id || '_a_b.txt'))",
            [old_path],
        )
        .unwrap();
        db.execute_batch(
            "DROP TABLE messages; DROP TABLE messages_fts; DROP TABLE outbox;
             CREATE TABLE messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL) STRICT;
             PRAGMA user_version = 1;",
        )
        .unwrap();
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

    let (_, port, store, client) = spawn_e2e_beeper_instance("master", None).await;
    assert_eq!(store.read().beeper_state.user_id, "@alex:beeper.com");
    assert_eq!(store.read().bridge_accounts.len(), 6);

    let info = client.info().await;
    let (_, raw) = client.send("GET", "/v1/info", &[], None).await;
    let raw = String::from_utf8(raw).unwrap();
    assert!(
        raw.starts_with(r#"{"app":{"name":"Beeper","version":"#) && raw.contains(r#""server":{"status":"running","#)
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(info.platform.arch, "x64", "Node's process.arch");
    assert_eq!(info.app.bundle_id, APP_BUNDLE_ID);
    assert_eq!(info.server.port, port);
    assert_eq!(info.endpoints.ws_events, format!("http://127.0.0.1:{port}/v1/ws"));
    assert_eq!(info.endpoints.mcp, format!("http://127.0.0.1:{port}/v0/mcp"));
    {
        let _ui = store.read();
        let chats = timeout(Duration::from_secs(5), client.list_chats()).await;
        assert!(!chats.expect("a GET shares the store lock with the UI").items.is_empty());
    }

    let (_, _, pages, pager) = spawn_e2e_beeper_instance("pages", None).await;
    let (_, accounts) = pager.request("GET", "/v1/accounts", None).await;
    let mut ws = pager.connect_ws(vec!["*".into(), "!x".into()]).await;
    assert_eq!(ws.recv().await.unwrap()["type"], "ready");
    let refused = ws.recv().await.unwrap();
    let message = "chatIDs cannot combine '*' with specific IDs";
    assert_eq!(refused, json!({ "type": "error", "code": "INVALID_PAYLOAD", "message": message }));
    let mut ws = pager.connect_ws(vec!["!wa_eric:beeper.com".into(), "!wa_eric:beeper.com".into()]).await;
    assert_eq!(ws.recv().await.unwrap()["type"], "ready");
    assert_eq!(ws.recv().await.unwrap()["chatIDs"], json!(["!wa_eric:beeper.com"]), "unique chat IDs");
    pager.send_message("!tg_rust:beeper.com", Some("not subscribed"), None, None).await;
    pager.send_message("!wa_eric:beeper.com", Some("subscribed"), None, None).await;
    let event = ws.recv().await.unwrap();
    assert_eq!(
        (&event["type"], &event["chatID"], &event["seq"]),
        (&json!("message.upserted"), &json!("!wa_eric:beeper.com"), &json!(1))
    );
    assert_eq!(event["entries"][0]["text"], "subscribed");
    let anon = BeeperClient::new(format!("http://{}", pager.host_port), "");
    let (status, page) = anon.send("GET", "/focus/local_wa_eric?draft=Hi%20there", &[], None).await;
    assert!(status == StatusCode::OK && String::from_utf8(page).unwrap().contains("You can close this window"));
    let eric_draft = pages.read().chats.iter().find(|c| c.id == "!wa_eric:beeper.com").unwrap().draft.clone();
    assert_eq!(eric_draft.unwrap().text.as_deref(), Some("Hi there"));
    assert_eq!(
        anon.send("GET", "/focus/local_wa_eric", &[("origin", "https://x.example")], None).await.0,
        StatusCode(403)
    );
    assert_eq!(anon.send("GET", "/focus?draft=x", &[], None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(anon.send("GET", "/focus/nope", &[], None).await.0, StatusCode::NOT_FOUND);
    pager.patch_chat_draft("!wa_eric:beeper.com", None).await;
    let upload = json!({ "content": "aGVsbG8=", "fileName": "hello.txt", "mimeType": "text/plain" });
    let (status, uploaded) = pager.request("POST", "/v1/assets/upload", Some(upload)).await;
    assert_eq!((status, &uploaded["fileName"]), (StatusCode::OK, &json!("hello.txt")));
    let (status, refused) = pager.request("POST", "/v1/assets/upload", Some(json!({ "content": "aGVsbG8=" }))).await;
    assert_eq!((status, &refused["code"]), (StatusCode::BAD_REQUEST, &json!("MISSING_FILENAME")));
    pager.add_reaction("!wa_eric:beeper.com", "msg_1001", "👍").await;
    let unreact = query_string("/v1/chats/!wa_eric:beeper.com/messages/msg_1001/reactions", &[("reactionKey", "👍")]);
    let (status, removed) = pager.request("DELETE", &unreact, None).await;
    assert_eq!((status, &removed["reactionKey"]), (StatusCode::OK, &json!("👍")));
    let (_, bridges) = pager.request("GET", "/v1/bridges", None).await;
    let whatsapp = bridges["items"].as_array().unwrap().iter().find(|b| b["id"] == "whatsapp").unwrap();
    assert_eq!(
        (&whatsapp["status"], &whatsapp["accounts"][0]["accountID"]),
        (&json!("connected"), &json!("acc_whatsapp"))
    );
    let (_, logins) = pager.request("GET", "/v1/bridges/whatsapp/logins", None).await;
    assert_eq!(logins["items"][0]["accountIDs"], json!(["acc_whatsapp"]));
    let (_, login) = pager.request("GET", "/v1/bridges/whatsapp/logins/login_whatsapp_01", None).await;
    assert_eq!(
        (&login["status"], &login["removeScopes"]),
        (&json!("connected"), &json!(["current-device", "all-devices"]))
    );
    // A demo account has no Beeper Cloud bridges to log in to.
    let (status, flows) = pager.request("GET", "/v1/bridges/whatsapp/login-flows", None).await;
    assert_eq!((status, &flows["code"]), (StatusCode::CONFLICT, &json!("needs_login")));
    assert_eq!(accounts[0]["bridge"], json!({ "id": "whatsapp", "type": "whatsapp", "provider": "local" }));
    let account = &accounts[2];
    assert_eq!(
        (&account["loginID"], &account["network"], &account["status"]),
        (&json!("login_telegram_01"), &json!("Telegram"), &json!("connected"))
    );
    let (mut listed, mut cursor) = (Vec::new(), None::<String>);
    loop {
        let path = match &cursor {
            None => "/v1/chats?limit=4".to_string(),
            Some(c) => query_string("/v1/chats", &[("limit", "4"), ("cursor", c)]),
        };
        let (_, page) = pager.request("GET", &path, None).await;
        let items = page["items"].as_array().unwrap();
        listed.extend(items.iter().map(|c| (c["lastActivity"].clone(), c["id"].as_str().unwrap().to_string())));
        if page["hasMore"] == false {
            break;
        }
        assert_eq!(items.len(), 4);
        cursor = Some(page["oldestCursor"].as_str().unwrap().to_string());
    }
    let ids: Vec<&String> = listed.iter().map(|(_, id)| id).collect();
    let all: Vec<String> = pages.read().chats.iter().map(|c| c.id.clone()).collect();
    assert_eq!((ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len()), (all.len(), all.len()));
    assert!(listed.windows(2).all(|w| w[0].0.as_str() >= w[1].0.as_str()), "newest activity first: {listed:?}");
    let (_, first) = pager.request("GET", "/v1/chats", None).await;
    let eric = &first["items"].as_array().unwrap().iter().find(|c| c["id"] == "!wa_eric:beeper.com").unwrap();
    assert_eq!(
        (&eric["network"], &eric["isMuted"], &eric["snooze"]),
        (&json!("WhatsApp"), &json!(false), &Value::Null)
    );
    assert!(eric["preview"].is_object() && eric.get("inboxGroup").is_none() && eric.get("labelIDs").is_none());
    let (_, one) = pager.request("GET", "/v1/chats/local_wa_eric?maxParticipantCount=1", None).await;
    assert_eq!((&one["participants"]["hasMore"], &one["participants"]["total"]), (&json!(true), &json!(2)));
    assert!(one.get("preview").is_none() && one["labels"][0]["name"].is_string(), "{one}");
    assert_eq!((first["items"].as_array().unwrap().len(), &first["hasMore"]), (all.len().min(25), &json!(false)));
    let (_, two) = pager.request("GET", "/v1/chats?accountIDs=acc_whatsapp&accountIDs=acc_telegram", None).await;
    let wanted =
        pages.read().chats.iter().filter(|c| ["acc_whatsapp", "acc_telegram"].contains(&&*c.account_id)).count();
    assert!(wanted > 0 && two["items"].as_array().unwrap().len() == wanted);
    for bad in ["limit=0", "limit=201", "cursor=nope", "direction=up"] {
        assert_eq!(pager.request("GET", &format!("/v1/chats?{bad}"), None).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let weekly = "!tg_weekly:beeper.com";
    for n in 0..22 {
        pager.send_message(weekly, Some(&format!("Page message {n}")), None, None).await;
    }
    let url = |q: &str| format!("/v1/chats/{weekly}/messages?{q}");
    let keys = |p: &Value| -> Vec<String> {
        p["items"].as_array().unwrap().iter().map(|m| m["sortKey"].as_str().unwrap().to_string()).collect()
    };
    let (_, newest) = pager.request("GET", &url(""), None).await;
    let newest_keys = keys(&newest);
    assert_eq!((newest_keys.len(), &newest["hasMore"]), (20, &json!(true)));
    assert!(newest_keys.is_sorted_by(|a, b| a > b), "newest first");
    assert_eq!(newest["items"][0]["text"], "Page message 21");
    assert_eq!(newest["oldestCursor"].as_str(), newest_keys.last().map(String::as_str));
    let (_, older) =
        pager.request("GET", &url(&format!("cursor={}", newest["oldestCursor"].as_str().unwrap())), None).await;
    assert_eq!((keys(&older).len(), &older["hasMore"]), (3, &json!(false)));
    let (_, newer) = pager.request("GET", &url(&format!("cursor={}&direction=after", newest_keys[2])), None).await;
    assert_eq!(keys(&newer), newest_keys[..2]);
    let (_, hits) = pager.request("GET", "/v1/messages/search?query=page&limit=5", None).await;
    assert_eq!((keys(&hits), &hits["hasMore"]), (newest_keys[..5].to_vec(), &json!(true)));
    assert_eq!(hits["chats"][weekly]["id"], weekly, "the chats of the messages");
    let next =
        query_string("/v1/messages/search", &[("query", "page"), ("cursor", hits["oldestCursor"].as_str().unwrap())]);
    let rest = keys(&pager.request("GET", &next, None).await.1);
    assert_eq!((rest.len(), &rest[..15]), (17, &newest_keys[5..]), "the other 17 page messages");
    let (_, local) = pager.request("GET", "/v1/messages/search?chatIDs=local_x1&limit=20", None).await;
    assert_eq!(keys(&local), newest_keys, "a local chat ID and no query");
    let media = "/v1/messages/search?mediaTypes=image&mediaTypes=file";
    let kinds: Vec<Value> = pager.request("GET", media, None).await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["attachments"].clone())
        .collect();
    let wanted = |a: &Value| a.as_array().unwrap().iter().any(|a| a["type"] == "img" || a["type"] == "unknown");
    assert!(kinds.len() == 6 && kinds.iter().all(wanted), "{kinds:?}");
    let (_, muted) = pager.request("GET", "/v1/messages/search?accountIDs=acc_imessage&includeMuted=false", None).await;
    assert!(muted["items"].as_array().unwrap().iter().all(|m| m["chatID"] != "!im_sarah:beeper.com"));
    let (_, late) = pager.request("GET", "/v1/messages/search?dateAfter=2100-01-01T00:00:00%2B02:00", None).await;
    assert_eq!(late["items"], json!([]));
    for bad in [
        "limit=21",
        "mediaTypes=audio",
        "chatType=dm",
        "dateAfter=2026-02-01T00:00:00Z&dateBefore=2026-01-01T00:00:00Z",
    ] {
        let status = pager.request("GET", &format!("/v1/messages/search?{bad}"), None).await.0;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(pager.request("GET", "/v1/messages/search?chatIDs=nope", None).await.0, StatusCode::NOT_FOUND);
    let titles = |p: &Value| -> Vec<String> {
        p["items"].as_array().unwrap().iter().map(|c| c["title"].as_str().unwrap().to_string()).collect()
    };
    let (_, by_title) = pager.request("GET", "/v1/chats/search?query=eric", None).await;
    assert_eq!(titles(&by_title), ["Eric Migicovsky"], "the default scope matches titles");
    let (_, by_people) = pager.request("GET", "/v1/chats/search?query=eric&scope=participants", None).await;
    assert_eq!(titles(&by_people).len(), 2, "{by_people}");
    let (_, rust) = pager.request("GET", "/v1/chats/search?query=rust&accountIDs=acc_signal", None).await;
    assert_eq!(rust["items"], json!([]));
    let eric = "/v1/chats/!wa_eric:beeper.com";
    let patch =
        json!({ "title": "Eric M", "description": null, "isPinned": false, "isMuted": true, "isArchived": true });
    let (status, patched) = pager.request("PATCH", eric, Some(patch)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((&patched["title"], &patched["description"]), (&json!("Eric M"), &Value::Null));
    assert_eq!(
        (&patched["isPinned"], &patched["isMuted"], &patched["isArchived"]),
        (&json!(false), &json!(true), &json!(true))
    );
    assert_eq!(
        pager.request("PATCH", eric, Some(json!({ "isMuted": "yes", "title": "x" }))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(pager.request("GET", eric, None).await.1["title"], "Eric M", "a bad field changes nothing");
    let upload = pager.upload_asset_base64("a.png", "image/png", b"png").await;
    let draft =
        json!({ "draft": { "text": "with file", "attachments": { upload["uploadID"].as_str().unwrap(): {} } } });
    let (_, drafted) = pager.request("PATCH", eric, Some(draft)).await;
    assert_eq!(drafted["draft"]["attachments"][upload["uploadID"].as_str().unwrap()]["fileName"], "a.png");
    let (status, read) = pager.request("POST", &format!("{eric}/read"), None).await;
    assert_eq!((status, &read["id"], &read["unreadCount"]), (StatusCode::OK, &json!("!wa_eric:beeper.com"), &json!(0)));
    let remind = json!({ "reminder": { "remindAt": "2027-01-01T09:00:00Z" } });
    assert_eq!(pager.request("POST", &format!("{eric}/reminders"), Some(remind)).await.0, StatusCode::NO_CONTENT);
    let reminder = pager.request("GET", eric, None).await.1["reminder"].clone();
    // Desktop's `setReminder` dismisses a reminder on an incoming message by default.
    assert_eq!(reminder, json!({ "remindAt": "2027-01-01T09:00:00.000Z", "dismissOnIncomingMessage": true }));
    let bad = json!({ "reminder": { "remindAt": "tomorrow" } });
    assert_eq!(pager.request("POST", &format!("{eric}/reminders"), Some(bad)).await.0, StatusCode::BAD_REQUEST);
    let (status, sent) = pager.request("POST", &format!("{eric}/messages"), Some(json!({ "text": "hi" }))).await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let rpc = |id: u32, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let hello =
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "e2e", "version": "1" } });
    let (status, init) = client.mcp(rpc(1, "initialize", hello)).await;
    assert_eq!((status, &init["result"]["serverInfo"]["name"]), (StatusCode::OK, &json!("beeper_desktop_api_api")));
    let (status, _) = client.mcp(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!(status, StatusCode(202), "notifications get no reply");
    let (_, list) = client.mcp(rpc(2, "tools/list", json!({}))).await;
    let names: Vec<&str> =
        list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 12);
    assert!(names.contains(&"send_message") && names.contains(&"search_chats"));
    let (_, only) = client.mcp_at("/v0/mcp?tool=search_chats&tool=get_chat", rpc(2, "tools/list", json!({}))).await;
    assert_eq!(only["result"]["tools"].as_array().unwrap().len(), 2, "the tool filter of the MCP URL");
    let (_, without) = client.mcp_at("/v0/mcp?no_tool=search_chats", rpc(2, "tools/list", json!({}))).await;
    assert_eq!(without["result"]["tools"].as_array().unwrap().len(), 11);
    let (status, unknown) = client.mcp_at("/v0/mcp?tool=nope", rpc(2, "tools/list", json!({}))).await;
    let message = "Invalid request: The following filters did not match any endpoints: tool=nope";
    assert_eq!((status, &unknown["error"]["message"]), (StatusCode::BAD_REQUEST, &json!(message)));
    let (status, bad) = client.mcp(json!({ "foo": 1 })).await;
    assert_eq!((status, &bad["error"]["code"], &bad["id"]), (StatusCode::BAD_REQUEST, &json!(-32700), &Value::Null));
    let tool = |id, name: &str, args: Value| rpc(id, "tools/call", json!({ "name": name, "arguments": args }));
    let (_, accounts) = client.mcp(tool(3, "get_accounts", json!({}))).await;
    let accounts: Value = serde_json::from_str(accounts["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(accounts.as_array().unwrap().len(), store.read().accounts.len());
    let chat_id = store.read().chats[0].id.clone();
    let (_, messages) = client.mcp(tool(4, "list_messages", json!({ "chatID": chat_id }))).await;
    assert!(messages["result"]["content"][0]["text"].as_str().unwrap().contains("\"items\""));
    let (_, missing) = client.mcp(tool(5, "get_chat", json!({ "chatID": "nope" }))).await;
    assert_eq!(missing["error"]["message"], "404 {\"message\":\"Chat not found\",\"code\":\"NOT_FOUND\"}");

    let mut requests = store.read().connect_tx.subscribe();
    let approver = store.clone();
    let approve = tokio::spawn(async move {
        let (id, outcome) = requests.recv().await.unwrap();
        assert!(outcome.is_none(), "the page asks for a token");
        let scopes = approver.read().connect.requests[0].scopes.clone();
        assert_eq!(scopes, ["read", "write"]);
        approver.write().answer_token_request(&id, Some((scopes, 3600)));
        assert_eq!(requests.recv().await.unwrap(), (id, Some(beeper_core::connect::Outcome::Approved)));
    });
    let base = format!("http://127.0.0.1:{port}");
    let mut oauth = BeeperClient::new(&base, "");
    let oauth_token =
        oauth.oauth_pkce_login("Beeper Raycast E2E", "super_secret_pkce_verifier_2026_beeper_rs").await.unwrap();
    approve.await.unwrap();
    assert!(oauth_token.starts_with("bdapi_") && store.read().connect.tokens.len() == 1);
    assert!(!store.write().connect.tokens_json().contains(&oauth_token), "only the hash is stored");
    assert_eq!(oauth.list_accounts().await.len(), store.read().accounts.len(), "the approved token reads");
    let anon = BeeperClient::new(&base, "");
    let unauthorized = json!({ "message": "Unauthorized: Invalid or missing token", "code": "unauthorized" });
    assert_eq!(anon.request("GET", "/v1/accounts", None).await, (StatusCode(401), unauthorized.clone()));
    let chat = json!({ "message": "Chat not found", "code": "NOT_FOUND" });
    assert_eq!(oauth.request("GET", "/v1/chats/nope", None).await, (StatusCode(404), chat));
    let route = json!({ "message": "Not found", "code": "not_found" });
    assert_eq!(oauth.request("GET", "/v1/nope", None).await, (StatusCode(404), route));
    let (status, refused) = anon.mcp(rpc(9, "tools/list", json!({}))).await;
    assert_eq!((status, &refused["id"], &refused["error"]["code"]), (StatusCode(401), &json!(9), &json!(-32000)));
    let reader = BeeperClient::new(&base, store.write().connect.create(vec!["read".into()], 0, None));
    let (status, refused) =
        reader.request("POST", &format!("/v1/chats/{chat_id}/messages"), Some(json!({ "text": "x" }))).await;
    assert_eq!(
        (status, refused["message"].as_str()),
        (StatusCode(403), Some("Insufficient permissions. Required scopes: write, missing: write"))
    );
    let (status, refused) = reader.setup_step("verifications", json!({})).await;
    assert_eq!(
        (status, refused["message"].as_str()),
        (StatusCode(403), Some("Insufficient permissions. Required scopes: read, write, missing: write"))
    );
    assert_eq!(anon.request("GET", "/v1/app/setup", None).await, (StatusCode(401), unauthorized.clone()));
    assert_eq!(anon.request("GET", "/v1/app/setup/nope", None).await.0, StatusCode(404), "no route needs no token");
    let (_, resource) = anon.request("GET", "/.well-known/oauth-protected-resource/v0/mcp", None).await;
    assert_eq!(resource["resource"], format!("{base}/v0/mcp"));
    let (_, seen) = anon.request("POST", "/oauth/introspect", Some(json!({ "token": oauth_token }))).await;
    assert_eq!(
        (&seen["active"], &seen["scope"], &seen["client"]["name"]),
        (&json!(true), &json!("read write"), &json!("Beeper Raycast E2E"))
    );
    assert_eq!(
        anon.request("POST", "/oauth/revoke", Some(json!({ "token": oauth_token }))).await,
        (StatusCode::OK, json!({}))
    );
    assert_eq!(
        oauth.request("GET", "/v1/accounts", None).await.1["message"],
        "Invalid token",
        "a revoked token is refused"
    );

    assert_eq!(client.setup_step("start", json!({})).await.0, StatusCode::CONFLICT, "demo account is signed in");
    let cloud = fake_beeper_cloud().await;
    for (name, code, user) in [("login", "123456", "alex"), ("register", "654321", "newbie")] {
        let (dir, fresh_port, st, c) = spawn_e2e_beeper_instance(name, Some(&cloud)).await;
        assert!(!st.read().signed_in() && st.read().chats.is_empty(), "fresh install is signed out");
        let reset = "verification/recovery-key/reset";
        assert_eq!(c.setup_step(reset, json!({})).await, (StatusCode(401), unauthorized.clone()));
        let token = st.write().connect.create(vec!["read".into(), "write".into()], 0, None);
        let login = json!({ "message": "Sign in to Beeper Desktop first", "code": "needs_login" });
        let tokened = BeeperClient::new(format!("http://127.0.0.1:{fresh_port}"), token);
        assert_eq!(tokened.setup_step(reset, json!({})).await, (StatusCode::CONFLICT, login));
        let signed_out = r#"{"state":"needs-login","e2ee":{"initialized":false,"secretStorage":false,"crossSigning":false,"verified":false,"secrets":{"masterKey":false,"selfSigningKey":false,"userSigningKey":false,"megolmBackupKey":false,"recoveryKey":false},"keyBackup":false,"firstSyncDone":false,"hasBackedUpRecoveryKey":false}}"#;
        let signed_out = serde_json::from_str(signed_out).unwrap();
        assert_eq!(c.request("GET", "/v1/app/setup", None).await, (StatusCode::OK, signed_out), "Desktop's body");
        let (status, start) = c.setup_step("start", json!({})).await;
        assert_eq!((status, &start["signInMethods"]), (StatusCode::OK, &json!(["email"])));
        let id = start["setupRequestID"].clone();
        let email = json!({ "setupRequestID": id, "email": format!("{user}@example.com") });
        assert_eq!(c.setup_step("email", email).await.0, StatusCode::NO_CONTENT);
        let (bad, err) = c.setup_step("response", json!({ "setupRequestID": id, "response": "000000" })).await;
        assert_eq!((bad, err["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_sign_in_code")));
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
        assert_eq!(
            (&done["session"]["state"], &done["session"]["matrix"]["userID"]),
            (&json!("initializing"), &done["matrix"]["userID"])
        );
        assert_eq!(c.setup_step("start", json!({})).await.0, StatusCode::CONFLICT);
        assert_eq!(st.read().me().full_name.as_deref(), Some(format!("{user} Real").as_str()));
        drop((st, c));
        let mut cfg = CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23380, None);
        cfg.api_url.clone_from(&cloud);
        let again = BeeperStore::from_config(cfg);
        assert_eq!(again.beeper_state.access_token, format!("syt_{user}"), "sign-in survives a restart");
        assert_eq!(again.chats.len(), 1);
        drop(again);
    }

    let (_, _, login_store, _) = spawn_e2e_beeper_instance("gui_login", Some(&cloud)).await;
    let mut login = BeeperGuiApp::new(login_store.clone());
    assert!(login.pane.selected_chat_id.is_none());
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
    shows(&login, &["Next", "Please enter your email."]);
    let _ = login.update(UiMessage::Login(LoginMsg::Email("alex@example.com".into())));
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Requested("req_1".into())))));
    assert_eq!(login.login.screen, Screen::Code);
    shows(&login, &["Check alex@example.com\nfor your login code", "Open Email"]);
    let _ = login.update(UiMessage::Login(LoginMsg::Code("12 34 56".into())));
    assert_eq!(login.login.code, "123456");
    let refused = beeper_core::cloud::CloudError { status: 401, message: "Invalid code".into() };
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Err(refused))));
    assert_eq!(login.login.error.as_deref(), Some("Invalid code"), "a refused code shows under the heading");
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
    assert!(profile.analytics_disabled, "whoami without analyticsDisabled disables analytics, as Desktop does");
    let patch_cloud = cloud.clone();
    let saved = tokio::task::spawn_blocking(move || {
        let c = beeper_core::cloud::Cloud::new(&patch_cloud, &patch_cloud);
        (c.set_analytics_disabled("syt_alex", false), c.set_analytics_disabled("bad", false).map_err(|e| e.status))
    })
    .await
    .unwrap();
    assert_eq!(saved, (Ok(()), Err(404)), "Share Analytics patches analyticsDisabled of the user");
    let profile_cloud = cloud.clone();
    let profile_calls = tokio::task::spawn_blocking(move || {
        let c = beeper_core::cloud::Cloud::new(&profile_cloud, &profile_cloud);
        (c.set_email("syt_alex", "new@example.com"), c.delete_account("syt_alex"), c.delete_account("bad").is_err())
    })
    .await
    .unwrap();
    assert_eq!(profile_calls, (Ok(()), Ok(()), true), "the profile page patches the email and deletes the user");
    let _ = login.update(UiMessage::Login(LoginMsg::Done(Ok(Outcome::Session(session, profile, accounts)))));
    assert!(login.login.busy, "the sign-in waits for the encrypted messaging check");
    let none = beeper_core::matrix::E2eeSetup { cross_signing: true, verified: true, devices: Vec::new() };
    // The check reads the keys of a set-up device into the state of the app.
    {
        let mut st = login_store.write();
        let s = &mut st.beeper_state;
        (s.verified, s.key_backup, s.first_sync_done) = (true, true, true);
        let k = &mut s.secrets;
        (k.master_key, k.self_signing_key, k.user_signing_key, k.megolm_backup_key) = (true, true, true, true);
    }
    let _ = login.update(UiMessage::Login(LoginMsg::E2ee(Ok(none))));
    assert_eq!(login_store.read().accounts.len(), 4);
    assert!(
        login.login_window.is_none() && login.main_window.is_some(),
        "connected chat accounts skip Connect Chat Accounts"
    );
    assert_eq!(login.pane.selected_chat_id.as_deref(), Some(NOTE_CHAT_ID));
    assert!(simulator(login.view_main()).find("Note").is_ok());
    let _ = login.update(UiMessage::ShareAnalytics);
    assert!(!login_store.read().whoami.analytics_disabled, "Share Analytics turns on at once");
    let _ = login.update(UiMessage::AnalyticsSaved(true, Err("offline".into())));
    assert!(login_store.read().whoami.analytics_disabled, "a failed save turns Share Analytics back off");
    assert_eq!(login.status_banner, "Something went wrong: offline");
    drop((login, login_store));

    let not_connected = json!({ "message": "Matrix is not connected yet", "code": "UNKNOWN_ERROR" });
    assert_eq!(client.setup_step("verifications", json!({})).await, (StatusCode(500), not_connected));
    let ready = json!({ "message": "Beeper Desktop is already set up", "code": "already_ready" });
    let key = json!({ "recoveryKey": DEFAULT_RECOVERY_CODE });
    assert_eq!(client.setup_step("verification/recovery-key", key).await, (StatusCode::CONFLICT, ready));
    let (_, app) = client.request("GET", "/v1/app/setup", None).await;
    assert_eq!((&app["state"], &app["e2ee"]["hasBackedUpRecoveryKey"]), (&json!("ready"), &json!(true)));
    assert_eq!(client.request("GET", "/v1/app/setup/verifications", None).await.1, json!({ "items": [] }));
    let details = json!({ "action": "cancel", "verificationID": "x", "availableActions": [] });
    let (status, refused) = client.setup_step("verifications/x/cancel", json!({})).await;
    assert_eq!(
        (status, &refused["code"], &refused["details"]),
        (StatusCode::CONFLICT, &json!("verification_action_not_available"), &details)
    );

    let (status, _) = client.request("POST", "/v1/bridges/linkedin/login-sessions", Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "a demo account adds no account");

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

    let mut focus_rx = store.read().focus_tx.subscribe();
    assert_eq!(client.focus(Some("!wa_eric:beeper.com"), None, Some("Focus draft")).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap().as_deref(), Some("!wa_eric:beeper.com"));
    let eric_idx = store.read().chat_idx("!wa_eric:beeper.com");
    let focus_draft = store.read().chats[eric_idx].draft.clone().unwrap().text;
    assert_eq!(focus_draft.as_deref(), Some("Focus draft"));
    assert_eq!(client.patch_chat_draft("!wa_eric:beeper.com", None).await.0, StatusCode::OK);
    assert_eq!(client.focus(None, None, None).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap(), None);
    assert_eq!(client.focus(Some("!missing:beeper.com"), None, None).await, StatusCode::NOT_FOUND);
    assert_eq!(
        client.focus(None, Some("msg_missing"), None).await,
        StatusCode::OK,
        "a message without a chat only focuses"
    );
    assert_eq!(focus_rx.recv().await.unwrap(), None);
    assert_eq!(client.focus(None, None, Some("No chat")).await, StatusCode(500));

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
    assert_eq!(client.focus(Some("!im_sarah:beeper.com"), Some(&sent_msg_id), None).await, StatusCode::OK);
    assert_eq!(focus_rx.recv().await.unwrap().as_deref(), Some("!im_sarah:beeper.com"));

    let mut next = async || timeout(Duration::from_secs(2), ws_rx.recv()).await.unwrap().unwrap();
    assert_eq!(next().await, json!({ "type": "ready", "version": 1, "chatIDs": [] }));
    assert_eq!(next().await, json!({ "type": "subscriptions.updated", "chatIDs": ["*"], "app": { "state": false } }));
    let ws_ev = next().await;
    assert_eq!((&ws_ev["seq"], ws_ev["ids"].as_array().unwrap().len()), (&json!(1), 1), "{ws_ev}");
    assert!(ws_ev["ts"].is_string() && ws_ev["chatID"].is_string() && ws_ev.get("message").is_none());

    let edit_resp =
        client.edit_message("!im_sarah:beeper.com", &sent_msg_id, "Routed message via merged chat (edited v2)").await;
    assert_eq!(edit_resp["success"], true);

    let sarah_msgs = client.list_messages(&merged_chat_id).await;
    let edited_msg = sarah_msgs.items.iter().find(|m| m["id"] == *sent_msg_id).unwrap();
    assert_eq!(edited_msg["text"], "Routed message via merged chat (edited v2)");
    assert!(edited_msg["editedTimestamp"].is_string() && edited_msg.get("edit_history").is_none());
    let sarah = store.read().message_idx(&sent_msg_id);
    assert_eq!(store.read().messages[sarah].edit_history[0].text, "Routed message via merged chat");

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
    assert!(search_res.items.iter().any(|m| m["id"] == *sent_msg_id));

    let media_res = client.search_messages("", Some("any")).await;
    assert!(!media_res.items.is_empty() && media_res.items.iter().all(|m| m["attachments"].is_array()));
    let voice =
        media_res.items.iter().flat_map(|m| m["attachments"].as_array().unwrap()).find(|a| a["isVoiceNote"] == true);
    let voice = voice.unwrap();
    assert_eq!(voice["transcription"]["engine"], "openai-whisper", "Desktop's `Transcription`: {voice}");
    assert!(voice["transcription"]["transcription"].is_string() && voice["transcription"]["language"] == "en");

    let unified = client.unified_search("rust").await;
    assert!(
        !unified["results"]["chats"].as_array().unwrap().is_empty() && unified["results"]["in_groups"] == json!([])
    );
    let people = client.unified_search("eric").await;
    assert_eq!(people["results"]["in_groups"][0]["title"], "#desktop-release", "groups match by participant");

    let settings_hits = search_settings(&store.read(), "notifications");
    assert_eq!(settings_hits[0], SettingsHit::Section(SettingsTab::Notifications, true));
    assert!(settings_hits.iter().any(|h| matches!(h, SettingsHit::Pref(_, r) if r.key == "groupNotifications")));
    assert_eq!(settings_hits.last(), Some(&SettingsHit::Section(SettingsTab::About, false)));
    let emoji = search_settings(&store.read(), "emoji autocomplete");
    assert!(
        matches!(emoji[..2], [SettingsHit::Section(SettingsTab::General, true), SettingsHit::Pref(_, r)] if r.key == "emojiAutocomplete")
    );
    assert_eq!(search_settings(&store.read(), "whats")[1], SettingsHit::Network("WhatsApp"));
    assert!(search_settings(&store.read(), " ").is_empty());

    assert_eq!(client.delete_message("!im_sarah:beeper.com", &sent_msg_id).await, StatusCode::NO_CONTENT);
    let search_after_del = client.search_messages("edited", None).await;
    assert!(!search_after_del.items.iter().any(|m| m["id"] == *sent_msg_id));

    let unmerge_chat = store.write().patch_chat(&merged_chat_id, false, None, Some(None)).unwrap();
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

    {
        let st = store.read();
        for (c, list) in st.chats.iter().map(|c| (&c.id, st.timeline_indices(&c.id))).filter(|(_, l)| !l.is_empty()) {
            let ids: Vec<&str> = list.iter().map(|&i| st.messages[i].id.as_str()).collect();
            let stored: Vec<String> =
                st.stored("chat_id = ?1 ORDER BY sort_key", [c]).into_iter().map(|m| m.id).collect();
            assert_eq!(ids, stored, "a loaded chat has the stored messages of the chat");
        }
    }
    let mut gui = BeeperGuiApp::new(store.clone());
    assert_eq!(gui.pane.selected_chat_id.as_deref(), Some("!note_self:beeper.com"));
    gui.select_chat("!wa_eric:beeper.com");
    {
        let mut sim = simulator(gui.view_main());
        for label in ["Inbox", "Eric", "Feedback?"] {
            assert!(sim.find(label).is_ok(), "main window shows {label}");
        }
    }
    {
        use iced::keyboard::{key::Named, Event::KeyPressed, Key, Location, Modifiers};
        use iced::widget::text_editor::{Action, Edit};
        let mut sim = simulator(gui.view_main());
        sim.click(iced::widget::Id::new("composer-input")).expect("the composer");
        let enter = |modifiers| {
            iced::Event::Keyboard(KeyPressed {
                key: Key::Named(Named::Enter),
                modified_key: Key::Named(Named::Enter),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: Location::Standard,
                modifiers,
                repeat: false,
                text: None,
            })
        };
        sim.simulate([enter(Modifiers::SHIFT), enter(Modifiers::empty())]);
        let keys: Vec<_> = sim
            .into_messages()
            .filter(|m| matches!(m, UiMessage::ComposerEdit(Action::Edit(_)) | UiMessage::SubmitComposer))
            .collect();
        assert!(
            matches!(keys[..], [UiMessage::ComposerEdit(Action::Edit(Edit::Enter)), UiMessage::SubmitComposer]),
            "Shift Enter adds a line and Enter sends"
        );
    }
    let _ = gui.update(UiMessage::ComposerChanged("make it bold".into()));
    let _ = gui.update(UiMessage::ComposerEdit(iced::widget::text_editor::Action::SelectAll));
    let _ = gui.update(UiMessage::Format(0));
    assert_eq!(gui.pane.composer.text(), "**make it bold**", "the formatting menu wraps the selection");
    let _ = gui.update(UiMessage::Format(0));
    assert_eq!(gui.pane.composer.text(), "make it bold", "pressing it again removes the marks");
    let _ = gui.update(UiMessage::ComposerChanged(String::new()));
    {
        let mut sim = simulator(gui.view_main());
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
    let _ = gui.update(UiMessage::NewChat);
    {
        let mut sim = simulator(gui.view_main());
        for label in ["New Chat", "All accounts", "New Group", "Join a Matrix room", "In your contacts"] {
            assert!(sim.find(label).is_ok(), "new chat panel shows {label}");
        }
    }
    let prev = gui.pane.selected_chat_id.clone().expect("a chat is selected");
    let _ = gui.update(UiMessage::NewChat);
    let ids: Vec<u32> = gui.new_chats.iter().map(|d| d.id).collect();
    assert_eq!((ids, gui.creating_chat), (vec![1, 0], Some(1)), "compose adds a new chat");
    let _ = gui.update(UiMessage::DeleteNewChat(1));
    assert_eq!(gui.creating_chat, Some(0), "deleting a new chat selects the next row");
    let _ = gui.update(UiMessage::DeleteNewChat(0));
    assert!(gui.new_chats.is_empty() && gui.creating_chat.is_none() && gui.pane.selected_chat_id.is_some());
    let _ = gui.update(UiMessage::SelectChat(prev));
    let _ = gui.update(UiMessage::ComposerChanged("Hello from **Iced** Simulator E2E!".to_string()));
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
        let st = store.read();
        assert!(st.settings.on("showMessagePreviews") && st.settings.pref("timeFormat") == 0);
    }
    let _ = gui.update(UiMessage::SetPref("timeFormat", 1));
    let _ = gui.update(UiMessage::SetPref("showMessagePreviews", 0));
    for (step, zoom) in [(1, 1.1), (1, 1.2), (-1, 1.1), (0, 1.0), (-1, 0.9)] {
        let _ = gui.update(UiMessage::Zoom(step));
        assert_eq!(gui.zoom, zoom);
    }
    for _ in 0..9 {
        let _ = gui.update(UiMessage::Zoom(-1));
    }
    assert_eq!(gui.zoom, 0.4, "Desktop zooms out to 40%");
    let _ = gui.update(UiMessage::ZoomDrag(4.0));
    assert_eq!(gui.zoom, 0.4, "the slider zooms on release");
    let _ = gui.update(UiMessage::ZoomRelease);
    assert_eq!(gui.zoom, 1.4);
    let _ = gui.update(UiMessage::Zoom(0));
    {
        let st = store.read();
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
        let n = beeper_ui::login::FREE_ACCOUNTS - store.read().bridge_accounts.len();
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
    let _ = gui.update(UiMessage::SelectSettingsTab(SettingsTab::Hotkeys));
    assert!(simulator(gui.view_settings()).find("Ctrl Shift Alt R").is_ok(), "Reload App shows Desktop's keys");
    assert!(simulator(gui.view_settings()).find("Record Shortcut").is_err());
    let _ = gui.update(UiMessage::ToggleHotkey("Filter: DMs"));
    assert!(simulator(gui.view_settings()).find("Record Shortcut").is_ok(), "an enabled hotkey without keys");
    let _ = gui.update(UiMessage::RevertHotkeys);
    let mods = iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::ALT;
    let rail = |d: &str| UiMessage::KeyPressed(iced::keyboard::Key::Character(d.into()), mods);
    let _ = gui.update(rail("2"));
    assert_eq!(gui.active_tab, InboxTab::Archived, "Ctrl Alt 2 opens Archive");
    let _ = gui.update(rail("7"));
    assert_eq!(
        gui.selected_account_filter.as_deref(),
        Some("hungryserv"),
        "Ctrl Alt 7 opens the first account after the six views without the label nudge"
    );
    let _ = gui.update(rail("1"));
    assert!(gui.active_tab == InboxTab::AllChats && gui.selected_account_filter.is_none());
    {
        use iced::keyboard::{key::Named, Key, Modifiers};
        let ids: Vec<String> = gui.visible_chats(&store.read()).iter().take(3).map(|c| c.id.clone()).collect();
        for id in &ids {
            let _ = gui.update(UiMessage::SelectChat(id.clone()));
        }
        let selected = |gui: &BeeperGuiApp| gui.pane.selected_chat_id.clone().unwrap();
        let key = |k, mods| UiMessage::KeyPressed(Key::Named(k), mods);
        let _ = gui.update(key(Named::ArrowLeft, Modifiers::CTRL | Modifiers::ALT));
        assert_eq!(selected(&gui), ids[1], "History: Previous Chat opens the chat before");
        let _ = gui.update(key(Named::ArrowRight, Modifiers::CTRL | Modifiers::ALT));
        assert_eq!(selected(&gui), ids[2], "History: Next Chat goes back");
        let _ = gui.update(UiMessage::Modifiers(Modifiers::CTRL));
        let _ = gui.update(key(Named::Tab, Modifiers::CTRL));
        let _ = gui.update(key(Named::Tab, Modifiers::CTRL));
        assert_eq!(selected(&gui), ids[0], "Ctrl Tab steps through the history without reordering it");
        let _ = gui.update(UiMessage::Modifiers(Modifiers::CTRL | Modifiers::SHIFT));
        let _ = gui.update(key(Named::Tab, Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(selected(&gui), ids[1], "Shift reverses the cycle");
        let _ = gui.update(UiMessage::Modifiers(Modifiers::empty()));
        let _ = gui.update(UiMessage::Modifiers(Modifiers::CTRL));
        let _ = gui.update(key(Named::Tab, Modifiers::CTRL));
        assert_eq!(selected(&gui), ids[2], "releasing Ctrl moves the chat to the front");
        let _ = gui.update(UiMessage::Modifiers(Modifiers::empty()));
        let _ = gui.update(UiMessage::KeyPressed(Key::Character("j".into()), Modifiers::CTRL));
        assert!(gui.show_command_palette && gui.cmd_bar_commands_mode, "Ctrl J opens the commands");
        for title in ["Archive all chats", "Toggle sidebar", "Media viewer", "Report a problem"] {
            let _ = gui.update(UiMessage::CommandQueryChanged(title.to_lowercase()));
            let mut sim = simulator(gui.view_main());
            assert!(sim.find(title).is_ok(), "the commands have {title}");
            assert!(title != "Media viewer" || sim.find("Window").is_ok(), "the commands have their category");
        }
        let _ = gui.update(UiMessage::CommandQueryChanged("toggle sidebar".into()));
        let _ = gui.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
        assert!(gui.sidebar_hidden && !gui.show_command_palette, "Enter runs the best match and closes the bar");
        let _ = gui.update(UiMessage::RunHotkey("Toggle Sidebar"));
        let _ = gui.update(UiMessage::KeyPressed(Key::Character("j".into()), Modifiers::CTRL));
        let _ = gui.update(UiMessage::CommandQueryChanged("magnify".into()));
        let _ = gui.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::empty()));
        assert_eq!(gui.cmd_index, 1, "Down highlights the next command");
        let _ = gui.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
        assert!(gui.zoom < 1.0, "a keyword finds Zoom out");
        let _ = gui.update(UiMessage::RunHotkey("Reset Zoom"));
        let _ = gui.update(UiMessage::KeyPressed(Key::Character("/".into()), Modifiers::CTRL));
        assert!(simulator(gui.view_main()).find("Unselect Chat / Select First Chat").is_ok(), "Ctrl / lists the keys");
        let _ = gui.update(UiMessage::HotkeysModal(Some("zoom".into())));
        assert!(simulator(gui.view_main()).find("Reset Zoom").is_ok());
        assert!(simulator(gui.view_main()).find("Command Bar").is_err(), "the search filters the list");
        let _ = gui.update(UiMessage::HotkeysModal(Some("qqq".into())));
        assert!(simulator(gui.view_main()).find("No results").is_ok());
        let _ = gui.update(UiMessage::CloseModals);
        assert!(gui.hotkeys_modal.is_none(), "Escape closes the list");
        let _ = gui.update(UiMessage::KeyPressed(Key::Character("s".into()), Modifiers::CTRL | Modifiers::ALT));
        assert!(gui.sidebar_hidden, "Ctrl Alt S hides the sidebar");
        let hidden = std::fs::read_to_string(store.read().config.data_dir.join("sidebar-hidden")).unwrap();
        assert_eq!(hidden, "true", "the hidden sidebar stays hidden after a restart");
        let width = gui.sidebar_width;
        let _ = gui.update(UiMessage::DragMoved(iced::Point::new(200.0, 0.0)));
        assert!(gui.sidebar_hidden && gui.sidebar_width == width, "a drag below Desktop's minimum keeps it hidden");
        let _ = gui.update(UiMessage::DragMoved(iced::Point::new(400.0, 0.0)));
        assert!(!gui.sidebar_hidden && gui.sidebar_width == 330.0, "a wider drag shows it again");
        let main = iced::window::Id::unique();
        gui.main_window = Some(main);
        let _ = gui.update(UiMessage::WindowResized(main, iced::Size::new(500.0, 800.0)));
        assert!(gui.sidebar_hidden, "a window narrower than 600 hides the sidebar");
        let _ = gui.update(UiMessage::WindowResized(main, iced::Size::new(1264.0, 893.0)));
        assert!(!gui.sidebar_hidden, "a wider window shows it again");
        gui.main_window = None;
    }
    {
        let mut st = store.write();
        let account = |net: &str| st.accounts.iter().find(|a| a.account_id.contains(net)).unwrap().account_id.clone();
        let space = |id: &str, account_id, name: &str| beeper_core::store::NetworkSpace {
            id: id.into(),
            account_id,
            name: name.into(),
        };
        let (signal, whatsapp) = (account("signal"), account("whatsapp"));
        st.network_spaces.extend([space("g1", signal, "Guild"), space("c1", whatsapp, "Community")]);
    }
    let _ = gui.update(UiMessage::SelectAccount(Some("c1".into())));
    assert!(simulator(gui.view_main()).find("Community").is_err(), "WhatsApp Communities are not spaces by default");
    let _ = gui.update(UiMessage::SetPref("whatsappCommunities", 1));
    assert!(
        simulator(gui.view_main()).find("Community (WhatsApp)").is_ok(),
        "the setting shows WhatsApp Communities as spaces"
    );
    let _ = gui.update(UiMessage::SetPref("whatsappCommunities", 0));
    let _ = gui.update(UiMessage::SelectAccount(Some("g1".into())));
    {
        let mut sim = simulator(gui.view_main());
        assert!(sim.find("Guild (Signal)").is_ok(), "the header shows the server name and its network");
        assert!(sim.find("You have no messages").is_ok(), "a server without chats is empty");
    }
    {
        let mut st = store.write();
        let i = st.chats.iter().position(|c| c.account_id.contains("signal")).expect("a chat of the account");
        st.chats[i].network_id = Some("g1".into());
        let shown: Vec<_> = gui.visible_chats(&st).iter().map(|c| c.id.clone()).collect();
        assert_eq!(shown, [st.chats[i].id.clone()], "the server shows only its chats");
        gui.selected_account_filter = Some(st.chats[i].account_id.clone());
        assert!(!gui.visible_chats(&st).iter().any(|c| c.id == st.chats[i].id), "the account omits the server chats");
    }
    {
        let cid = store.read().chats[0].id.clone();
        let was = store.read().chats[0].is_muted == Some(true);
        take_outbox(&mut store.write());
        let _ = gui.update(UiMessage::ToggleFlag(cid.clone(), ChatFlag::Muted));
        let mut st = store.write();
        assert_eq!(st.chats[0].is_muted, Some(!was), "the mute toggle changes the chat");
        let queued = matches!(take_outbox(&mut st).as_slice(), [(c, Outgoing::Flag(ChatFlag::Muted, on))] if *c == cid && *on != was);
        assert!(queued, "the mute toggle goes to the homeserver");
        st.chats[0].is_pinned = Some(true);
        assert!(!st.set_chat_flag(0, ChatFlag::Archived, None), "pinned chats can't be archived");
        assert!(st.outbox.is_empty());
        let read = st.chats[0].unread_count == 0 && st.chats[0].is_marked_unread != Some(true);
        drop(st);
        let _ = gui.update(UiMessage::OpenChatMenu(cid.clone()));
        assert!(gui.chat_menu.as_ref().is_some_and(|(c, _)| *c == cid), "right-click opens the chat menu");
        let _ = gui.update(UiMessage::ToggleRead(cid.clone()));
        assert!(gui.chat_menu.is_none(), "a menu action closes the menu");
        let c = store.read().chats[0].clone();
        assert_eq!(c.unread_count == 0 && c.is_marked_unread != Some(true), !read, "the read toggle flips the chat");
        let _ = gui.update(UiMessage::OpenChatMenu(cid.clone()));
        let _ = gui.update(UiMessage::ChatSub(Some(1)));
        assert!(simulator(gui.view_main()).find("In an Hour").is_ok(), "Remind Me opens the presets");
        let at = beeper_core::config::now_unix_millis() + 3_600_000;
        take_outbox(&mut store.write());
        let _ = gui.update(UiMessage::SetReminder(cid.clone(), Some(at)));
        let _ = gui.update(UiMessage::SetReminder(cid.clone(), None));
        {
            let mut st = store.write();
            let sent: Vec<_> = take_outbox(&mut st)
                .into_iter()
                .map(|(_, o)| match o {
                    Outgoing::Reminder(r) => r.map(|r| r.remind_at_ms),
                    _ => panic!("only reminders are queued"),
                })
                .collect();
            assert_eq!(sent, [Some(at), None], "a reminder is set and dismissed on the homeserver");
            assert!(st.chats[0].reminder.is_none() && gui.chat_menu.is_none());
            for (i, at) in [(0, 2), (1, 1)] {
                st.set_reminder(i, Some(ChatReminder { remind_at_ms: at, dismiss_on_incoming_message: true }));
            }
        }
        let _ = gui.update(UiMessage::SelectTab(InboxTab::Reminders));
        let order = |gui: &BeeperGuiApp| {
            let st = store.read();
            gui.visible_chats(&st).iter().map(|c| c.reminder.as_ref().unwrap().remind_at_ms).collect::<Vec<_>>()
        };
        assert_eq!(order(&gui), [1, 2], "reminders come by least time remaining");
        let _ = gui.update(UiMessage::ReminderMenu);
        let _ = gui.update(UiMessage::OrderReminders(1));
        assert_eq!(order(&gui), [2, 1], "the display options change the order");
        assert_eq!(store.read().settings.reminders_order, 1);
        let _ = gui.update(UiMessage::OrderReminders(0));
        let _ = gui.update(UiMessage::Ask(Some(1)));
        assert!(simulator(gui.view_main()).find("Dismiss All Reminders?").is_ok());
        let _ = gui.update(UiMessage::DismissAllReminders);
        assert!(order(&gui).is_empty() && gui.confirm.is_none(), "dismiss all clears every reminder");
        let _ = gui.update(UiMessage::SelectTab(InboxTab::AllChats));
        {
            let mut st = store.write();
            st.chats[1].is_pinned = Some(false);
            assert!(st.set_chat_flag(1, ChatFlag::Archived, Some(true)));
            st.set_reminder(1, Some(ChatReminder { remind_at_ms: 1, dismiss_on_incoming_message: true }));
        }
        let _ = gui.update(UiMessage::CheckReminders);
        let other = store.read().chats[1].id.clone();
        assert_eq!(store.read().chats[1].is_archived, Some(false), "a due reminder unarchives the chat");
        assert!(simulator(gui.view_main()).find("Reminder").is_ok(), "the chat list shows the Reminder badge");
        gui.select_chat(&other);
        gui.select_chat(&cid);
        assert!(store.read().chats[1].reminder.is_none(), "leaving a reminded chat dismisses the reminder");
        let _ = gui.update(UiMessage::SearchRoom(cid.clone()));
        assert!(gui.show_command_palette && gui.search_room.as_deref() == Some(cid.as_str()));
        let _ = gui.update(UiMessage::ToggleCommandPalette);
        assert!(gui.show_command_palette && gui.search_room.is_none(), "Ctrl K widens a chat search");
        let mid = store.read().messages.iter().find(|m| m.chat_id == cid).expect("chat has messages").id.clone();
        let _ = gui.update(UiMessage::JumpTo(cid.clone(), mid.clone()));
        assert!(!gui.show_command_palette && gui.pane.selected_chat_id.as_deref() == Some(cid.as_str()));
        assert!(gui.flash.as_ref().is_some_and(|(id, _)| *id == mid), "a message hit flashes the message");
        let _ = gui.update(UiMessage::ToggleCommandPalette);
        let picks = [SearchPick::Sent(true), SearchPick::Sent(false), SearchPick::Period(2), SearchPick::Period(2)];
        for pick in picks.into_iter().chain([SearchPick::Media(0), SearchPick::Media(3), SearchPick::Media(0)]) {
            let _ = gui.update(UiMessage::PickSearchFilter(pick));
        }
        assert_eq!((gui.search_filters.sent, gui.search_filters.period), (Some(false), None), "a second pick clears");
        assert_eq!(gui.search_filters.media, [3], "media kinds toggle one by one");
        let _ = gui.update(UiMessage::ToggleCommandPalette);
        let _ = gui.update(UiMessage::ToggleCommandPalette);
        assert_eq!(gui.search_filters, SearchFilters::default(), "a new search starts without filters");
        let _ = gui.update(UiMessage::ToggleCommandPalette);
        let other = store.read().chats[1].id.clone();
        let _ = gui.update(UiMessage::HoverChat(cid.clone(), true));
        let _ = gui.update(UiMessage::HoverChat(other.clone(), false));
        assert_eq!(gui.hovered_chat.as_deref(), Some(cid.as_str()), "leaving another row keeps the hover");
        let _ = gui.update(UiMessage::HoverChat(cid.clone(), false));
        assert!(gui.hovered_chat.is_none());
        for _ in 0..2 {
            let _ = gui.update(UiMessage::ComposerChanged("draft".into()));
            let _ = gui.update(UiMessage::SelectChat(other.clone()));
            let _ = gui.update(UiMessage::SelectChat(cid.clone()));
            assert_eq!(gui.pane.composer.text(), "draft", "a chat keeps its draft");
        }
        let _ = gui.update(UiMessage::DeleteDraft(cid.clone()));
        let c = store.read().chats[0].clone();
        assert!(c.draft.is_none() && gui.pane.composer.is_empty(), "the hover action deletes the draft");
        let note = beeper_core::store::NOTE_CHAT_ID;
        let left = store.read().chats.last().expect("a chat").id.clone();
        assert!(left != note && left != "!wa_eric:beeper.com");
        gui.select_chat(&left);
        store.write().remove_chat(&left);
        let _ = simulator(gui.view_main());
        let _ = gui.update(UiMessage::StoreChanged);
        let st = store.read();
        assert!(st.resolve_chat_idx(&left).is_none() && st.timeline_indices(&left).is_empty());
        assert!(
            st.messages.iter().filter(|m| st.loaded(&m.id).is_some()).all(|m| m.chat_id != left),
            "a left chat loses its messages"
        );
        assert_eq!(gui.pane.selected_chat_id.as_deref(), Some(note), "leaving the open chat opens Note to self");
        drop(st);
        let doomed = {
            let mut st = store.write();
            let c = st.chats.last_mut().expect("a chat");
            assert!(c.id != note && c.id != "!wa_eric:beeper.com");
            c.delete_modes = DELETE_FOR_ME | DELETE_FOR_EVERYONE;
            c.id.clone()
        };
        let _ = gui.update(UiMessage::OpenChatMenu(doomed.clone()));
        let _ = gui.update(UiMessage::AskDeleteChat(doomed.clone()));
        assert!(gui.chat_menu.is_none() && simulator(gui.view_main()).find("Delete for Everyone").is_ok());
        take_outbox(&mut store.write());
        let _ = gui.update(UiMessage::DeleteChat(doomed.clone(), true));
        let mut st = store.write();
        let sent = matches!(take_outbox(&mut st).as_slice(), [(c, Outgoing::DeleteChat { for_everyone: true })] if *c == doomed);
        assert!(sent && st.resolve_chat_idx(&doomed).is_none(), "Delete Chat asks the bridge and drops the chat");
        let picks: Vec<String> =
            st.chats.iter().filter(|c| c.is_muted != Some(true)).take(2).map(|c| c.id.clone()).collect();
        drop(st);
        let _ = gui.update(UiMessage::BeginSelecting(picks[0].clone()));
        let _ = gui.update(UiMessage::ToggleSelected(picks[1].clone()));
        assert!(simulator(gui.view_main()).find("2 chats selected").is_ok(), "Begin Selecting shows the selection bar");
        let _ = gui.update(UiMessage::SelectionSet(ChatFlag::Muted, true));
        let st = store.read();
        let muted = picks.iter().all(|id| st.chats[st.resolve_chat_idx(id).unwrap()].is_muted == Some(true));
        assert!(muted && gui.selection.is_empty(), "a selection action applies to every chat and ends the selection");
        drop(st);
        for id in &picks {
            let _ = gui.update(UiMessage::ToggleFlag(id.clone(), ChatFlag::Muted));
        }
        take_outbox(&mut store.write());
    }
    let _ = gui.update(rail("1"));
    let _ = gui.update(UiMessage::SelectTheme(ThemeChoice::System));
    for (mode, dark) in [(iced::theme::Mode::Dark, true), (iced::theme::Mode::Light, false)] {
        let _ = gui.update(UiMessage::SystemTheme(mode));
        assert_eq!(gui.dark, dark);
    }
    let _ = gui.update(UiMessage::SettingsQueryChanged("tray".to_string()));
    let _ = simulator(gui.view_settings());
    let hit = search_settings(&store.read(), "tray")[1];
    let _ = gui.update(UiMessage::PickSetting(hit));
    assert_eq!(gui.active_settings_tab, SettingsTab::Appearance);
    assert!(simulator(gui.view_settings()).find("Show tray icon").is_ok());
    let _ = gui.update(UiMessage::SetPref("appLanguage", 2));
    assert!(simulator(gui.view_main()).find("Inbox").is_ok(), "the language changes after a relaunch");
    assert!(simulator(gui.view_main()).find("App language will change after relaunch").is_ok());
    let _ = gui.update(UiMessage::SetPref("appLanguage", 1));
    let _ = gui.update(UiMessage::DismissBanner);
    let _ = gui.update(UiMessage::CloseModals);

    let final_wa_msgs = client.list_messages("!wa_eric:beeper.com").await;
    assert!(final_wa_msgs
        .items
        .iter()
        .any(|m| m["text"] == "Hello from **Iced** Simulator E2E!" && m.get("html").is_none()));
    let html = Some("Hello from <strong>Iced</strong> Simulator E2E!");
    assert!(store.read().messages.iter().any(|m| m.html.as_deref() == html));

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
        let emu_png = temp_dir("iced-shot").join("dark.png");
        write_rgba_png(&emu_png, shot.size.width, shot.size.height, &shot.rgba);
        assert!(emu_png.exists());
    })
    .join()
    .unwrap();

    #[cfg(target_os = "linux")]
    {
        let bin_exe = env!("CARGO_BIN_EXE_beeper-rs");
        let shots = temp_dir("gui-shots");
        let png = |name| shots.join(name).with_extension("png").into_os_string().into_string().unwrap();
        let (wl_png, wl_settings_png, x_png) = (png("wayland_window"), png("wayland_settings"), png("xvfb_window"));
        let run_gui = |name: &str, hide: &str, envs: &[(&str, &str)]| {
            let data = temp_dir(&format!("BeeperRs_gui_{name}"));
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
            let mut gui = std::process::Command::new(bin_exe)
                .args(["--data-dir", data.to_str().unwrap(), "--port", &port])
                .args(envs.iter().any(|e| e.0 == "WAYLAND_DISPLAY").then_some("--demo"))
                .env_remove(hide)
                .env_remove("ICED_BACKEND")
                .envs(envs.iter().copied())
                .spawn()
                .expect("run beeper-rs GUI");
            let maps = format!("/proc/{}/maps", gui.id());
            let status = loop {
                let libs = std::fs::read_to_string(&maps).expect("read the memory map of the GUI");
                assert!(!libs.contains("libvulkan"), "the GUI loads Vulkan on a display without a GPU");
                match gui.try_wait().expect("wait for the GUI") {
                    Some(status) => break status,
                    None => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            };
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
            "wayland",
            "DISPLAY",
            &[
                xdg,
                ("WAYLAND_DISPLAY", "beeper-wl-e2e"),
                ("BEEPER_SCREENSHOT_EXIT", &wl_png),
                ("BEEPER_SCREENSHOT_SETTINGS", &wl_settings_png),
            ],
        );
        stop(weston);
        assert!(wl_status.success(), "beeper-rs GUI on headless Wayland exited cleanly");

        // Xvfb takes a free display and writes its number when it accepts clients.
        let mut xvfb = std::process::Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", "1440x900x24", "-ac", "-nolisten", "tcp"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn Xvfb");
        let mut display = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(xvfb.stdout.take().unwrap()), &mut display).unwrap();
        let display = format!(":{}", display.trim());
        let x_status = run_gui("x11", "WAYLAND_DISPLAY", &[("DISPLAY", &display), ("BEEPER_SCREENSHOT_EXIT", &x_png)]);
        stop(xvfb);
        assert!(x_status.success(), "beeper-rs GUI on Xvfb {display} exited cleanly");

        for (png, min) in [(wl_png, 25_000), (wl_settings_png, 15_000), (x_png, 15_000)] {
            let len = std::fs::metadata(&png).expect("screenshot PNG is written").len();
            assert!(len > min, "{png} has {len} bytes");
        }
        std::fs::remove_dir_all(wl_runtime).expect("remove test dir");
    }
}

/// Bad requests get Desktop's errors, and the server answers the next request.
#[tokio::test]
async fn e2e_api_rejects_bad_input_and_keeps_serving() {
    let (data_dir, _, _, client) = spawn_e2e_beeper_instance("bad_input", None).await;
    let media = path_to_file_url(&data_dir.join("media"));
    let issue = |path: &str| json!({ "code": "custom", "message": "'single' chat must have exactly one participantID.", "path": [path] });
    let too_small = json!({
        "code": "too_small", "minimum": 1, "type": "array", "inclusive": true, "exact": false,
        "message": "Array must contain at least 1 element(s)", "path": ["participantIDs"]
    });
    let invalid = |issues: Value| json!({ "message": "Invalid input", "code": "VALIDATION_ERROR", "details": { "issues": issues } });
    let not_found = |m: &str| json!({ "message": m, "code": "NOT_FOUND" });
    let cases = [
        (
            "POST",
            "/v1/chats".to_string(),
            Some(json!({ "accountID": "acc_whatsapp", "type": "single", "participantIDs": [] })),
            400,
            invalid(json!([too_small, issue("participantIDs")])),
        ),
        (
            "POST",
            "/v1/chats".to_string(),
            Some(json!({ "accountID": "acc_whatsapp", "type": "group" })),
            400,
            invalid(json!([{
                "code": "invalid_type", "expected": "array", "received": "undefined", "path": ["participantIDs"],
                "message": "Required"
            }])),
        ),
        (
            "DELETE",
            "/v1/chats/!tg_rust:beeper.com/messages/nope/reactions/x".to_string(),
            None,
            404,
            not_found("Message not found"),
        ),
        (
            "DELETE",
            "/v1/chats/!tg_pebble:beeper.com/messages/msg_1004".to_string(),
            None,
            404,
            not_found("Message not found"),
        ),
        (
            "GET",
            "/v1/chats/!tg_pebble:beeper.com/messages/msg_1004".to_string(),
            None,
            404,
            not_found("Message not found"),
        ),
        (
            "PATCH",
            "/v1/chats/!tg_rust:beeper.com".to_string(),
            Some(json!({ "merge": { "chatIDs": ["!nope"] } })),
            404,
            not_found("Chat not found"),
        ),
        (
            "POST",
            "/v1/chats".to_string(),
            Some(json!({ "mode": "merge", "merge": { "chatIDs": ["!tg_rust:beeper.com"] } })),
            400,
            json!({ "message": "title is required", "code": "INVALID_INPUT" }),
        ),
        (
            "GET",
            beeper_api::http::query_string("/v1/assets/serve", &[("url", &media)]),
            None,
            500,
            json!({ "message": "EISDIR: illegal operation on a directory, read", "code": "UNKNOWN_ERROR" }),
        ),
    ];
    for (method, path, body, status, want) in cases {
        assert_eq!(client.request(method, &path, body).await, (StatusCode(status), want), "{method} {path}");
        assert_eq!(client.info().await.server.status, "running", "the server answers after {method} {path}");
    }
    let merged = client.merge_chats("Merged", vec!["!tg_rust:beeper.com".into()], None).await;
    let path = format!("/v1/chats/{}/messages/msg_1004", merged["chatID"].as_str().unwrap());
    let (status, msg) = client.request("GET", &path, None).await;
    assert_eq!(
        (status, &msg["chatID"]),
        (StatusCode::OK, &json!("!tg_rust:beeper.com")),
        "a merged chat has the messages of its members"
    );

    let outside = std::env::temp_dir().join("beeper_bad_input_outside.txt");
    std::fs::write(&outside, b"secret").unwrap();
    let uploaded = client.upload_asset_base64("a.txt", "text/plain", b"attachment").await;
    let (_, refused) = client.request("POST", "/v1/assets/download", Some(json!({ "url": uploaded["srcURL"] }))).await;
    assert_eq!(refused, json!({ "error": "URL must be mxc:// or localmxc://" }), "an upload has a file URL");
    let denied = br#"{"message":"Access denied: Path must be within allowed attachment directories","code":"SERVE_ASSET_ERROR"}"#;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut files = vec![
        (uploaded["srcURL"].as_str().unwrap().to_string(), 200, &b"attachment"[..]),
        (path_to_file_url(&outside), 403, denied),
        (path_to_file_url(&data_dir.join("beeper-rs.db")), 403, denied),
        (path_to_file_url(&data_dir.join("nope")), 404, br#"{"message":"File not found","code":"SERVE_ASSET_ERROR"}"#),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, data_dir.join("media/link.txt")).unwrap();
        files.push((path_to_file_url(&data_dir.join("media/link.txt")), 403, denied));
    }
    for (url, status, body) in files {
        assert_eq!(client.serve_asset_range(&url, None).await, (StatusCode(status), body.to_vec()), "serve {url}");
    }
    // The server keeps its data folder open, and Windows cannot remove an open file. The next run removes the folder.
    std::fs::remove_file(outside).unwrap();
}
