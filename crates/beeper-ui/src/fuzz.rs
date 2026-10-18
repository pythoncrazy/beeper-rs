//! Random clicks, keys and messages on every window of a demo account, with every value of every preference.
//! `FUZZ_SEED` is the first of the four seeds, and `FUZZ_STEPS` is the number of steps of each seed.

use super::*;
use beeper_core::config::CentralConfig;
use iced::mouse::{Button, ScrollDelta};
use iced_test::simulator::{simulator, Simulator};
use std::sync::RwLock;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "a choice from no items");
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

const TEXTS: [&str; 10] =
    ["", "a", "hello world", ":)", "https://example.com", "**bold** _it_", "@", "/", "ü日本 🎉", "x\ny"];

/// The preferences with values, without "App language": the language is global, and other tests expect English.
fn prefs() -> impl Iterator<Item = &'static prefs::Pref> {
    prefs::all().map(|(_, p)| p).filter(|p| !matches!(p.control, prefs::Control::Microphone) && p.key != "appLanguage")
}

fn demo_app(name: &str) -> BeeperGuiApp {
    let dir = std::env::temp_dir().join(format!("beeper-rs-fuzz-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = CentralConfig::new(dir, "127.0.0.1".into(), 23391, None);
    config.demo = true;
    let store = Arc::new(RwLock::new(BeeperStore::from_config(config)));
    BeeperGuiApp::boot(store).0
}

fn windows(app: &BeeperGuiApp) -> Vec<window::Id> {
    let media = app.media_window.as_ref().map(|w| w.id);
    let chats = app.chat_windows.iter().map(|w| w.0);
    [app.main_window, app.settings_window, media].into_iter().flatten().chain(chats).collect()
}

/// Whether a message stays inside the process: no browser, relaunch, notification, recording or playback.
fn local(m: &UiMessage) -> bool {
    match m {
        UiMessage::OpenUrl(_)
        | UiMessage::Relaunch
        | UiMessage::NotifyTimer(..)
        | UiMessage::Record(_)
        | UiMessage::Voice(_)
        | UiMessage::Video(_)
        | UiMessage::Logout
        | UiMessage::CopyImage(..) => false,
        UiMessage::KeyPressed(Key::Character(c), mods) => !(c == "p" && mods.command() && mods.shift()),
        UiMessage::InWindow(_, m) | UiMessage::MessageMenu(m) | UiMessage::MediaItemAct(m) => local(m),
        _ => true,
    }
}

/// Renders a window at `size` and sends it `events` at `at`, with the messages that the window produces.
fn interact(app: &BeeperGuiApp, id: window::Id, size: Size, at: Point, events: Vec<iced::Event>) -> Vec<UiMessage> {
    let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
    sim.point_at(at);
    let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
    let _ = sim.simulate(events);
    sim.into_messages().collect()
}

fn random_events(rng: &mut Rng) -> (String, Vec<iced::Event>) {
    let button = |b| [mouse::Event::ButtonPressed(b), mouse::Event::ButtonReleased(b)].map(iced::Event::Mouse).to_vec();
    match rng.below(6) {
        0 | 1 => ("click".into(), button(Button::Left)),
        2 => ("right click".into(), button(Button::Right)),
        3 => {
            let y = *rng.pick(&[-3.0, 3.0, -20.0]);
            (
                format!("scroll {y}"),
                vec![iced::Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Lines { x: 0.0, y } })],
            )
        }
        4 => ("double click".into(), [button(Button::Left), button(Button::Left)].concat()),
        _ => {
            let text = rng.pick(&TEXTS);
            (format!("type {text:?}"), iced_test::simulator::typewrite(text).collect())
        }
    }
}

