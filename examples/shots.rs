use beeper_core::{config::CentralConfig, models::ThemeChoice, store::BeeperStore};
use beeper_ui::{
    login::{Login, LoginMsg, Screen},
    style::{fw, ALL_FONTS},
    BeeperGuiApp, El, SettingsTab, UiMessage,
};
use iced::{font::Weight, Size, Theme};
use iced_test::simulator::Simulator;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn shot(out: &Path, name: &str, size: (f32, f32), theme: &Theme, el: El<'_>) {
    let settings = iced::Settings {
        default_font: fw(Weight::Normal),
        fonts: ALL_FONTS.iter().map(|f| (*f).into()).collect(),
        ..iced::Settings::default()
    };
    let path = out.join(format!("{name}-tiny-skia.png"));
    if path.exists() {
        std::fs::remove_file(&path).expect("remove old shot");
    }
    let mut sim = Simulator::with_size(settings, Size::new(size.0, size.1), el);
    assert!(sim.snapshot(theme).expect("snapshot").matches_image(out.join(name)).expect("write shot"));
    println!("{}", path.display());
}

fn main() {
    unsafe { std::env::set_var("ICED_TEST_BACKEND", "tiny-skia") };
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "/tmp/beeper-rs-shots".into()));
    let dark = args.next().as_deref() == Some("dark");
    std::fs::create_dir_all(&out).expect("create shot dir");
    let real = args.next().map(PathBuf::from);
    let data =
        real.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("beeper-rs-shots-{}", std::process::id())));
    let mut cfg = CentralConfig::new(data.clone(), "127.0.0.1".into(), 23390, None);
    cfg.demo = real.is_none();
    let store = Arc::new(RwLock::new(BeeperStore::from_config(cfg)));
    let mut app = BeeperGuiApp::new(store, false);
    let (choice, theme) = if dark { (ThemeChoice::Dark, Theme::Dark) } else { (ThemeChoice::Light, Theme::Light) };
    let _ = app.update(UiMessage::SelectTheme(choice));
    shot(&out, "main", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::FilterButton);
    shot(&out, "main_unread", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::ToggleFilterMenu);
    shot(&out, "main_filter_menu", (1200.0, 800.0), &theme, app.view_main());
    let _ = app.update(UiMessage::PickFilter(0));
    for tab in SettingsTab::ALL {
        let _ = app.update(UiMessage::OpenSettings(tab));
        shot(&out, &format!("settings_{tab:?}").to_lowercase(), (800.0, 740.0), &theme, app.view_settings());
    }
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Devices));
    let _ = app.update(UiMessage::AskLogout(true));
    shot(&out, "settings_logout", (800.0, 740.0), &theme, app.view_settings());
    let _ = app.update(UiMessage::AskLogout(false));
    for screen in [Screen::Welcome, Screen::Email, Screen::Code, Screen::Username, Screen::Accounts] {
        app.login = Login::default();
        let _ = app.update(UiMessage::Login(LoginMsg::Show(screen)));
        shot(&out, &format!("login_{screen:?}").to_lowercase(), (330.0, 620.0), &theme, app.view_login());
    }
    if real.is_none() {
        std::fs::remove_dir_all(data).expect("remove shot data dir");
    }
}
