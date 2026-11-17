use beeper_core::config::CentralConfig;
use beeper_core::models::{
    AppVerification, ThemeChoice, VerificationDevice, VerificationQr, VerificationSas, VerificationSasEmoji,
};
use beeper_core::store::{BeeperStore, StoreLock};
use beeper_ui::{
    login::{Login, LoginMsg, Screen},
    style::{ensure_fonts_loaded, fw},
    BeeperGuiApp, El, SettingsTab, UiMessage,
};
use iced::{font::Weight, Size, Theme};
use iced_test::simulator::Simulator;
use std::path::{Path, PathBuf};

fn shot(out: &Path, name: &str, size: (f32, f32), theme: &Theme, el: El<'_>) {
    ensure_fonts_loaded();
    let settings = iced::Settings { default_font: fw(Weight::Normal), ..iced::Settings::default() };
    let path = out.join(format!("{name}-tiny-skia.png"));
    if path.exists() {
        std::fs::remove_file(&path).expect("remove old shot");
    }
    let mut sim = Simulator::with_size(settings, Size::new(size.0, size.1), el);
    assert!(sim.snapshot(theme).expect("snapshot").matches_image(out.join(name)).expect("write shot"));
    println!("{}", path.display());
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "/tmp/beeper-rs-shots".into()));
    let dark = args.next().as_deref() == Some("dark");
    std::fs::create_dir_all(&out).expect("create shot dir");
    let real = args.next().map(PathBuf::from);
    let cfg = match &real {
        Some(dir) => CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23390, None),
        None => beeper_core::testing::demo_config(if dark { "shots-dark" } else { "shots-light" }),
    };
    let data = cfg.data_dir.clone();
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let mut app = BeeperGuiApp::new(store);
    let (choice, theme) = if dark { (ThemeChoice::Dark, Theme::Dark) } else { (ThemeChoice::Light, Theme::Light) };
    let _ = app.update(UiMessage::Settings(beeper_ui::settings::Msg::SelectTheme(choice)));
    shot(&out, "main", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::ChatList(beeper_ui::chat_list::Msg::FilterButton));
    shot(&out, "main_unread", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::ChatList(beeper_ui::chat_list::Msg::ToggleFilterMenu));
    shot(&out, "main_filter_menu", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::ChatList(beeper_ui::chat_list::Msg::PickFilter(0)));
    let _ = app.update(UiMessage::CommandBar(beeper_ui::commands::Msg::TogglePalette));
    shot(&out, "main_search", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::NewChat(beeper_ui::new_chat::Msg::Open));
    shot(&out, "main_new_chat", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::NewChat(beeper_ui::new_chat::Msg::Delete(0)));
    let dm = {
        let mut st = app.store.write();
        let peer =
            |c: &beeper_core::models::Chat| c.participants.items.iter().find(|u| u.is_self != Some(true)).cloned();
        let c = st.chats.iter().find(|c| c.chat_type == "single" && peer(c).is_some()).expect("a DM");
        let (id, user) = (c.id.clone(), peer(c).expect("a peer").id);
        st.typing.insert(id.clone(), vec![user]);
        id
    };
    app.select_chat(&dm);
    shot(&out, "main_typing", (1200.0, 800.0), &theme, app.view_main());
    let marks = "**bold** _italic_ ~~gone~~ `code` [link](https://beeper.com)\n> quote\n- item";
    let _ = app.update(UiMessage::Composer(beeper_ui::composer::Msg::Changed(marks.into())));
    shot(&out, "main_composer_marks", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::Composer(beeper_ui::composer::Msg::Changed(String::new())));
    app.store.write().typing.clear();
    app.store.write().recent_emoji = vec![("👍".into(), 3), ("joy".into(), 2), ("❤️".into(), 1)];
    let _ = app.update(UiMessage::Emoji(beeper_ui::emoji::EmojiMsg::Pointer(0, true)));
    shot(&out, "main_emoji", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::ChatList(beeper_ui::chat_list::Msg::BeginSelecting(dm.clone())));
    shot(&out, "main_selecting", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::CloseModals);
    app.select_chat("!wa_eric:beeper.com");
    shot(&out, "main_voice", (1200.0, 800.0), &theme, app.view_main());
    app.select_chat("!sl_infra:beeper.com");
    shot(&out, "main_threads", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::Thread(beeper_ui::threads::Msg::ShowUnread));
    shot(&out, "main_unread_threads", (1200.0, 800.0), &theme, app.view_main());
    let _ =
        app.update(UiMessage::Thread(beeper_ui::threads::Msg::Open("!sl_infra:beeper.com".into(), "msg_1007".into())));
    shot(&out, "main_unread_thread", (1200.0, 800.0), &theme, app.view_main());
    app.select_chat("!sl_infra:beeper.com");
    let _ = app.update(UiMessage::Thread(beeper_ui::threads::Msg::Show("msg_1007".into())));
    shot(&out, "main_thread_pane", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::Thread(beeper_ui::threads::Msg::Hide));
    for tab in SettingsTab::ALL {
        let _ = app.update(UiMessage::Settings(beeper_ui::settings::Msg::Open(tab)));
        shot(&out, &format!("settings_{tab:?}").to_lowercase(), (800.0, 740.0), &theme, app.view_settings());
    }
    let _ = app.update(UiMessage::Settings(beeper_ui::settings::Msg::Open(SettingsTab::Devices)));
    let _ = app.update(UiMessage::Ask(Some(0)));
    shot(&out, "settings_logout", (800.0, 740.0), &theme, app.view_settings());
    let _ = app.update(UiMessage::Ask(None));
    {
        let mut st = app.store.write();
        let emojis = [
            ("🐶", "Dog"),
            ("🔑", "Key"),
            ("🎸", "Guitar"),
            ("🚀", "Rocket"),
            ("🌵", "Cactus"),
            ("⏰", "Clock"),
            ("🍕", "Pizza"),
        ];
        let emojis = emojis.map(|(e, d)| VerificationSasEmoji { emoji: e.into(), description: d.into() }).to_vec();
        st.verification = Some(AppVerification {
            id: "shot".into(),
            purpose: "device",
            direction: "outgoing",
            state: "sas_ready",
            methods: vec!["sas"],
            available_actions: vec!["sas.confirm", "cancel"],
            other_user_id: None,
            other_device: Some(VerificationDevice { id: "PHONE".into(), name: Some("Pixel 9".into()) }),
            qr: Some(VerificationQr { data: (0..120u8).collect() }),
            sas: Some(VerificationSas { emojis, decimals: "1234 5678 9012".into() }),
            error: None,
        });
    }
    shot(&out, "verification_sas", (330.0, 480.0), &theme, app.view_verify());
    app.store.write().verification.as_mut().expect("the shot verification").state = "ready";
    shot(&out, "verification_qr", (330.0, 480.0), &theme, app.view_verify());
    for screen in [Screen::Welcome, Screen::Email, Screen::Code, Screen::Username, Screen::Accounts] {
        app.login = Login::default();
        let _ = app.update(UiMessage::Login(LoginMsg::Show(screen)));
        shot(&out, &format!("login_{screen:?}").to_lowercase(), (330.0, 620.0), &theme, app.view_login());
    }
    let _ = app.update(UiMessage::Login(LoginMsg::Show(Screen::Email)));
    let _ = app.update(UiMessage::Login(LoginMsg::SubmitEmail));
    let _ = app.update(UiMessage::Login(LoginMsg::Menu));
    shot(&out, "login_email_toast_menu", (330.0, 620.0), &theme, app.view_login());
    let _ = app.update(UiMessage::Login(LoginMsg::Email("foo".into())));
    let _ = app.update(UiMessage::Login(LoginMsg::SubmitEmail));
    shot(&out, "login_email_invalid", (330.0, 620.0), &theme, app.view_login());
    let _ = app.update(UiMessage::Login(LoginMsg::Email("someone@example.com".into())));
    let _ = app.update(UiMessage::Login(LoginMsg::Done(Ok(beeper_ui::login::Outcome::Requested("r".into())))));
    let _ = app.update(UiMessage::Login(LoginMsg::Code("12".into())));
    shot(&out, "login_code_typed", (330.0, 620.0), &theme, app.view_login());
    let _ = app.update(UiMessage::Login(LoginMsg::Code("123456".into())));
    let refused = beeper_core::cloud::CloudError {
        status: 401,
        message: "Invalid code. Please try again.".into(),
        errcode: None,
        body: None,
    };
    let _ = app.update(UiMessage::Login(LoginMsg::Done(Err(refused))));
    shot(&out, "login_code_error", (330.0, 620.0), &theme, app.view_login());
    let lead = beeper_ui::login::Outcome::Lead("lead".into(), vec!["alex-rs".into(), "alex2".into()]);
    let _ = app.update(UiMessage::Login(LoginMsg::Done(Ok(lead))));
    shot(&out, "login_username_suggested", (330.0, 620.0), &theme, app.view_login());
    let _ = app.update(UiMessage::Login(LoginMsg::Username("supportbot".into())));
    shot(&out, "login_username_refused", (330.0, 620.0), &theme, app.view_login());
    if real.is_none() {
        std::fs::remove_dir_all(data).expect("remove shot data dir");
    }
}