/// A message that a user can cause, with the ids of the demo account and the messages of the open chat, and its
/// description.
fn random_message(rng: &mut Rng, app: &BeeperGuiApp) -> (String, UiMessage) {
    let st = app.store.read().unwrap();
    let chats: Vec<String> = st.chats.iter().map(|c| c.id.clone()).collect();
    let deletable: Vec<String> = st.chats.iter().filter(|c| c.delete_modes != 0).map(|c| c.id.clone()).collect();
    let open = app.pane.selected_chat_id.as_deref().and_then(|c| st.chat_msg_indices.get(c)).filter(|m| !m.is_empty());
    let msgs: Vec<(String, String)> = match open {
        Some(idx) => idx.iter().map(|&i| (st.messages[i].chat_id.clone(), st.messages[i].id.clone())).collect(),
        None => vec![("!note_self:beeper.com".into(), "none".into())],
    };
    let open = open.is_some();
    let accounts: Vec<String> = st.accounts.iter().map(|a| a.account_id.clone()).collect();
    drop(st);
    let chat = rng.pick(&chats).clone();
    let (mchat, msg) = rng.pick(&msgs).clone();
    let text = rng.pick(&TEXTS).to_string();
    let flag = *rng.pick(&[
        ChatFlag::Archived,
        ChatFlag::Pinned,
        ChatFlag::LowPriority,
        ChatFlag::Muted,
        ChatFlag::MarkedUnread,
    ]);
    let tab = *rng.pick(&[
        InboxTab::AllChats,
        InboxTab::LowPriority,
        InboxTab::Archived,
        InboxTab::Reminders,
        InboxTab::SendLater,
        InboxTab::Requests,
    ]);
    let pref = *rng.pick(&prefs().collect::<Vec<_>>());
    let hotkeys: Vec<&str> = prefs::HOTKEYS.iter().flat_map(|(_, k)| k.iter().map(|k| k.0)).collect();
    let key = match rng.below(3) {
        0 => Key::Character(
            rng.pick(&["a", "e", "f", "k", "p", "u", "m", "i", "y", ",", "=", "-", "0", "[", "]"]).to_string().into(),
        ),
        _ => Key::Named(*rng.pick(&[
            Named::Enter,
            Named::Escape,
            Named::ArrowUp,
            Named::ArrowDown,
            Named::ArrowLeft,
            Named::ArrowRight,
            Named::Tab,
            Named::Backspace,
            Named::Delete,
            Named::PageUp,
        ])),
    };
    let mods = *rng.pick(&[
        Modifiers::empty(),
        Modifiers::CTRL,
        Modifiers::CTRL | Modifiers::SHIFT,
        Modifiers::ALT,
        Modifiers::SHIFT,
    ]);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let hits = search_settings(&text);
    let branch = rng.below(60);
    let what =
        format!("message #{branch} chat {chat} message {msg} text {text:?} pref {} key {key:?} {mods:?}", pref.key);
    let needs_msg = (7..=14).contains(&branch) || [16, 53].contains(&branch);
    if needs_msg && !open {
        return (what, UiMessage::Noop);
    }
    let m = match branch {
        0 => UiMessage::SelectTab(tab),
        1 => UiMessage::SelectChat(chat),
        2 => UiMessage::ClickChat(chat),
        3 => UiMessage::OpenChatMenu(chat),
        4 => UiMessage::ToggleFlag(chat, flag),
        5 => UiMessage::ToggleRead(chat),
        6 => UiMessage::SetReminder(chat, rng.coin().then_some(now + 3_600_000)),
        7 => UiMessage::OpenMessageMenu(msg),
        8 => UiMessage::ClickMessage(msg),
        9 => UiMessage::DoubleClickMessage(msg),
        10 => UiMessage::StartReply(msg),
        11 => UiMessage::StartEdit(msg, text),
        12 => UiMessage::AskDeleteMessage(msg),
        13 => UiMessage::ToggleReaction(mchat, msg, "👍".into()),
        14 => UiMessage::ToggleBookmark(msg),
        15 => UiMessage::ComposerChanged(text),
        16 => UiMessage::SubmitComposer,
        17 => UiMessage::CancelComposeMode,
        18 => UiMessage::StepMessage(rng.coin()),
        19 => UiMessage::OpenSettings(*rng.pick(&SettingsTab::ALL)),
        20 => UiMessage::SelectSettingsTab(*rng.pick(&SettingsTab::ALL)),
        21 => UiMessage::SettingsQueryChanged(text),
        22 | 23 | 24 => UiMessage::SetPref(pref.key, rng.below(usize::from(pref.max_value()) + 1) as u8),
        25 => UiMessage::ToggleHotkey(rng.pick(&hotkeys)),
        26 => UiMessage::RevertHotkeys,
        27 => UiMessage::Zoom(*rng.pick(&[-1, 0, 1])),
        28 => UiMessage::SelectTheme(*rng.pick(&ThemeChoice::ALL)),
        29 => UiMessage::SystemTheme(*rng.pick(&[iced::theme::Mode::Dark, iced::theme::Mode::Light])),
        30 => UiMessage::ToggleRightPanel,
        31 => UiMessage::ToggleCommandPalette,
        32 => UiMessage::CommandQueryChanged(text),
        33 => UiMessage::ToggleFilterMenu,
        34 => UiMessage::PickFilter(rng.below(8) as u8),
        35 => UiMessage::NewChat,
        36 => UiMessage::NewChatQueryChanged(text),
        37 => UiMessage::BeginSelecting(chat),
        38 => UiMessage::ToggleSelected(chat),
        39 => UiMessage::SelectionSet(flag, rng.coin()),
        40 => UiMessage::SearchRoom(text),
        41 => UiMessage::OpenMedia,
        42 => UiMessage::CloseModals,
        43 => UiMessage::ToggleAttachMenu,
        44 => UiMessage::OpenChatWindow(chat),
        45 => UiMessage::CloseRequested(*rng.pick(&windows(app))),
        46 | 47 | 48 => UiMessage::KeyPressed(key, mods),
        49 if !deletable.is_empty() => UiMessage::AskDeleteChat(rng.pick(&deletable).clone()),
        50 => UiMessage::ReminderMenu,
        51 => UiMessage::ChatSub(Some(rng.below(SUB_MENU_W.len()) as u8)),
        52 => UiMessage::ArchiveStale,
        53 => UiMessage::Forward(msg),
        54 if app.forward.is_some() => UiMessage::ForwardPick(chat),
        55 => UiMessage::SelectAccount(rng.coin().then(|| rng.pick(&accounts).clone())),
        56 if !hits.is_empty() => UiMessage::PickSetting(*rng.pick(&hits)),
        57 => UiMessage::WindowFocused(rng.coin()),
        58 => UiMessage::HotkeyQuery(text),
        _ => UiMessage::ToggleDnd,
    };
    (what, m)
}

fn run(seed: u64, steps: usize) {
    let mut app = demo_app(&format!("random-{seed}"));
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut log: Vec<String> = Vec::new();
    for step in 0..steps {
        let ids = windows(&app);
        let id = *rng.pick(&ids);
        let size = *rng.pick(&[Size::new(1440.0, 900.0), Size::new(1024.0, 640.0), Size::new(420.0, 320.0)]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (what, msgs) = if rng.below(3) == 0 {
                let (what, m) = random_message(&mut rng, &app);
                (what, vec![m])
            } else {
                let at = Point::new(rng.below(size.width as usize) as f32, rng.below(size.height as usize) as f32);
                let (what, events) = random_events(&mut rng);
                let what = format!("{what} at ({}, {}) in {id:?}", at.x, at.y);
                (what, interact(&app, id, size, at, events))
            };
            log.push(format!("step {step} {size:?}: {what} gave {} messages", msgs.len()));
            for m in msgs.into_iter().filter(local) {
                let _ = app.update(m);
            }
            for id in windows(&app) {
                let _ = interact(&app, id, size, Point::ORIGIN, Vec::new());
            }
        }));
        if result.is_err() {
            panic!("seed {seed} panicked after:\n{}", log[log.len().saturating_sub(12)..].join("\n"));
        }
    }
}

#[test]
fn every_pref_value_renders() {
    let mut app = demo_app("prefs");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::General));
    for pref in prefs() {
        for v in 0..=pref.max_value() {
            let _ = app.update(UiMessage::SetPref(pref.key, v));
            for tab in SettingsTab::ALL {
                let _ = app.update(UiMessage::SelectSettingsTab(tab));
                let _ = simulator(app.view_settings());
            }
            let _ = simulator(app.view_main());
        }
        let _ = app.update(UiMessage::SetPref(pref.key, pref.default_value()));
    }
}

#[test]
fn random_input_does_not_panic() {
    let var =
        |k, default| std::env::var(k).map_or(default, |n| n.parse().unwrap_or_else(|_| panic!("{k} is a number")));
    let (seed, steps) = (var("FUZZ_SEED", 0), var("FUZZ_STEPS", 100));
    for seed in seed..seed + 4 {
        run(seed, steps as usize);
    }
}
