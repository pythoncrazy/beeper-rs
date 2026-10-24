//! Random clicks, keys and messages on every window of a demo account, with every value of every preference.
//! `FUZZ_SEED` is the first of the four seeds, and `FUZZ_STEPS` is the number of steps of each seed.

use super::*;
use beeper_core::config::CentralConfig;
use beeper_core::store::StoreLock;
use iced::mouse::{Button, ScrollDelta};
use iced_test::simulator::{simulator, Simulator};

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
/// The preferences with their settings tab.
fn prefs() -> impl Iterator<Item = (SettingsTab, &'static prefs::Pref)> {
    let tab = |page| *SettingsTab::ALL.iter().find(|t| t.meta().0 == page).expect("a settings tab");
    prefs::all()
        .filter(|(_, p)| !matches!(p.control, prefs::Control::Microphone | prefs::Control::Multi(_)))
        .filter(|(_, p)| p.key != "appLanguage")
        .map(move |(page, p)| (tab(page), p))
}

pub(crate) fn demo_app(name: &str) -> BeeperGuiApp {
    let dir = std::env::temp_dir().join("beeper-rs-fuzz").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = CentralConfig::new(dir, "127.0.0.1".into(), 23391, None);
    config.demo = true;
    let store = StoreLock::shared(BeeperStore::from_config(config));
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
    match rng.below(7) {
        5 => {
            let k = *rng.pick(&[Named::Backspace, Named::Delete, Named::Enter, Named::ArrowLeft, Named::Home]);
            let m = *rng.pick(&[Modifiers::empty(), Modifiers::CTRL, Modifiers::SHIFT, Modifiers::ALT]);
            (format!("click and {k:?} {m:?}"), [button(Button::Left), vec![key(k, m)]].concat())
        }
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
    let st = app.store.read();
    let chats: Vec<String> = st.chats.iter().map(|c| c.id.clone()).collect();
    let deletable: Vec<String> = st.chats.iter().filter(|c| c.delete_modes != 0).map(|c| c.id.clone()).collect();
    let open = app.pane.selected_chat_id.as_deref().and_then(|c| st.chat_msg_indices.get(c)).filter(|m| !m.is_empty());
    let msgs: Vec<(String, String)> = match open {
        Some(idx) => idx.iter().map(|&i| (st.messages[i].chat_id.clone(), st.messages[i].id.clone())).collect(),
        None => vec![("!note_self:beeper.com".into(), "none".into())],
    };
    let open = open.is_some();
    let accounts: Vec<String> = st.accounts.iter().map(|a| a.account_id.clone()).collect();
    let rail = crate::rail_accounts(&st);
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
    let pref = rng.pick(&prefs().collect::<Vec<_>>()).1;
    let hotkeys: Vec<&str> = prefs::HOTKEYS.iter().map(|h| h.0).collect();
    let key = match rng.below(3) {
        0 => {
            let combos = prefs::HOTKEYS.iter().flat_map(|h| h.2.split(", "));
            let keys = combos.filter_map(|c| c.rsplit('+').next().filter(|k| k.chars().count() == 1));
            Key::Character(rng.pick(&keys.chain(["a", "1", "c", "v"]).collect::<Vec<_>>()).to_string().into())
        }
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
        Modifiers::CTRL | Modifiers::ALT,
        Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT,
        Modifiers::ALT,
        Modifiers::SHIFT,
    ]);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let hits = search_settings(&app.store.read(), &text);
    let branch = rng.below(65);
    let what =
        format!("message #{branch} chat {chat} message {msg} text {text:?} pref {} key {key:?} {mods:?}", pref.key);
    let needs_msg = (7..=14).contains(&branch) || [53, 62].contains(&branch);
    let needs_chat = [16, 18].contains(&branch);
    if (needs_msg && !open) || (needs_chat && app.pane.selected_chat_id.is_none()) {
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
        12 => UiMessage::AskDeleteMessage(vec![msg]),
        13 => UiMessage::ToggleReaction(mchat, msg, "👍".into()),
        14 => UiMessage::ToggleBookmark(msg),
        15 => UiMessage::ComposerChanged(text),
        16 => UiMessage::SubmitComposer,
        17 => UiMessage::CancelComposeMode,
        18 => UiMessage::StepMessage(rng.coin(), rng.coin()),
        19 => UiMessage::OpenSettings(*rng.pick(&SettingsTab::ALL)),
        20 => UiMessage::SelectSettingsTab(*rng.pick(&SettingsTab::ALL)),
        21 => UiMessage::SettingsQueryChanged(text),
        22..=24 => UiMessage::SetPref(pref.key, rng.below(usize::from(pref.max_value()) + 1) as u8),
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
        46..=48 => UiMessage::KeyPressed(key, mods),
        49 if !deletable.is_empty() => UiMessage::AskDeleteChat(rng.pick(&deletable).clone()),
        50 => UiMessage::ReminderMenu,
        51 => UiMessage::ChatSub(Some(rng.below(SUB_MENU_W.len()) as u8)),
        52 => UiMessage::ArchiveStale,
        53 => UiMessage::Forward(vec![msg]),
        54 if app.forward.is_some() => UiMessage::ForwardPick(chat),
        55 => UiMessage::SelectAccount(rng.coin().then(|| rng.pick(&accounts).clone())),
        56 if !hits.is_empty() => UiMessage::PickSetting(*rng.pick(&hits)),
        57 => UiMessage::WindowFocused(rng.coin()),
        58 => UiMessage::HotkeyQuery(text),
        59 => UiMessage::OpenAccount(rng.coin().then(|| rng.pick(&accounts).clone())),
        60 => UiMessage::SetAccountProps(
            rng.pick(&accounts).clone(),
            beeper_core::models::AccountProps { notifications: rng.coin(), previews: rng.coin(), silent: rng.coin() },
        ),
        61 => UiMessage::OpenChatMenu(format!("account:{}", rng.pick(&rail))),
        62 => UiMessage::SelectMessages(vec![msg]),
        63 => UiMessage::CopySelected(rng.coin()),
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

/// Sends `events` to the main window, after a click on the composer if `focus`, and applies the messages.
fn press(app: &mut BeeperGuiApp, focus: bool, events: impl IntoIterator<Item = iced::Event>) {
    let id = app.main_window.expect("a main window");
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id));
    if focus {
        sim.click(iced::widget::Id::new("composer-input")).expect("a composer");
    }
    let _ = sim.simulate(events);
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    for m in msgs {
        let _ = app.update(m);
    }
}

/// The modifier of Desktop's word deletion: Option on macOS, Ctrl elsewhere.
const WORD: Modifiers = if cfg!(target_os = "macos") { Modifiers::ALT } else { Modifiers::CTRL };

fn key(key: impl Into<Key>, modifiers: Modifiers) -> iced::Event {
    let key = key.into();
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers,
        repeat: false,
        text: None,
    })
}

#[test]
fn composer_keys_edit_like_desktop() {
    use iced_test::simulator::typewrite;
    let mut app = demo_app("composer");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    press(&mut app, true, typewrite("hello big world").chain([key(Named::Backspace, WORD)]));
    assert_eq!(text(&app), "hello big ");
    press(&mut app, true, [key(Named::Backspace, WORD)]);
    assert_eq!(text(&app), "hello ");
    press(&mut app, true, [key(Named::Home, Modifiers::empty()), key(Named::Delete, WORD)]);
    assert_eq!(text(&app), " ");
    press(&mut app, true, typewrite("a bc"));
    let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Select(text_editor::Motion::Left)));
    let _ = app.update(UiMessage::DeleteTo(text_editor::Motion::WordLeft, true));
    assert_eq!(text(&app), " a b", "Ctrl+Backspace deletes only the selection");
    press(&mut app, false, [key(Named::Enter, Modifiers::empty()), key(Named::Backspace, WORD)]);
    assert_eq!(text(&app), " a b", "an unfocused composer ignores keys");
}

#[test]
fn composer_enter_follows_the_hotkeys() {
    use iced_test::simulator::typewrite;
    let note = "!note_self:beeper.com";
    let mut app = demo_app("enter");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    let archived = |app: &BeeperGuiApp| {
        let st = app.store.read();
        has_flag(&st.chats[st.resolve_chat_idx(note).expect("note")], ChatFlag::Archived)
    };
    press(
        &mut app,
        true,
        typewrite("a").chain([key(Named::Enter, Modifiers::SHIFT), key(Named::Enter, Modifiers::ALT)]),
    );
    assert_eq!(text(&app), "a\n\n", "Shift and Alt Enter insert new lines");
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    let tick = || key(Key::Character("`".into()), Modifiers::empty());
    press(&mut app, true, typewrite("b").chain([key(Named::Home, Modifiers::SHIFT), tick()]));
    assert_eq!(text(&app), "`b`", "a backtick formats the selection as code");
    press(&mut app, true, [tick()]);
    assert_eq!(text(&app), "`b``", "a backtick without a selection is a backtick");
    let _ = app.update(UiMessage::ComposerChanged("b".into()));
    let _ = app.update(UiMessage::ToggleFlag(note.into(), ChatFlag::Pinned));
    press(&mut app, true, [key(Named::Enter, Modifiers::CTRL)]);
    assert!(text(&app).is_empty() && archived(&app), "Ctrl Enter sends and archives");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    press(&mut app, true, typewrite("c").chain([key(Named::Enter, Modifiers::empty())]));
    assert!(text(&app).is_empty() && !archived(&app), "a new message moves the chat to the inbox");
    {
        let keys = &mut app.store.write().settings.keybindings;
        keys.insert("Send Message".into(), "ctrl+enter".into());
        keys.insert("Message: Send Message and Archive".into(), "disabled".into());
    }
    press(&mut app, true, typewrite("d").chain([key(Named::Enter, Modifiers::empty())]));
    assert_eq!(text(&app), "d\n", "Enter inserts a new line when it does not send");
}

#[test]
fn attachments_send_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("attach");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let mut png = Vec::new();
    ::image::RgbaImage::new(3, 2).write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).unwrap();
    let file = |name: &str, bytes: &[u8]| {
        let path = std::env::temp_dir().join(format!("beeper-rs-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let f = portal::load_file(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        PendingMedia { name: name.into(), ..f }
    };
    let files = vec![file("a.png", &png), file("b.pdf", b"%PDF")];
    let _ = app.update(UiMessage::FilesLoaded(Ok(files.clone())));
    assert!(simulator(app.view_main()).find("PDF ・ 4 B").is_ok(), "a file shows its type and size");
    let _ = app.update(UiMessage::ComposerChanged("hi".into()));
    let _ = app.update(UiMessage::SubmitComposer);
    let last = |app: &BeeperGuiApp, n: usize| {
        let st = app.store.read();
        let ids = &st.chat_msg_indices[note];
        let msgs = ids[ids.len() - n..].iter().map(|&i| &st.messages[i]);
        msgs.map(|m| (m.text.clone(), m.attachments.iter().map(|a| a.attachment_type.clone()).collect())).collect()
    };
    let sent: Vec<(Option<String>, Vec<String>)> = last(&app, 3);
    let want = [(Some("hi".into()), vec![]), (None, vec!["img".into()]), (None, vec!["file".into()])];
    assert_eq!(sent, want, "the text first, then each attachment");
    assert!(app.pane.attachments.is_empty() && app.pane.composer.text().is_empty());
    let _ = app.update(UiMessage::FilesLoaded(Ok(files[1..].to_vec())));
    let _ = app.update(UiMessage::ComposerChanged("cap".into()));
    let _ = app.update(UiMessage::SubmitComposer);
    assert_eq!(last(&app, 1), [(Some("cap".into()), vec!["file".into()])], "one attachment has the text as caption");
    let _ = app.update(UiMessage::FilesLoaded(Ok(files.clone())));
    let _ = app.update(UiMessage::DiscardAttachment(0));
    assert_eq!(app.pane.attachments.len(), 1);
    let _ = app.update(UiMessage::DiscardAttachment(0));
    let id = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::FileHover(id, true));
    assert!(simulator(app.view_main()).find("Drop files here to attach").is_ok(), "a drag shows the drop zone");
    let _ = app.update(UiMessage::Modifiers(Modifiers::SHIFT));
    let _ = app.update(UiMessage::FileDropped(id, "/none".into()));
    let _ = app.update(UiMessage::FilesLoaded(Ok(files[1..].to_vec())));
    assert!(!app.dropping && app.pane.attachments.is_empty(), "a drop with Shift sends");
    assert_eq!(last(&app, 1), [(None, vec!["file".into()])]);
    let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    let _ = app.update(UiMessage::FileHover(id, true));
    let _ = app.update(UiMessage::FileDropped(id, "/none".into()));
    let _ = app.update(UiMessage::FilesLoaded(Ok(files[1..].to_vec())));
    assert_eq!(app.pane.attachments.len(), 1, "a drop attaches");
}

#[test]
fn filter_account_submenu_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("filteracc");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("a".into()), Modifiers::CTRL | Modifiers::ALT));
    assert_eq!(app.cmd_submenu, Some(commands::SubMenu::FilterAccount));
    let cmds = app.commands(&app.store.read(), "");
    assert_eq!(cmds[0].title, "All Accounts");
    assert_eq!(cmds[1].account.as_deref(), Some("hungryserv"));
    assert!(cmds.iter().all(|c| c.account.is_none() || c.hotkey.is_some()), "{}", cmds.len());
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Filter account").is_ok() && sim.find("All Accounts").is_ok());
        assert!(sim.find("Window").is_err(), "a submenu has no category headings");
    }
    let target = cmds.iter().find(|c| c.account.as_deref().is_some_and(|a| network_name(a) == "Telegram")).unwrap();
    let target = target.account.clone();
    let _ = app.update(UiMessage::CommandQueryChanged("telegram".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(!app.show_command_palette && app.selected_account_filter == target, "the network name finds the account");
    let _ = app.update(UiMessage::RunHotkey("Filter By Account"));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.selected_account_filter.is_none(), "All Accounts clears the filter");
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    assert!(app.cmd_submenu.is_none(), "the next command bar is the normal one");
}

#[test]
fn incognito_toggle_like_desktop() {
    let mut app = demo_app("incognito");
    let on = |app: &BeeperGuiApp, key| app.store.read().settings.on(key);
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert!(!on(&app, "incognito"), "the free plan locks incognito mode");
    let trial =
        |ends: &str| serde_json::json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": ends } });
    app.store.write().features.usage = trial("2999-01-01T00:00:00Z");
    assert!(!on(&app, "incognito") && on(&app, "typingIndicators"));
    let titles = |app: &BeeperGuiApp| app.commands(&app.store.read(), "stealth").into_iter().map(|c| c.title);
    assert_eq!(titles(&app).next().as_deref(), Some("Turn on incognito mode"));
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert!(on(&app, "incognito"));
    assert_eq!(app.status_banner, "Incognito mode on. Also hide when you're typing from recipients?");
    assert_eq!(titles(&app).next().as_deref(), Some("Turn off incognito mode"));
    assert!(simulator(app.view_main()).find("Undo").is_ok(), "Undo follows Hide typing");
    {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("Hide typing").expect("the toast offers to hide typing");
        for m in sim.into_messages() {
            let _ = app.update(m);
        }
    }
    assert!(!on(&app, "typingIndicators") && app.status_banner.is_empty());
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert_eq!(app.status_banner, "Incognito mode off. Also show when you're typing to recipients?");
    app.status_banner = "Copied".into();
    assert!(simulator(app.view_main()).find("Show typing").is_err(), "another banner has no stale button");
    let _ = app.update(UiMessage::DismissBanner);
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert_eq!(app.status_banner, "Incognito mode on.");
    let _ = app.update(UiMessage::Undo(false));
    assert!(!on(&app, "incognito") && app.status_banner == "Undo: Enable Incognito Mode");
    let _ = app.update(UiMessage::SetPref("typingIndicators", 1));
    let _ = app.update(UiMessage::DismissBanner);
    let _ = app.update(UiMessage::SetPref("incognito", 1));
    assert!(app.status_banner.is_empty(), "Settings change incognito mode without a toast");
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert_eq!(app.status_banner, "Incognito mode off.");
    let _ = app.update(UiMessage::SetPref("incognito", 1));
    assert!(app.store.read().incognito());
    app.store.write().features.usage = trial("2020-01-04T00:00:00Z");
    assert!(!app.store.read().incognito() && app.store.read().typing_notices(), "the trial ended");
}

#[test]
fn settings_commands_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("settingscmd");
    let cmds = app.commands(&app.store.read(), "");
    let settings: Vec<&str> = cmds.iter().filter(|c| c.category == "Settings").map(|c| c.title.as_str()).collect();
    assert_eq!(settings[..4], ["Add Account", "Accounts", "Beeper Plus", "General"]);
    assert_eq!(settings[4], "General → Incognito mode");
    assert!(settings.contains(&"Appearance → Show sidebar") && settings.contains(&"Accounts → Telegram"));
    assert_eq!(settings.last(), Some(&"Turn on incognito mode"));
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::CommandQueryChanged("compact chat list".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.settings_window.is_some() && app.active_settings_tab == SettingsTab::Appearance);
    assert_eq!(app.pref_flash.map(|f| f.0), Some("compactChatList"));
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::CommandQueryChanged("Accounts → Telegram".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.active_settings_tab == SettingsTab::Accounts && app.adding_account);
}

#[test]
fn remind_submenu_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("remindsub");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let _ = app.update(UiMessage::KeyPressed(Key::Character("l".into()), Modifiers::CTRL));
    assert_eq!(app.cmd_submenu, Some(commands::SubMenu::Remind(id.clone())), "Ctrl L opens Remind Me");
    let labels = |app: &BeeperGuiApp, q: &str| -> Vec<String> {
        app.commands(&app.store.read(), q).into_iter().filter_map(|c| c.render.map(|r| r.0)).collect()
    };
    let all = labels(&app, "");
    assert_eq!(all.iter().collect::<std::collections::HashSet<_>>().len(), all.len(), "no phrase shows twice");
    assert!(all.contains(&"in two hours".into()) && all.contains(&"next sunday".into()), "{all:?}");
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Remind Me").is_ok(), "the heading");
        assert!(app.cmd_no_reply, "the choice starts at If no reply");
        assert!(sim.find("Remind Me marks this chat as new at the scheduled time, if there is no reply.").is_ok());
    }
    let _ = app.update(UiMessage::CmdNoReply(false));
    assert!(simulator(app.view_main()).find("Remind Me marks this chat as new at the scheduled time").is_ok());
    let counted = labels(&app, "3");
    for unit in ["3 minutes", "3 hours", "3 days", "3 weeks"] {
        assert!(counted.contains(&unit.into()), "{unit} in {counted:?}");
    }
    let _ = app.update(UiMessage::CommandQueryChanged("friday at 5pm".into()));
    assert_eq!(labels(&app, "friday at 5pm")[0], "friday at 5pm", "the typed time comes first");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let at = parse_when_millis("friday at 5pm", now_unix_millis()).unwrap();
    let st = app.store.read();
    let r = st.chats[st.resolve_chat_idx(&id).unwrap()].reminder.clone().expect("a reminder");
    assert!(r.remind_at_ms == at && !r.dismiss_on_incoming_message, "Always keeps the reminder after a reply");
    drop(st);
    let _ = app.update(UiMessage::OpenSubMenu(commands::SubMenu::Remind(id.clone())));
    assert!(app.cmd_no_reply, "the choice starts at the setting");
    assert_eq!(labels(&app, "").last().map(String::as_str), Some("dismiss reminder"));
    assert_eq!(app.store.read().settings.last_time, "friday at 5pm", "the phrase is kept for Last Used");
    assert_eq!(labels(&app, "")[0], "last used");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::OpenChatMenu(id.clone()));
    let _ = app.update(UiMessage::ChatSub(Some(1)));
    assert!(simulator(app.view_main()).find("Last Used").is_ok(), "the chat menu offers the last phrase too");
}

#[test]
fn send_later_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("sendlater");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let edit = text_editor::Edit::Paste(Arc::new("see you then".into()));
    let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
    let _ = app.update(UiMessage::KeyPressed(Key::Character("l".into()), Modifiers::CTRL | Modifiers::SHIFT));
    assert_eq!(app.cmd_submenu, Some(commands::SubMenu::Schedule(Vec::new())), "Ctrl Shift L opens Send Later");
    assert!(app.cmd_no_reply, "the choice starts at If no reply");
    let footer = "Send Later messages can only send if Beeper app is running (and your computer is awake)";
    assert!(simulator(app.view_main()).find(footer).is_ok());
    let _ = app.update(UiMessage::CommandQueryChanged("friday at 5pm".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let at = parse_when_millis("friday at 5pm", now_unix_millis()).unwrap();
    assert_eq!(app.pane.send_later, Some((at, true)));
    let when = send_day(at, false);
    assert!(simulator(app.view_main()).find(format!("{when} if no reply")).is_ok(), "the composer banner");
    let sent = app.store.read().messages.len();
    let _ = app.update(UiMessage::SubmitComposer);
    let st = app.store.read();
    assert_eq!((st.messages.len(), app.pane.send_later), (sent, None), "nothing sends now");
    let s = st.scheduled[0].clone();
    assert!(s.text == "see you then" && s.send_on == at && s.send_only_on_no_response && !s.canceled);
    drop(st);
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find(format!("Send {when}")).is_ok() && sim.find("see you then").is_ok(), "the timeline");
    }
    let _ = app.update(UiMessage::SelectTab(InboxTab::SendLater));
    assert_eq!(app.visible_chats(&app.store.read()).iter().map(|c| &c.id).collect::<Vec<_>>(), [&id]);
    let _ = app.update(UiMessage::RescheduleTo(vec![s.id.clone()], at + 60_000, false));
    assert!(!app.store.read().scheduled[0].send_only_on_no_response);
    let _ = app.update(UiMessage::RescheduleTo(vec![s.id.clone()], at, true));
    let mut st = app.store.write();
    let (_, reply, _) = st.send_message(&id, Some("sure".into()), None, None).unwrap();
    let i = st.msg_idx_by_id[&reply];
    let m = st.update_message(i, |m| m.is_sender = Some(false));
    drop(st);
    let _ = app.update(UiMessage::Upserted(Box::new(m)));
    assert!(app.store.read().scheduled[0].canceled, "a reply cancels an If no reply message");
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    assert!(simulator(app.view_main()).find("Send Later message canceled by reply.").is_ok());
    let _ = app.update(UiMessage::Unschedule(vec![s.id.clone()]));
    assert!(app.store.read().scheduled.is_empty());
    app.store.write().schedule(&id, "now".into(), None, now_unix_millis() - 1, false);
    let _ = app.update(UiMessage::CheckScheduled);
    let st = app.store.read();
    assert!(
        st.scheduled.is_empty() && st.messages.last().unwrap().text.as_deref() == Some("now"),
        "a due message sends"
    );
}

#[test]
fn show_replies_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("replies");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let mut st = app.store.write();
    let root = st.messages[*st.chat_msg_indices[&id].last().unwrap()].id.clone();
    let mut send =
        |text: &str, to: Option<&String>| st.send_message(&id, Some(text.into()), to.cloned(), None).unwrap().1;
    let one = send("first answer", Some(&root));
    send("something else", None);
    let two = send("second answer", Some(&root));
    let chat = st.chats[st.resolve_chat_idx(&id).unwrap()].clone();
    drop(st);
    let msgs = {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("1 Reply").expect("the replies button under the first answer");
        sim.into_messages().collect::<Vec<_>>()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(app.pane.reply_thread.as_ref(), Some(&one));
    let place = format!("Reply to {} on {}", chat.title, network_name(&chat.account_id));
    assert_eq!(app.composer_placeholder(&chat), place, "the composer replies in the chain");
    let edit = text_editor::Edit::Paste(Arc::new("third answer".into()));
    let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
    let _ = app.update(UiMessage::SubmitComposer);
    let st = app.store.read();
    assert_eq!(st.messages.last().unwrap().linked_message_id.as_ref(), Some(&one), "a send replies to the message");
    drop(st);
    let _ = app.update(UiMessage::CloseModals);
    assert_eq!(app.pane.reply_thread, None, "Escape closes the replies");
    let _ = app.update(UiMessage::SelectMessages(vec![two]));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::CTRL));
    assert_eq!(app.pane.reply_thread.as_ref(), Some(&root), "Ctrl Down shows the chain of the selected message");
    let _ = app.update(UiMessage::OpenMessageMenu(one));
    assert!(simulator(app.view_main()).find("Jump to Message").is_ok());
}

#[test]
fn pickers_check_the_network_like_desktop() {
    let mut app = demo_app("pickers");
    let st = app.store.read();
    let on = |want: bool| {
        let stickers =
            |c: &&Chat| matches!(account_network(&c.account_id).kind, NetworkKind::WhatsApp | NetworkKind::Telegram);
        st.chats.iter().find(|c| stickers(c) == want).unwrap().id.clone()
    };
    let (wa, other) = (on(true), on(false));
    drop(st);
    let _ = app.update(UiMessage::SelectChat(wa));
    let _ = app.update(UiMessage::TogglePicker(Picker::Sticker));
    assert_eq!(app.pane.picker, Some(Picker::Sticker));
    assert!(simulator(app.view_main()).find("No sticker packs found").is_ok());
    let _ = app.update(UiMessage::TogglePicker(Picker::Sticker));
    assert_eq!(app.pane.picker, None, "the hotkey closes the open picker");
    let _ = app.update(UiMessage::SelectChat(other.clone()));
    let _ = app.update(UiMessage::TogglePicker(Picker::Sticker));
    assert_eq!(app.pane.picker, None);
    assert!(app.status_banner.starts_with("Stickers aren't supported on "), "{}", app.status_banner);
    let mut st = app.store.write();
    let i = st.resolve_chat_idx(&other).unwrap();
    st.chats[i].files = 0;
    drop(st);
    let _ = app.update(UiMessage::TogglePicker(Picker::Gif));
    assert_eq!(app.pane.picker, None);
    assert!(app.status_banner.starts_with("GIFs aren't supported on "), "{}", app.status_banner);
    let _ = app.update(UiMessage::Record(record::Msg::Start));
    assert!(app.recorder.is_none());
    assert!(app.status_banner.starts_with("Recording audio isn't supported on "), "{}", app.status_banner);
}

#[test]
fn profile_links_match_desktop() {
    let app = demo_app("profile");
    let st = app.store.read();
    let links: Vec<String> = st.chats.iter().filter_map(profile_link).collect();
    assert!(links.iter().any(|l| l.starts_with("https://www.instagram.com/") && l.ends_with('/')), "{links:?}");
    for c in st.chats.iter().filter(|c| c.chat_type == "group" || network_name(&c.account_id) == "WhatsApp") {
        assert_eq!(profile_link(c), None, "{} has no profile link", c.title);
    }
}

#[test]
fn chat_commands_like_desktop() {
    let mut app = demo_app("chatcmds");
    let id = app.store.read().chats.iter().find(|c| profile_link(c).is_some()).map(|c| c.id.clone()).unwrap();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let cmds = app.commands(&app.store.read(), "");
    let titled = |t: &str| cmds.iter().find(|c| c.title == t).unwrap_or_else(|| panic!("no command {t}"));
    assert_eq!(cmds[0].title, "Remind me later");
    assert!(titled("Report as spam").msg.is_none(), "no network reports chats here");
    assert_eq!(titled("Send file").hotkey, Some("Chat: Send File"));
    assert_eq!(titled("Filter by account").category, "Filter Chats");
    let open = cmds.iter().find(|c| c.hotkey == Some("Open User Profile")).expect("a profile command");
    let network = network_name(&app.store.read().chats.iter().find(|c| c.id == id).unwrap().account_id);
    assert!(open.title.starts_with("Open @") && open.title.ends_with(&format!(" on {network}")), "{}", open.title);
    let _ = app.update(UiMessage::Commands(vec![UiMessage::CloseModals, cmds[0].msg.clone().unwrap()]));
    assert!(app.show_command_palette && app.cmd_submenu == Some(commands::SubMenu::Remind(id)));
}

#[test]
fn history_and_folder_submenus_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("historysub");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().take(3).map(|c| c.id.clone()).collect();
    for id in &ids {
        let _ = app.update(UiMessage::SelectChat(id.clone()));
    }
    let _ = app.update(UiMessage::OpenSubMenu(commands::SubMenu::History));
    let rows = app.commands(&app.store.read(), "");
    let chats: Vec<&str> = rows.iter().filter_map(|c| c.chat.as_deref()).collect();
    assert_eq!(chats[..3], [&*ids[2], &*ids[1], &*ids[0]], "the most recent chat first");
    assert_eq!((rows[0].hotkey, rows[1].hotkey), (None, Some("History: Next Chat")));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::empty()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(&*ids[1]));
    let _ = app.update(UiMessage::OpenSubMenu(commands::SubMenu::Folders));
    let titles: Vec<String> = app.commands(&app.store.read(), "").into_iter().map(|c| c.title).collect();
    assert_eq!(titles, ["Archived", "Low Priority", "Requests"], "the open folder is not listed");
    let _ = app.update(UiMessage::CommandQueryChanged("low".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert_eq!(app.active_tab, InboxTab::LowPriority);
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let cmds = app.commands(&app.store.read(), "folder");
    assert!(cmds[0].title == "Filter by folder type" && cmds[0].msg.is_some());
    for (query, title, to) in [
        ("criticism", "Email feedback", "mailto:help+feedback@beeper.com"),
        ("whats new", "View changelog", CHANGELOG_URL),
    ] {
        let c = app.commands(&app.store.read(), query).into_iter().next().unwrap();
        assert!(c.title == title && matches!(c.msg, Some(UiMessage::OpenUrl(u)) if u == to), "{query}");
    }
}

#[test]
fn media_carousel_draws() {
    let mut app = demo_app("mediacarousel");
    let _ = app.update(UiMessage::OpenMedia);
    let id = app.media_window.as_ref().map(|w| w.id).expect("a media window");
    let mid = app.store.read().messages.iter().find(|m| !m.attachments.is_empty()).map(|m| m.id.clone()).unwrap();
    let _ = app.update(UiMessage::OpenCarousel(mid, 0, true));
    drop(app.view(id));
}

#[test]
fn selected_message_commands_like_desktop() {
    let mut app = demo_app("msgcmds");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let linked: Vec<String> = {
        let mut st = app.store.write();
        let own = st.messages.iter_mut().filter(|m| m.chat_id == "!note_self:beeper.com" && m.is_sender == Some(true));
        let linked = own.filter_map(|m| {
            let url = m.text.as_deref()?.split_whitespace().find(|w| w.starts_with("https://"))?.to_string();
            m.links = vec![beeper_core::models::MessageLink { url, title: "Title".into(), summary: None }];
            Some(m.id.clone())
        });
        linked.collect()
    };
    assert!(linked.len() > 1, "the demo has links");
    let _ = app.update(UiMessage::SelectMessages(linked[..1].to_vec()));
    let cmds = app.commands(&app.store.read(), "");
    let titles: Vec<&str> = cmds.iter().take_while(|c| c.category == "Message").map(|c| c.title.as_str()).collect();
    let all = ["Edit", "Delete for everyone", "Forward", "Copy", "Copy with timestamps", "Open link", "Unselect"];
    assert_eq!(titles, [&all[..], &["Reply", "Send later"]].concat());
    assert!(matches!(&cmds[5].msg, Some(UiMessage::OpenUrl(u)) if u.starts_with("https://")));
    let _ = app.update(cmds[6].msg.clone().unwrap());
    assert!(app.pane.selected_msgs.is_empty(), "Unselect clears the selection");
    let _ = app.update(UiMessage::SelectMessages(linked[..2].to_vec()));
    let cmds = app.commands(&app.store.read(), "");
    let titles: Vec<&str> = cmds.iter().take_while(|c| c.category == "Messages").map(|c| c.title.as_str()).collect();
    assert_eq!(titles, all[1..]);
    let _ = app.update(UiMessage::Commands(vec![UiMessage::CloseModals, cmds[4].msg.clone().unwrap()]));
    let rows = app.commands(&app.store.read(), "");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.title.contains(" — https://")),
        "{:?}",
        rows.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
}

#[test]
fn emoticons_follow_the_setting() {
    use iced_test::simulator::typewrite;
    let mut app = demo_app("emoticons");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    press(&mut app, true, typewrite("hi :) "));
    assert_eq!(text(&app), "hi 🙂 ");
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    press(&mut app, true, typewrite("hi :) ").chain([key(Named::Backspace, Modifiers::empty())]));
    assert_eq!(text(&app), "hi :) ", "Backspace undoes the replacement");
    press(&mut app, true, [key(Named::Backspace, Modifiers::empty())]);
    assert_eq!(text(&app), "hi :)");
    press(&mut app, true, [key(Named::Home, Modifiers::empty())].into_iter().chain(typewrite(";) ")));
    assert_eq!(text(&app), "😉 hi :)", "the replacement is at the caret");
    press(&mut app, true, typewrite("x").chain([key(Named::Backspace, Modifiers::empty())]));
    assert_eq!(text(&app), "😉 hi :)", "an other edit ends the undo");
    let _ = app.update(UiMessage::ComposerChanged("`a".into()));
    press(&mut app, true, typewrite(" :) "));
    assert_eq!(text(&app), "`a :) ", "no replacement in code");
    let _ = app.update(UiMessage::SetPref("keepEmoticons", 1));
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    press(&mut app, true, typewrite(":D "));
    assert_eq!(text(&app), ":D ", "the setting keeps emoticons");
}

#[test]
fn composer_undo_groups_like_prosemirror() {
    let mut app = demo_app("undo");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::ComposerChanged(String::new()));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    let paste = |app: &mut BeeperGuiApp, s: &str| {
        let edit = text_editor::Edit::Paste(Arc::new(s.into()));
        let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
    };
    let z = |shift: bool| {
        key(Key::Character("z".into()), if shift { Modifiers::COMMAND | Modifiers::SHIFT } else { Modifiers::COMMAND })
    };
    app.pane.history = History::default();
    paste(&mut app, "hello");
    app.pane.history.last = None;
    paste(&mut app, " world");
    press(&mut app, true, [z(false)]);
    assert_eq!(text(&app), "hello");
    assert_eq!(app.pane.composer.cursor().position.column, 5);
    press(&mut app, true, [z(false), z(false)]);
    assert_eq!(text(&app), "", "the oldest group is the empty text, and an undo without history does nothing");
    press(&mut app, true, [z(true)]);
    assert_eq!(text(&app), "hello");
    press(&mut app, true, [key(Key::Character("y".into()), Modifiers::COMMAND)]);
    assert_eq!(text(&app), "hello world");
    press(&mut app, true, [z(false)]);
    paste(&mut app, "!");
    press(&mut app, true, [z(true)]);
    assert_eq!(text(&app), "hello!", "an edit clears the redo history");
    paste(&mut app, "?");
    let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Move(text_editor::Motion::DocumentStart)));
    paste(&mut app, "> ");
    press(&mut app, true, [z(false)]);
    assert_eq!(text(&app), "hello!?", "an edit at an other place starts a group");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    assert!(app.pane.history.undo.is_empty(), "a chat starts a new history");
}

#[test]
fn arrow_keys_step_over_a_deleted_selected_message() {
    let mut app = demo_app("step");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let ids: Vec<String> = {
        let st = app.store.read();
        st.chat_msg_indices[chat].iter().map(|&i| st.messages[i].id.clone()).collect()
    };
    assert!(ids.len() >= 3, "the demo chat has messages");
    let mid = ids[ids.len() - 2].clone();
    let _ = app.update(UiMessage::ClickMessage(mid.clone()));
    {
        let mut st = app.store.write();
        let i = st.msg_idx_by_id[&mid];
        st.messages[i].is_deleted = Some(true);
    }
    let _ = app.update(UiMessage::StepMessage(true, false));
    assert_eq!(app.pane.selected_msgs.last(), ids.last());
    let _ = app.update(UiMessage::ClickMessage(mid));
    let _ = app.update(UiMessage::StepMessage(false, false));
    assert_eq!(app.pane.selected_msgs, [ids[ids.len() - 3].clone()]);
}

#[test]
fn forward_selected_messages_like_desktop() {
    let mut app = demo_app("fwdmany");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let texts = |app: &BeeperGuiApp, cid: &str| -> Vec<Option<String>> {
        let st = app.store.read();
        st.chat_msg_indices[cid].iter().map(|&i| st.messages[i].text.clone()).collect()
    };
    let source = texts(&app, chat);
    let n = source.len();
    let key = |app: &mut BeeperGuiApp, k: Named, m| app.update(UiMessage::KeyPressed(Key::Named(k), m));
    let _ = key(&mut app, Named::ArrowUp, Modifiers::empty());
    let _ = key(&mut app, Named::ArrowUp, Modifiers::SHIFT);
    let _ = app.update(UiMessage::KeyPressed(Key::Character("f".into()), Modifiers::CTRL | Modifiers::SHIFT));
    assert_eq!(app.forward.as_ref().map(|f| f.mids.len()), Some(2), "Ctrl Shift F forwards all selected messages");
    let target = app.visible_chats(&app.store.read()).iter().find(|c| c.id != chat).unwrap().id.clone();
    let before = texts(&app, &target).len();
    let _ = app.update(UiMessage::ForwardPick(target.clone()));
    let _ = app.update(UiMessage::ForwardSend);
    let after = texts(&app, &target);
    assert_eq!(after[before..], source[n - 2..], "the messages arrive in timeline order");
}

#[test]
fn messages_select_like_desktop() {
    let mut app = demo_app("multi");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let ids: Vec<String> = {
        let st = app.store.read();
        st.chat_msg_indices[chat].iter().map(|&i| st.messages[i].id.clone()).collect()
    };
    let n = ids.len();
    let key = |app: &mut BeeperGuiApp, k: Named, m| app.update(UiMessage::KeyPressed(Key::Named(k), m));
    let _ = key(&mut app, Named::ArrowUp, Modifiers::empty());
    let _ = key(&mut app, Named::ArrowUp, Modifiers::SHIFT);
    let _ = key(&mut app, Named::ArrowUp, Modifiers::SHIFT);
    assert_eq!(
        app.pane.selected_msgs,
        [ids[n - 1].as_str(), ids[n - 2].as_str(), ids[n - 3].as_str()],
        "Shift Up adds the previous message"
    );
    let _ = key(&mut app, Named::ArrowDown, Modifiers::SHIFT);
    assert_eq!(app.pane.selected_msgs.len(), 3, "Shift Down past the last message adds nothing");
    let _ = key(&mut app, Named::ArrowUp, Modifiers::empty());
    assert_eq!(app.pane.selected_msgs, [ids[n - 2].as_str()], "Up goes from the last selected message");
    let _ = app.update(UiMessage::Modifiers(Modifiers::CTRL));
    let _ = app.update(UiMessage::ClickMessage(ids[n - 2].clone()));
    let _ = app.update(UiMessage::ClickMessage(ids[0].clone()));
    assert_eq!(app.pane.selected_msgs, [ids[0].as_str()], "Ctrl click toggles a message");
    let _ = app.update(UiMessage::Modifiers(Modifiers::SHIFT));
    let _ = app.update(UiMessage::ClickMessage(ids[2].clone()));
    assert_eq!(
        app.pane.selected_msgs,
        [ids[0].as_str(), ids[1].as_str(), ids[2].as_str()],
        "Shift click adds the range"
    );
    let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    let _ = app.update(UiMessage::ClickMessage(ids[1].clone()));
    assert_eq!(
        app.pane.selected_msgs,
        [ids[0].as_str(), ids[2].as_str()],
        "a click on a selected message unselects it"
    );
    let _ = app.update(UiMessage::ClickMessage(ids[3].clone()));
    assert_eq!(app.pane.selected_msgs, [ids[3].as_str()], "a click selects only the message");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("a".into()), Modifiers::COMMAND));
    assert_eq!(app.pane.selected_msgs, ids, "Ctrl A selects all messages");
    let _ = key(&mut app, Named::Escape, Modifiers::empty());
    assert!(app.pane.selected_msgs.is_empty());
    let _ = key(&mut app, Named::Escape, Modifiers::empty());
    let _ = app.update(UiMessage::KeyPressed(Key::Character("a".into()), Modifiers::COMMAND));
    let list = app.list_ids(&app.store.read());
    assert!(app.selection == list && app.pane.selected_chat_id == list.first().cloned(), "Ctrl A selects all chats");
}

#[test]
fn devices_offer_the_app_like_desktop() {
    let mut app = demo_app("devices");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Devices));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Take Beeper with you").is_ok());
    let _ = sim.click("Get the app");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    assert!(app.app_qr && simulator(app.view_settings()).find("Get the app").is_err(), "the button shows the QR code");
    let _ = app.update(UiMessage::SelectSettingsTab(SettingsTab::About));
    assert!(!app.app_qr, "another tab hides the QR code again");
    let _ = app.update(UiMessage::SelectSettingsTab(SettingsTab::Devices));
    let device = |name: &str, model: Option<&str>, kind: Option<&str>| beeper_core::matrix::DeviceInfo {
        name: name.into(),
        model: model.map(Into::into),
        kind: kind.map(Into::into),
        last_seen_ms: None,
        verified: true,
        current: false,
    };
    let _ = app.update(UiMessage::DevicesLoaded(Ok(vec![
        device("Beeper Desktop", Some("Linux PC"), Some("computer")),
        device("Beeper iPad", Some("iPad Air"), Some("tablet")),
        device("beeper-rs", None, None),
    ])));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Linux PC").is_ok() && sim.find("iPad Air").is_ok(), "the device metadata model shows");
}

#[test]
fn storage_clears_like_desktop() {
    let mut app = demo_app("storage");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Storage));
    let _ = app.update(UiMessage::StorageScanned(beeper_core::storage::scan(&app.store)));
    let mut sim = simulator(app.view_settings());
    for label in ["Messages", "Clear storage older than", "Clear All Cache", "Storage by Chat", "Note to self"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    let _ = sim.click("Note to self");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    let _ = app.update(UiMessage::StorageAge(1, 0));
    let _ = app.update(UiMessage::Ask(Some(ASK_CLEAR_CHAT)));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Clear storage for Note to self?").is_ok(), "the dialog names the chat");
    let picked: Vec<_> = app.storage_pick(2).into_iter().cloned().collect();
    assert!(!picked.is_empty() && picked.iter().all(|e| e.chat.as_deref() == Some(NOTE_CHAT_ID)));
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(beeper_core::storage::clear(app.store.clone(), picked)).unwrap();
    let after = beeper_core::storage::scan(&app.store);
    assert!(after.iter().all(|e| e.chat.as_deref() != Some(NOTE_CHAT_ID)), "the chat has no local data left");
    assert!(app.store.read().chat_msg_indices[NOTE_CHAT_ID].is_empty());
    let counts = [compact(34), compact(1234), compact(15_000), compact(2_500_000), grouped(1234567)];
    assert_eq!(counts, ["34", "1.2K", "15K", "2.5M", "1,234,567"]);
}

#[test]
fn integrations_like_desktop() {
    let mut app = demo_app("integrations");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Integrations));
    let mut sim = simulator(app.view_settings());
    for label in
        ["Allow connections", "Join Developer Community chat", "Open in Cursor", "Copy Command", "MCP endpoint"]
    {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    let base = app.store.read().config.base_url();
    assert!(sim.find(format!("claude mcp add beeper {base}/v0/mcp -t http -s user")).is_ok());
    drop(sim);
    let on = app.store.read().api_on.subscribe();
    let _ = app.update(UiMessage::ToggleSetting(|x| x.enable_pas_api ^= true));
    assert!(*on.borrow() && app.store.read().settings.enable_pas_api, "the toggle starts the Desktop API");
    let _ = app.update(UiMessage::McpTarget(3));
    assert!(simulator(app.view_settings()).find("Prefer Streamable HTTP when possible.").is_ok());
    let _ = app.update(UiMessage::JoinCommunity);
    assert_eq!(app.update(UiMessage::JoinCommunity).units(), 0, "a second click waits for the join");
    let base = "http://127.0.0.1:23373";
    assert_eq!(
        mcp_value("Cursor", base),
        "cursor://anysphere.cursor-deeplink/mcp/install?name=beeper&config=eyJ1cmwiOiJodHRwOi8vMTI3LjAuMC4xOjIzMzczL3YwL21jcCJ9"
    );
    assert_eq!(
        mcp_value("VS Code", base),
        "vscode:mcp/install?%7B%22name%22%3A%22beeper%22%2C%22type%22%3A%22http%22%2C%22url%22%3A%22http%3A%2F%2F127.0.0.1%3A23373%2Fv0%2Fmcp%22%7D"
    );
    let stdio =
        serde_json::json!({ "mcpServers": { "beeper": { "command": "npx", "args": ["-y", "@beeper/mcp-remote"] } } });
    assert_eq!(serde_json::from_str::<serde_json::Value>(&mcp_value("stdio", base)).unwrap(), stdio);
}

#[test]
fn approve_connection_like_desktop() {
    use beeper_core::connect::{ClientInfo, Outcome};
    let mut app = demo_app("approve");
    let ask = |app: &mut BeeperGuiApp, version: Option<&str>| {
        let client =
            ClientInfo { name: Some("Raycast".into()), version: version.map(String::from), ..ClientInfo::default() };
        let (id, _, rx) = app.store.write().connect.request(client, vec!["read".into()], "raycast://oauth");
        let _ = app.update(UiMessage::TokenRequest(id.clone(), None));
        (id, rx)
    };
    let (_, mut rx) = ask(&mut app, Some("1.2"));
    let mut sim = simulator(app.view_main());
    for label in ["Approve connection", "Allow sensitive actions", "Expires in", "Decline", "Approve"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    drop(sim);
    assert_eq!(app.token_ask.as_ref().unwrap().app, "Raycast v1.2");
    let _ = app.update(UiMessage::TokenWrite);
    let _ = app.update(UiMessage::TokenExpiry(Expiry(3600)));
    let _ = app.update(UiMessage::TokenAnswer(true));
    let token = rx.try_recv().unwrap().expect("approved");
    let stored = app.store.read().connect.validate(&token, "", "").expect("the token works");
    assert_eq!(stored.scopes, ["read", "write"], "Allow sensitive actions grants write");
    assert!(stored.expires_at.unwrap() - stored.created_at == 3_600_000 && app.token_ask.is_none());
    let (_, mut rx) = ask(&mut app, None);
    let _ = app.update(UiMessage::TokenAnswer(false));
    assert_eq!(rx.try_recv().unwrap(), None, "Decline rejects the request");
    let (id, _) = ask(&mut app, None);
    let _ = app.update(UiMessage::TokenRequest(id, Some(Outcome::Timeout)));
    assert!(simulator(app.view_main()).find("The request has timed out.").is_ok());
    let _ = app.update(UiMessage::TokenClose);
    assert!(app.token_ask.is_none());
}

#[test]
fn approved_connections_like_desktop() {
    let mut app = demo_app("connections");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Integrations));
    assert!(simulator(app.view_settings()).find("Add a connection to get started").is_ok());
    let _ = app.update(UiMessage::AddToken(true));
    let mut sim = simulator(app.view_settings());
    for label in ["Name (optional)", "Expires In", "Allow sensitive actions", "Create Access Token"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    drop(sim);
    let _ = app.update(UiMessage::AddTokenName("Script".into()));
    let _ = app.update(UiMessage::AddTokenWrite);
    let _ = app.update(UiMessage::AddTokenExpiry(Expiry(0)));
    let _ = app.update(UiMessage::CreateToken);
    let token = app.integrations.add.as_ref().unwrap().token.clone().expect("the token shows once");
    assert!(simulator(app.view_settings()).find(token.as_str()).is_ok());
    let stored = app.store.read().connect.validate(&token, "", "").expect("the token works");
    assert!(stored.scopes == ["read", "write"] && stored.expires_at.is_none());
    let _ =
        app.update(UiMessage::WindowKey(app.settings_window.unwrap(), Key::Named(Named::Escape), Modifiers::empty()));
    assert!(app.integrations.add.is_none() && app.settings_window.is_some(), "Escape closes only the dialog");
    let mut sim = simulator(app.view_settings());
    for label in ["Script", "Sensitive actions", "Created less than a minute ago · Never expires"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    let _ = sim.click("Revoke");
    let msgs: Vec<_> = sim.into_messages().collect();
    msgs.into_iter().for_each(|m| drop(app.update(m)));
    assert!(simulator(app.view_settings()).find("Are you sure you want to revoke this connection?").is_ok());
    let _ = app.update(UiMessage::RevokeConfirmed);
    assert!(app.store.read().connect.validate(&token, "", "").is_err() && app.confirm.is_none());
    assert_eq!(crate::distance(50 * 60_000), "about 1 hour");
    assert_eq!(crate::distance(30 * 86_400_000), "about 1 month");
    assert_eq!(crate::distance(400 * 86_400_000), "about 1 year");
}

#[test]
fn remote_access_like_desktop() {
    let mut app = demo_app("remote");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Integrations));
    let _ = app.update(UiMessage::RemoteAccess(true));
    assert!(
        simulator(app.view_settings()).find("Enable Remote Access?").is_ok()
            && !app.store.read().settings.remote_access
    );
    let _ = app.update(UiMessage::RemoteAccess(true));
    assert!(app.store.read().settings.remote_access && app.confirm.is_none(), "Allow Remote Access turns it on");
    assert_eq!(app.relaunch_toast, Some("Remote Access will be enabled after relaunch"));
    let _ = app.update(UiMessage::RemoteAccess(false));
    assert!(!app.store.read().settings.remote_access && app.confirm.is_none(), "off needs no confirmation");
    app.store.write().config.launch_host = true;
    let mut sim = simulator(app.view_settings());
    let _ = sim.click("Remote Access");
    assert!(sim.into_messages().all(|m| !matches!(m, UiMessage::RemoteAccess(_))), "--pas-hostname locks the toggle");
}

#[test]
fn ai_settings_like_desktop() {
    let mut app = demo_app("ai");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Ai));
    let mut sim = simulator(app.view_settings());
    for heading in ["General", "Voice Transcription", "AI Mentions"] {
        assert!(sim.find(heading).is_ok(), "the {heading} group shows");
    }
    let mention = "Will send the 20 most recent messages, including sender names and any @mentioned users.";
    assert!(sim.find(mention).is_ok() && sim.find("Requires a paid subscription.").is_ok());
    let _ = sim.click("Whisper");
    assert!(sim.into_messages().next().is_none(), "the Plus toggles are locked");
}

#[test]
fn a_sent_link_keeps_its_preview() {
    let mut app = demo_app("preview");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let url = "https://example.com/";
    let edit = text_editor::Edit::Paste(Arc::new(format!("see {url}")));
    let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
    let page = serde_json::json!({ "matched_url": url, "og:title": "Example" });
    app.pane.composer_links = vec![(url.into(), LinkState::Loaded(page, None))];
    let _ = app.update(UiMessage::SubmitComposer);
    let st = app.store.read();
    let links = &st.messages.last().unwrap().links;
    assert_eq!(links.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), ["Example"]);
}

#[test]
fn undo_send_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("undo-send");
    app.store.write().features.flags["features"]["*"]["undo-send"] = true.into();
    let _ = app.update(UiMessage::SetPref("undoSendDelay", 1));
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let send = |app: &mut BeeperGuiApp, text: &str| {
        let edit = text_editor::Edit::Paste(Arc::new(text.into()));
        let _ = app.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
        let _ = app.update(UiMessage::SubmitComposer);
        app.store.read().messages.last().unwrap().id.clone()
    };
    let count = |app: &BeeperGuiApp| app.store.read().chat_msg_indices[NOTE_CHAT_ID].len();
    let before = count(&app);
    send(&mut app, "oops");
    assert_eq!((count(&app), app.queued.len()), (before + 1, 1), "the message waits");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("z".into()), Modifiers::COMMAND));
    assert_eq!(count(&app), before, "Ctrl Z takes the message back");
    assert_eq!(app.pane.composer.text().trim(), "oops", "the composer has the text again");
    let _ = app.update(UiMessage::SetPref("undoOnPendingClick", 1));
    let id = send(&mut app, "");
    let _ = app.update(UiMessage::ClickMessage(id));
    assert_eq!((count(&app), app.pane.composer.text().trim()), (before, "oops"), "a click on the message undoes");
    send(&mut app, "");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("Z".into()), Modifiers::COMMAND | Modifiers::SHIFT));
    assert!(app.queued.is_empty() && count(&app) == before + 1, "Ctrl Shift Z sends now");
    send(&mut app, "later");
    app.queued[0].due = 0;
    let _ = app.update(UiMessage::SendQueued(None));
    assert!(app.queued.is_empty(), "the delay sends");
    app.store.write().features.flags["features"]["*"]["undo-send"] = false.into();
    send(&mut app, "now");
    assert!(app.queued.is_empty(), "the free plan sends at once");
}

#[test]
fn plus_rows_follow_the_feature_flags() {
    let mut app = demo_app("features");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::General));
    let clicks = |app: &BeeperGuiApp, label: &str| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1024.0, 4000.0), app.view_settings());
        assert!(sim.find(label).is_ok(), "{label} shows");
        let _ = sim.click(label);
        sim.into_messages().filter(|m| matches!(m, UiMessage::SetPref(..))).count()
    };
    // Rows with a Plus pill lay out each word of the label apart.
    let undo = "pending";
    let hit = if cfg!(target_os = "macos") {
        "Hit ⌘ Shift Z to send instantly."
    } else {
        "Hit Ctrl Shift Z to send instantly."
    };
    assert!(simulator(app.view_settings()).find(hit).is_ok());
    assert_eq!(clicks(&app, undo), 0, "the free plan locks undo send");
    assert_eq!(clicks(&app, "reminder"), 1, "free reminders are left");
    app.store.write().features.flags["features"]["*"]["undo-send"] = true.into();
    assert_eq!(clicks(&app, undo), 1);
    app.store.write().features.flags = serde_json::Value::Null;
    let mut sim = simulator(app.view_settings());
    assert!(sim.find(undo).is_err() && sim.find("Incognito").is_err(), "unknown features hide their rows");
    assert!(!search_settings(&app.store.read(), "undo").iter().any(|h| matches!(h, SettingsHit::Pref(..))));
}

#[test]
fn every_pref_value_renders() {
    let mut app = demo_app("prefs");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::General));
    for (tab, pref) in prefs() {
        let _ = app.update(UiMessage::SelectSettingsTab(tab));
        for v in 0..=pref.max_value() {
            let _ = app.update(UiMessage::SetPref(pref.key, v));
            let _ = simulator(app.view_settings());
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
    std::thread::scope(|s| {
        for seed in seed..seed + 4 {
            s.spawn(move || run(seed, steps as usize));
        }
    });
}

#[test]
fn zoom_keys_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("zoom-keys");
    for (key, zoom) in [("=", 1.1), ("-", 1.0), ("-", 0.9), ("0", 1.0)] {
        let _ = app.update(UiMessage::KeyPressed(Key::Character(key.into()), Modifiers::CTRL));
        assert_eq!(app.zoom, zoom, "ctrl {key}");
    }
}

#[test]
fn search_finds_custom_items_like_desktop() {
    let mut app = demo_app("search-custom");
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::General));
    let hit =
        |q: &str| search_settings(&app.store.read(), q).into_iter().find(|h| matches!(h, SettingsHit::Custom(..)));
    assert_eq!(hit("wallpaper"), Some(SettingsHit::Custom(SettingsTab::Appearance, "Default chat theme")));
    assert_eq!(hit("dark mode"), Some(SettingsHit::Custom(SettingsTab::Appearance, "Theme Selector")));
    assert_eq!(hit("zoom con"), Some(SettingsHit::Custom(SettingsTab::Appearance, "Zoom Controls")));
    let names = |q: &str| search_settings(&app.store.read(), q).into_iter().map(SettingsHit::label).collect::<Vec<_>>();
    assert!(names("notification sound").contains(&"Group Notification Sound".to_string()));
    assert!(names("sound effect volume").contains(&"Sound Effect Volume".to_string()));
    let mcp = search_settings(&app.store.read(), "mcp");
    assert_eq!(mcp.first(), Some(&SettingsHit::Section(SettingsTab::Integrations, true)), "a section keyword matches");
    let _ = app.update(UiMessage::PickSetting(hit("wallpaper").expect("a hit")));
    assert_eq!(app.active_settings_tab, SettingsTab::Appearance);
    assert_eq!(app.pref_flash.map(|f| f.0), Some("Default chat theme"));
    let _ = simulator(app.view_settings());
}

#[test]
fn polls_like_desktop() {
    let mut app = demo_app("polls");
    let (chat, mid) = ("!tg_rust:beeper.com", "msg_1004");
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let me = app.store.read().me().id.clone();
    let poll = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.messages[st.msg_idx_by_id[mid]].poll.clone().expect("a poll")
    };
    let click = |app: &BeeperGuiApp, label: &str| -> Vec<UiMessage> {
        let mut sim = simulator(app.view_main());
        let _ = sim.click(label).expect("the option");
        sim.into_messages().filter(|m| matches!(m, UiMessage::VotePoll(..))).collect()
    };
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Select one option").is_ok() && sim.find("2 votes").is_ok());
        let st = app.store.read();
        let m = &st.messages[st.msg_idx_by_id[mid]];
        assert_eq!((crate::message_tag(m), crate::preview_body(m)), (Some((crate::POLL_TAG, "Poll", None)), None));
    }
    let mut sim = simulator(app.view_main());
    let _ = sim.click("2 votes").expect("the votes button");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    assert_eq!(app.vote_windows.len(), 1);
    let (id, ..) = app.vote_windows[0];
    assert_eq!(app.title(id), "2 votes");
    {
        let mut sim = simulator(app.view_poll_votes(chat, mid));
        assert!(sim.find("Which default dark theme do you prefer?").is_ok() && sim.find("Dark Purple").is_ok());
    }
    let _ = app.update(UiMessage::WindowClosed(id));
    assert!(app.vote_windows.is_empty());
    for m in click(&app, "Nocturne") {
        let _ = app.update(m);
    }
    assert_eq!((poll(&app).picks(&me), poll(&app).voters()), (vec!["opt_3".to_string()], 3));
    for m in click(&app, "Nocturne") {
        let _ = app.update(m);
    }
    assert!(poll(&app).picks(&me).is_empty(), "a second click takes the vote back");
    app.store.write().update_poll(mid, |p| p.max_selections = 2);
    for label in ["Dracula", "Nocturne"] {
        for m in click(&app, label) {
            let _ = app.update(m);
        }
    }
    assert_eq!(poll(&app).picks(&me), ["opt_2", "opt_3"]);
    assert!(click(&app, "Dark Purple").is_empty(), "the maximum answers are picked");
    app.store.write().update_poll(mid, |p| p.ended = true);
    assert!(simulator(app.view_main()).find("Poll ended").is_ok());
    assert!(click(&app, "Dracula").is_empty(), "an ended poll takes no votes");
    app.store.write().update_poll(mid, |p| (p.ended, p.undisclosed) = (false, true));
    assert!(simulator(app.view_main()).find("Results will be displayed when the poll ends").is_ok());
    let i = app.store.read().chat_idx_by_id[chat];
    app.store.write().update_chat(i, |c| c.polls = None);
    let mut sim = simulator(app.view_main());
    assert!(
        sim.find("Select one or more options").is_err() && sim.find("Nocturne").is_err(),
        "the fallback text shows"
    );
}

#[test]
fn poll_creator_like_desktop() {
    let mut app = demo_app("poll-creator");
    let chat = "!tg_rust:beeper.com".to_string();
    assert!(crate::hidden_actions(&app.store.read()).contains(&6), "Create Poll is behind the plus button");
    let i = app.store.read().chat_idx_by_id[&chat];
    let rules = PollRules { hidden_votes: false, end: true, duplicates: false, max_options: 3, max_length: 200 };
    app.store.write().update_chat(i, |c| c.polls = Some(rules));
    let _ = app.update(UiMessage::OpenPollCreator(chat.clone()));
    let _ = app.update(UiMessage::OpenPollCreator(chat.clone()));
    assert_eq!(app.poll_windows.len(), 1, "a chat has one poll window");
    let id = app.poll_windows[0].id;
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Poll(id, e)));
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_poll_creator(&app.poll_windows[0])).find(s).is_ok();
    assert!(found(&app, "Add an option") && found(&app, "Allow multiple answers"));
    assert!(!found(&app, "Hide results until end of poll"), "the network has no hidden votes");
    edit(&mut app, PollEdit::Question("Best editor?".into()));
    edit(&mut app, PollEdit::Option(0, "Vim".into()));
    edit(&mut app, PollEdit::Option(1, " Vim".into()));
    assert_eq!(app.poll_windows[0].options.len(), 3, "typing in the last option adds one");
    assert!(found(&app, "This option already exists"));
    let _ = app.update(UiMessage::Poll(id, PollEdit::Send));
    assert_eq!(app.poll_windows.len(), 1, "a poll with a duplicate does not send");
    edit(&mut app, PollEdit::Option(1, "Emacs".into()));
    edit(&mut app, PollEdit::Option(2, "Helix".into()));
    assert_eq!(app.poll_windows[0].options.len(), 3, "the network allows 3 options");
    assert!(found(&app, "You have added the maximum number of options") && !found(&app, "Add an option"));
    edit(&mut app, PollEdit::Option(2, String::new()));
    edit(&mut app, PollEdit::Option(2, String::new()));
    assert_eq!(app.poll_windows[0].options.len(), 2, "Backspace in an empty option removes it");
    assert!(found(&app, "You can enter 1 more option"));
    edit(&mut app, PollEdit::Multiple(true));
    edit(&mut app, PollEdit::Send);
    assert!(app.poll_windows.is_empty());
    {
        let st = app.store.read();
        let m = &st.messages[*st.chat_msg_indices[&chat].last().expect("the sent poll")];
        let poll = m.poll.as_ref().expect("a poll message");
        let labels: Vec<&str> = poll.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!((poll.question.as_str(), labels, poll.max_selections), ("Best editor?", vec!["Vim", "Emacs"], 2));
        assert_eq!(m.message_type.as_deref(), Some("POLL"));
    }
    let sent = {
        let st = app.store.read();
        st.messages[*st.chat_msg_indices[&chat].last().expect("the sent poll")].id.clone()
    };
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = app.update(UiMessage::OpenMessageMenu(sent.clone()));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("End Poll").expect("the sender can end the poll");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    {
        let st = app.store.read();
        assert!(st.messages[st.msg_idx_by_id[&sent]].poll.as_ref().expect("a poll").ended);
    }
    let _ = app.update(UiMessage::OpenMessageMenu(sent.clone()));
    assert!(simulator(app.view_main()).find("End Poll").is_err(), "an ended poll has no End Poll");
    assert!(simulator(app.view_main()).find("Edit…").is_err(), "a poll is not edited");
    let _ = app.update(UiMessage::OpenPollCreator(chat.clone()));
    let id = app.poll_windows[0].id;
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Poll(id, e)));
    edit(&mut app, PollEdit::Question("Lunch?".into()));
    let _ = app.update(UiMessage::CloseRequested(id));
    assert!(app.poll_windows[0].discarding && found(&app, "Discard poll?") && found(&app, "Keep editing"));
    edit(&mut app, PollEdit::Discard(false));
    edit(&mut app, PollEdit::Question(String::new()));
    edit(&mut app, PollEdit::Close);
    assert!(!app.poll_windows[0].discarding, "an empty draft closes without a question");
    let _ = app.update(UiMessage::WindowClosed(id));
    assert!(app.poll_windows.is_empty());
}

#[test]
fn chat_theme_like_desktop() {
    let mut app = demo_app("chat-theme");
    let chat = app.store.read().chats[0].id.clone();
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Appearance));
    {
        let mut sim = simulator(app.view_settings());
        assert!(
            sim.find("Pick a wallpaper and bubble color").is_ok() && sim.find("Only you see your chat theme").is_ok()
        );
    }
    let swatch = |id: &str| Some(Bubble::Swatch { id: id.into() });
    let _ = app.update(UiMessage::ChatBubble(None, swatch("blue")));
    let _ = app.update(UiMessage::ChatWallpaper(None, Wallpaper::Gradient { id: "linen".into() }));
    let _ = app.update(UiMessage::ChatWallpaper(Some(chat.clone()), Wallpaper::None));
    let stored = serde_json::to_value(&app.store.read().settings.chat_appearance).expect("json");
    let want = serde_json::json!({
        "global": { "wallpaper": { "type": "gradient", "id": "linen" }, "bubble": { "type": "swatch", "id": "blue" } },
        "perChat": { chat.as_str(): { "wallpaper": { "type": "none" } } },
    });
    assert_eq!(stored, want, "the chat theme is stored like Desktop's chat_appearance");
    let a = app.store.read().settings.chat_appearance.clone();
    let dir = app.store.read().data_dir.clone();
    let (wallpaper, bubble) = crate::chat_theme::resolve(&a, Some(&chat), false, &dir);
    assert!(wallpaper.is_none(), "the chat's own wallpaper wins");
    assert_eq!(
        bubble,
        Some((crate::style::hex(0x3478f6), crate::style::hex(0x1a1a1a))),
        "the default bubble color applies"
    );
    assert!(
        crate::chat_theme::resolve(&a, Some("other"), false, &dir).0.is_some(),
        "other chats use the default wallpaper"
    );
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::ChatWallpaper(Some(chat.clone()), Wallpaper::Color { id: "mint".into() }));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::ChatBubble(None, None));
    let _ = app.update(UiMessage::ChatWallpaper(None, Wallpaper::None));
    let a = app.store.read().settings.chat_appearance.clone();
    assert!(a.theme(None).bubble.is_none(), "the default bubble color clears");
    let _ = app.update(UiMessage::OpenChatTheme(chat.clone()));
    assert_eq!(app.theme_windows.len(), 1);
    let _ = app.update(UiMessage::OpenChatTheme(chat.clone()));
    assert_eq!(app.theme_windows.len(), 1, "a chat has one chat theme window");
    {
        let mut sim = simulator(app.view_chat_theme(&chat));
        assert!(sim.find("Only you see your chat themes.").is_ok());
        let _ = sim.click("Use app default").expect("the chat has its own theme");
        for m in sim.into_messages() {
            let _ = app.update(m);
        }
    }
    assert!(app.store.read().settings.chat_appearance.per_chat.is_empty());
    assert!(simulator(app.view_chat_theme(&chat)).find("Use app default").is_err());
    let picked = dir.join("picked.PNG");
    ::image::RgbaImage::new(1, 1).save_with_format(&picked, ::image::ImageFormat::Png).expect("write a photo");
    let _ = app.update(UiMessage::WallpaperPicked(Some(chat.clone()), Ok(vec![picked])));
    let themed = app.store.read().settings.chat_appearance.clone();
    let Some(Wallpaper::Photo { file_name }) = themed.theme(Some(&chat)).wallpaper else { panic!("a photo wallpaper") };
    assert!(file_name.ends_with(".png") && crate::chat_theme::photo_dir(&dir).join(&file_name).exists());
    let (wallpaper, _) = crate::chat_theme::resolve(&themed, Some(&chat), true, &dir);
    assert!(matches!(wallpaper, Some((crate::chat_theme::Paint::Photo(..), Color::WHITE))));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::ChatWallpaper(Some(chat.clone()), Wallpaper::None));
    assert!(!crate::chat_theme::photo_dir(&dir).join(&file_name).exists(), "an unused photo is removed");
    assert_eq!(
        crate::chat_theme::paint(&a.theme(Some(&chat)).wallpaper.unwrap(), true, &dir).unwrap().0,
        crate::chat_theme::Paint::Flat(Color::from_rgb8(71, 109, 92))
    );
}

#[test]
fn incognito_indicator_like_desktop() {
    let mut app = demo_app("incognito-indicator");
    let trial = serde_json::json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": "2999-01-01T00:00:00Z" } });
    app.store.write().features.usage = trial;
    let _ = app.update(UiMessage::SetPref("incognito", 1));
    let chat = app.store.read().chats.iter().find(|c| c.unread_count > 0).expect("an unread chat").id.clone();
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let unread = |app: &BeeperGuiApp| app.store.read().chats.iter().find(|c| c.id == chat).unwrap().unread_count;
    assert!(unread(&app) > 0, "incognito mode keeps the chat unread");
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("🫣 Unread until you reply").is_ok() && sim.find("New").is_err());
        let days = app.store.read().features.trial_days_left().expect("the trial is open");
        assert!(sim.find(format!("{days} more days of free incognito mode")).is_ok());
        let _ = sim.click("Mark as Read").expect("the link marks the chat read");
        for m in sim.into_messages() {
            let _ = app.update(m);
        }
    }
    assert_eq!(unread(&app), 0);
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(chat.as_str()), "the chat stays open");
    let _ = app.update(UiMessage::SetPref("incognitoIndicator", 0));
    assert!(!app.store.read().incognito_indicator());
}

#[test]
fn labels_in_the_rail_like_desktop() {
    let mut app = demo_app("label-rail");
    let rail = rail_accounts(&app.store.read());
    let first = rail.iter().position(|a| a.starts_with("label:")).expect("a label in the rail");
    assert_eq!(
        rail[first..],
        ["label:lbl_favorite", "label:lbl_vip", "label:lbl_work"],
        "labels follow the accounts by name"
    );
    let _ = app.update(UiMessage::SelectAccount(Some("label:lbl_work".into())));
    {
        let st = app.store.read();
        let work = &st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label").rooms;
        let shown = app.visible_chats(&st);
        assert!(!shown.is_empty() && shown.iter().all(|c| work.contains(&c.id)), "a label shows its chats");
    }
    simulator(app.view_sidebar()).find("Work").expect("the label name is the heading");
    app.store.write().labels.iter_mut().for_each(|l| l.color = Some(40));
    drop(simulator(app.view_rail()));
    app.store.write().features.flags = serde_json::Value::Null;
    assert!(!rail_accounts(&app.store.read()).iter().any(|a| a.starts_with("label:")), "labels-v2 hides labels");
}

#[test]
fn create_label_like_desktop() {
    let mut app = demo_app("create-label");
    let chats: Vec<String> =
        app.store.read().chats.iter().filter(|c| c.label_ids.is_empty()).map(|c| c.id.clone()).take(2).collect();
    let _ = app.update(UiMessage::OpenCreateLabel(Vec::new()));
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Label(e)));
    let found = |app: &BeeperGuiApp, s: &str| {
        simulator(app.view_create_label(app.label_window.as_ref().expect("the label window"))).find(s).is_ok()
    };
    assert!(found(&app, "Chats") && !found(&app, "Selected"));
    edit(&mut app, LabelEdit::Toggle(chats[0].clone()));
    edit(&mut app, LabelEdit::Toggle(chats[1].clone()));
    assert!(found(&app, "Selected"));
    edit(&mut app, LabelEdit::Create);
    assert!(app.label_window.is_some(), "a label needs a title");
    edit(&mut app, LabelEdit::Search("zzzz".into()));
    assert!(found(&app, "Clear search to show all selected chats") && !found(&app, "Selected"));
    edit(&mut app, LabelEdit::Picker(true));
    assert!(found(
        &app,
        "Choose a title, icon, and/or color to customize your label. This is how it will appear throughout the app."
    ));
    edit(&mut app, LabelEdit::Color(3));
    edit(&mut app, LabelEdit::Color(11));
    edit(&mut app, LabelEdit::Icon(DEFAULT_LABEL_ICON));
    edit(&mut app, LabelEdit::Icon("3257-5950"));
    edit(&mut app, LabelEdit::Icon("3257-5950"));
    let _ = app.update(UiMessage::WindowKey(
        app.label_window.as_ref().expect("open").id,
        Key::Named(Named::Escape),
        keyboard::Modifiers::empty(),
    ));
    assert!(app.label_window.as_ref().is_some_and(|d| !d.picker), "Escape closes the picker first");
    edit(&mut app, LabelEdit::Name(" Friends ".into()));
    edit(&mut app, LabelEdit::Create);
    let st = app.store.read();
    let label = st.labels.iter().find(|l| l.name == "Friends").expect("the new label");
    assert_eq!(label.rooms, chats);
    assert_eq!((label.color, label.icon.as_deref()), (Some(11), None), "a second pick clears the icon");
    assert!(chats.iter().all(|c| st.chats[st.chat_idx_by_id[c]].label_ids.contains(&label.id)));
    assert_eq!(app.selected_account_filter, Some(format!("label:{}", label.id)), "the new label opens");
    assert_eq!(app.status_banner, "Created Friends");
    drop(st);
    let _ = app.update(UiMessage::Undo(false));
    assert!(app.store.read().labels.iter().all(|l| l.name != "Friends") && app.selected_account_filter.is_none());
}

#[test]
fn edit_and_delete_labels_like_desktop() {
    let mut app = demo_app("label-menu");
    let _ = app.update(UiMessage::OpenChatMenu("account:label:lbl_work".into()));
    let menu = |app: &BeeperGuiApp| simulator(app.view_account_menu("label:lbl_work")).find("Edit Label…").is_ok();
    assert!(menu(&app));
    let _ = app.update(UiMessage::EditLabel("lbl_work".into()));
    let ready = |app: &BeeperGuiApp| app.label_window.as_ref().expect("the label window").ready(&app.store.read());
    assert!(!ready(&app), "an unchanged label does not save");
    simulator(app.view_create_label(app.label_window.as_ref().expect("open"))).find("Save").expect("a Save button");
    let _ = app.update(UiMessage::Label(LabelEdit::Name("Work Team".into())));
    assert!(ready(&app));
    let _ = app.update(UiMessage::Label(LabelEdit::Create));
    let rooms = {
        let st = app.store.read();
        let work = st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label");
        assert_eq!(work.name, "Work Team");
        work.rooms.clone()
    };
    assert_eq!(app.status_banner, "Updated Work Team");
    let _ = app.update(UiMessage::Undo(false));
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.name == "Work" && l.rooms == rooms));
    let _ = app.update(UiMessage::SelectAccount(Some("label:lbl_work".into())));
    let _ = app.update(UiMessage::AskDeleteLabel("lbl_work".into()));
    simulator(app.view_confirm(ASK_DELETE_LABEL)).find("Delete").expect("a Delete button");
    app.store.write().set_label_muted("lbl_work", true);
    app.patch_settings(|s| s.spaces_order = vec![SpaceOrder { id: "label:lbl_work".into(), visible: true }]);
    let _ = app.update(UiMessage::DeleteLabel("lbl_work".into()));
    {
        let st = app.store.read();
        assert!(st.labels.iter().all(|l| l.id != "lbl_work"));
        assert!(rooms.iter().all(|r| !st.chats[st.chat_idx_by_id[r]].label_ids.iter().any(|l| l == "lbl_work")));
    }
    assert_eq!(app.selected_account_filter, None, "the inbox opens when its label is deleted");
    assert_eq!(app.status_banner, "Deleted \"Work\"");
    let _ = app.update(UiMessage::Undo(false));
    let st = app.store.read();
    let back = st.labels.iter().find(|l| l.name == "Work").expect("Undo makes the label again");
    assert!(back.muted && back.rooms == rooms);
    assert!(rooms.iter().all(|r| st.chats[st.chat_idx_by_id[r]].label_ids.contains(&back.id)));
    assert_eq!(st.settings.spaces_order[0].id, format!("label:{}", back.id), "the label keeps its place");
}

#[test]
fn sidebar_follows_the_spaces_order_like_desktop() {
    use beeper_core::models::SpaceOrder;
    let mut app = demo_app("spaces-order");
    let default = rail_spaces(&app.store.read());
    assert_eq!(default[..2], ["inbox", "archive"]);
    assert!(default.iter().any(|s| s == "label:lbl_work") && !default.iter().any(|s| s == LABEL_NUDGE));
    let order =
        [("archive", true), ("label:lbl_work", true), ("inbox", true), ("reminders", false), ("label:lbl_vip", false)];
    app.store.write().settings.spaces_order = order.map(|(id, visible)| SpaceOrder { id: id.into(), visible }).into();
    let rail = rail_spaces(&app.store.read());
    assert_eq!(rail[..3], ["archive", "label:lbl_work", "inbox"]);
    assert!(!rail.iter().any(|s| s == "reminders" || s == "label:lbl_vip"), "hidden spaces leave the sidebar");
    assert_eq!(rail.len(), default.len() - 2, "the spaces that are not in the order follow");
    let _ = app.run_hotkey("Switch First 9 Spaces Modifier", 1);
    assert_eq!(app.selected_account_filter.as_deref(), Some("label:lbl_work"));
    let _ = app.run_hotkey("Switch First 9 Spaces Modifier", 0);
    assert_eq!((app.selected_account_filter.as_deref(), app.active_tab), (None, InboxTab::Archived));
}

#[test]
fn edit_spaces_like_desktop() {
    let mut app = demo_app("edit-spaces");
    let _ = app.update(UiMessage::OpenChatMenu("account:label:lbl_work".into()));
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu("label:lbl_work"));
        let _ = sim.click("Edit Spaces…").expect("an Edit Spaces item");
        sim.into_messages().filter(|m| matches!(m, UiMessage::OpenEditSpaces)).collect()
    };
    for m in open {
        let _ = app.update(m);
    }
    fn view(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
        simulator(app.view_edit_spaces(app.spaces_window.as_ref().expect("open")))
    }
    {
        let mut sim = view(&app);
        assert!(sim.find("Pinned").is_ok() && sim.find("No Hidden Spaces").is_ok() && sim.find("Work").is_ok());
        assert!(sim.find("Search Spaces").is_ok() && sim.find("New Label").is_ok());
    }
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Spaces(e)));
    let id = app.spaces_window.as_ref().unwrap().id;
    let wheel = mouse::Event::WheelScrolled { delta: ScrollDelta::Lines { x: 0.0, y: -5.0 } };
    for m in interact(&app, id, SPACES_SIZE, Point::new(180.0, 300.0), vec![iced::Event::Mouse(wheel)]) {
        let _ = app.update(m);
    }
    assert!(app.spaces_window.as_ref().unwrap().scrolled, "the search row casts a shadow over a scrolled list");
    edit(&mut app, SpacesEdit::Drag("reminders".into()));
    edit(&mut app, SpacesEdit::Over(String::new()));
    edit(&mut app, SpacesEdit::Drop);
    edit(&mut app, SpacesEdit::Drag("label:lbl_work".into()));
    edit(&mut app, SpacesEdit::Over("inbox".into()));
    edit(&mut app, SpacesEdit::Drop);
    let rail = rail_spaces(&app.store.read());
    assert_eq!(rail[..2], ["label:lbl_work", "inbox"]);
    assert!(!rail.iter().any(|s| s == "reminders"), "a space dropped after Hidden leaves the sidebar");
    assert_eq!(hidden_spaces(&app.store.read()), ["reminders"]);
    assert!(app.status_banner.is_empty(), "Edit Spaces sorts without a toast");
    assert!(view(&app).find("No Hidden Spaces").is_err());
    edit(&mut app, SpacesEdit::Drag(SPACES_DIVIDER.into()));
    edit(&mut app, SpacesEdit::Over("label:lbl_work".into()));
    edit(&mut app, SpacesEdit::Drop);
    assert_eq!(rail_spaces(&app.store.read())[0], "inbox", "the Inbox stays when all spaces are hidden");
    edit(&mut app, SpacesEdit::Search("wor".into()));
    {
        let mut sim = view(&app);
        assert!(sim.find("Work").is_ok() && sim.find("Archive").is_err());
    }
    edit(&mut app, SpacesEdit::Search(String::new()));
    edit(&mut app, SpacesEdit::Menu(Some("label:lbl_work".into())));
    let picked: Vec<UiMessage> = {
        let mut sim = view(&app);
        assert!(
            sim.find("Edit Label…").is_ok() && sim.find("Delete Label").is_ok() && sim.find("Edit Spaces…").is_err()
        );
        let _ = sim.click("Mute").expect("the label gear menu mutes");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.muted));
    assert!(app.spaces_window.as_ref().unwrap().menu.is_none(), "an item closes the menu");
    let account = all_spaces(&app.store.read())
        .into_iter()
        .find(|s| !s.starts_with("label:") && !RAIL_VIEWS.iter().any(|v| v.0 == s) && s != LABEL_NUDGE);
    let muted = {
        let st = app.store.read();
        !st.settings.account(&rail_owner(&st, account.as_ref().unwrap())).notifications
    };
    edit(&mut app, SpacesEdit::Menu(account));
    {
        let mut sim = view(&app);
        assert!(sim.find("Show in Inbox").is_ok() && sim.find(if muted { "Unmute" } else { "Mute" }).is_ok());
        assert!(sim.find("Add/Remove Accounts…").is_err() && sim.find("Mute Space").is_err());
    }
    edit(&mut app, SpacesEdit::Menu(Some("label:lbl_work".into())));
    let ask = "Are you sure you want to delete \"Work\"? This will remove the label from all chats.";
    let picked: Vec<UiMessage> = {
        let mut sim = view(&app);
        let _ = sim.click("Delete Label").expect("the label gear menu deletes");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(simulator(app.view_main()).find(ask).is_err(), "the confirm shows in the Edit Spaces window");
    let picked: Vec<UiMessage> = {
        let mut sim = view(&app);
        let _ = sim.click("Delete").expect("the confirm in the Edit Spaces window");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(app.confirm.is_none() && !app.store.read().labels.iter().any(|l| l.id == "lbl_work"));
    assert!(!app.spaces_window.as_ref().unwrap().order.iter().any(|o| o == "label:lbl_work"));
    edit(&mut app, SpacesEdit::NewLabel);
    assert!(app.label_window.is_some());
}

#[test]
fn label_nudge_hides_after_the_first_label_like_desktop() {
    let mut app = demo_app("label-nudge");
    let nudge = |app: &BeeperGuiApp| rail_spaces(&app.store.read()).iter().any(|s| s == LABEL_NUDGE);
    let ids: Vec<String> = app.store.read().labels.iter().map(|l| l.id.clone()).collect();
    for id in &ids {
        app.store.write().remove_label(id);
    }
    assert!(!nudge(&app), "the demo had labels");
    app.store.write().settings.ever_labeled = false;
    assert!(nudge(&app), "an account that never had labels");
    let _ = app.update(UiMessage::OpenCreateLabel(vec!["!note_self:beeper.com".into()]));
    let _ = app.update(UiMessage::Label(LabelEdit::Name("Friends".into())));
    let _ = app.update(UiMessage::Label(LabelEdit::Create));
    let id = app.store.read().labels[0].id.clone();
    app.store.write().delete_label(&id);
    assert!(app.store.read().labels.is_empty() && !nudge(&app), "the nudge stays hidden after a label");
}

#[test]
fn archive_and_low_priority_toasts_undo_like_desktop() {
    let mut app = demo_app("undo-toast");
    let ids: Vec<String> = {
        let st = app.store.read();
        let chats = app.visible_chats(&st);
        chats.iter().filter(|c| c.is_pinned != Some(true)).take(2).map(|c| c.id.clone()).collect()
    };
    let flag = |app: &BeeperGuiApp, id: &str, f| {
        let st = app.store.read();
        has_flag(&st.chats[st.resolve_chat_idx(id).unwrap()], f)
    };
    let undo = |app: &mut BeeperGuiApp| {
        let picked: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            let _ = sim.click("Undo").expect("the Undo button of the toast");
            sim.into_messages().collect()
        };
        for m in picked {
            let _ = app.update(m);
        }
    };
    let _ = app.update(UiMessage::ToggleFlag(ids[0].clone(), ChatFlag::Archived));
    assert_eq!(app.status_banner, "Archived");
    undo(&mut app);
    assert!(!flag(&app, &ids[0], ChatFlag::Archived) && app.status_banner.is_empty());
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(ids[0].as_str()), "Undo selects the chat");
    app.selection = ids.clone();
    let _ = app.update(UiMessage::SelectionSet(ChatFlag::LowPriority, true));
    assert_eq!(app.status_banner, "Moved 2 chats to Low Priority");
    assert!(ids.iter().all(|id| flag(&app, id, ChatFlag::LowPriority)));
    undo(&mut app);
    assert!(ids.iter().all(|id| !flag(&app, id, ChatFlag::LowPriority)));
    assert_eq!(app.selection, ids, "Undo selects the chats again");
    let _ = app.update(UiMessage::SelectionSet(ChatFlag::Archived, true));
    assert!(app.status_banner == "Archived 2 chats" && app.toast.as_ref().unwrap().icon.is_none());
    let _ = app.update(UiMessage::Undo(false));
    assert!(ids.iter().all(|id| !flag(&app, id, ChatFlag::Archived)) && app.selection == ids, "Mod-z runs the Undo");
    assert!(app.shown_undo().is_none());
}

#[test]
fn toasts_stack_like_desktop() {
    let mut app = demo_app("toast-stack");
    let ids: Vec<String> = {
        let st = app.store.read();
        let chats = app.visible_chats(&st);
        chats.iter().filter(|c| c.is_pinned != Some(true)).take(2).map(|c| c.id.clone()).collect()
    };
    let archived = |app: &BeeperGuiApp| {
        let st = app.store.read();
        has_flag(&st.chats[st.resolve_chat_idx(&ids[0]).unwrap()], ChatFlag::Archived)
    };
    let _ = app.update(UiMessage::ToggleFlag(ids[0].clone(), ChatFlag::Archived));
    let _ = app.update(UiMessage::ToggleFlag(ids[1].clone(), ChatFlag::LowPriority));
    assert_eq!(app.older_toasts.iter().map(|t| t.0.text.as_str()).collect::<Vec<_>>(), ["Archived"]);
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        sim.find("Archived").expect("the older toast");
        let _ = sim.click("Undo").expect("the Undo button of the older toast");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(!archived(&app) && app.older_toasts.is_empty(), "the Undo of an older toast closes it");
    assert!(!app.status_banner.is_empty(), "the newest toast stays");
    let _ = app.update(UiMessage::ToggleFlag(ids[0].clone(), ChatFlag::Archived));
    let _ = app.update(UiMessage::Undo(false));
    assert!(!archived(&app) && app.older_toasts.len() == 1, "Mod-z closes the newest toast and keeps the others");
    let _ = app.update(UiMessage::ToastHover(true));
    assert!(!app.toast_running(), "the toasts wait while the pointer is on them");
    let _ = app.update(UiMessage::ToastHover(false));
    for _ in 0..TOAST_TIMEOUT.as_millis() / TOAST_TICK.as_millis() {
        let _ = app.update(UiMessage::ToastTick);
    }
    assert!(app.older_toasts.is_empty() && app.status_banner.is_empty(), "each toast closes after its timeout");
}

#[test]
fn sidebar_spaces_drag_like_desktop() {
    let mut app = demo_app("rail-drag");
    let hide = SpaceOrder { id: "requests".into(), visible: false };
    app.patch_settings(|s| s.spaces_order = vec![hide.clone()]);
    let press = |app: &mut BeeperGuiApp, moves: &[f32]| {
        let _ = app.update(UiMessage::RailHover("inbox".into(), Point::new(18.0, 18.0)));
        let _ = app.update(UiMessage::RailPress);
        for &y in moves {
            let _ = app.update(UiMessage::RailDragMoved(Point::new(34.0, y)));
        }
    };
    press(&mut app, &[32.0, 34.0]);
    let _ = app.update(UiMessage::RailDragEnd);
    assert_eq!(app.store.read().settings.spaces_order, std::slice::from_ref(&hide), "a click does not sort");
    press(&mut app, &[32.0, 60.0]);
    let _ = app.update(UiMessage::RailHover("archive".into(), Point::new(18.0, 4.0)));
    let _ = app.view_rail();
    let _ = app.update(UiMessage::RailDragEnd);
    let st = app.store.read();
    assert_eq!(rail_spaces(&st)[..2], ["archive", "inbox"]);
    assert_eq!(st.settings.spaces_order.last(), Some(&hide), "the hidden spaces stay hidden");
}

#[test]
fn create_label_shows_a_draft_space_like_desktop() {
    let mut app = demo_app("label-draft");
    let rail = |app: &mut BeeperGuiApp, name: &str| {
        let _ = app.update(UiMessage::Label(LabelEdit::Name(name.into())));
        let _ = app.view_rail();
        let d = app.label_window.as_ref().expect("Create Label is open");
        draft_rail_spaces(&app.store.read(), &d.name)
    };
    let _ = app.update(UiMessage::OpenCreateLabel(Vec::new()));
    let ids = rail(&mut app, "Aaa");
    let at = ids.iter().position(|s| s == LABEL_DRAFT).expect("the draft");
    assert!(ids[at + 1].starts_with("label:"), "the draft sorts before the other labels");
    assert_eq!(rail(&mut app, "Zzz").last().map(String::as_str), Some(LABEL_DRAFT));
    let ids: Vec<String> = app.store.read().labels.iter().map(|l| l.id.clone()).collect();
    for id in &ids {
        app.store.write().remove_label(id);
    }
    app.store.write().settings.ever_labeled = false;
    let order = [LABEL_NUDGE, "inbox"].map(|id| SpaceOrder { id: id.into(), visible: true });
    app.store.write().settings.spaces_order = order.into();
    let ids = rail(&mut app, "Friends");
    assert!(ids[0] == LABEL_DRAFT && !ids.iter().any(|s| s == LABEL_NUDGE), "the draft takes the slot of the nudge");
}

#[test]
fn sidebar_menus_open_accounts_like_desktop() {
    let mut app = demo_app("rail-menus");
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    for entry in [account.as_str(), "label:lbl_work"] {
        let _ = app.update(UiMessage::OpenChatMenu(format!("account:{entry}")));
        let mut sim = simulator(app.view_account_menu(entry));
        assert!(entry.starts_with("label:") || sim.find("Mute Space").is_ok());
        let _ = sim.click("Add/Remove Accounts…").expect("an Add/Remove Accounts item");
        let open = sim.into_messages().find(|m| matches!(m, UiMessage::OpenSettings(SettingsTab::Accounts)));
        let _ = app.update(open.expect("the Accounts settings"));
        assert!(app.chat_menu.is_none() && app.settings_window.is_some());
    }
    let _ = app.update(UiMessage::OpenChatMenu(format!("account:{account}")));
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu(&account));
        let _ = sim.click("Account Settings…").expect("an Account Settings item");
        sim.into_messages().collect()
    };
    for m in open {
        let _ = app.update(m);
    }
    assert_eq!(app.account_page.as_deref(), Some(account.as_str()));
    assert!(app.chat_menu.is_none() && matches!(app.active_settings_tab, SettingsTab::Accounts));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Sound").is_ok(), "the account page");
}

#[test]
fn account_status_like_desktop() {
    let mut app = demo_app("account-status");
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    assert_eq!(status_text(&app.store.read(), &account), None, "a connected login has no status");
    for (state, text) in [
        ("TRANSIENT_DISCONNECT", "Connection Interrupted"),
        ("UNKNOWN_ERROR", "Disconnected"),
        ("UNCONFIGURED", "Connection Required"),
        ("BAD_CREDENTIALS", "Reconnect Required"),
    ] {
        app.store.write().bridge_accounts[0].state = state.into();
        assert_eq!(status_text(&app.store.read(), &account), Some(text));
    }
    let _ = app.update(UiMessage::OpenSettings(SettingsTab::Accounts));
    let _ = simulator(app.view_settings());
    let _ = simulator(app.view_rail());
}

#[test]
fn sidebar_view_menus_like_desktop() {
    let mut app = demo_app("view-menus");
    let open =
        interact(&app, app.main_window.expect("a main window"), Size::new(1440.0, 900.0), Point::new(34.0, 30.0), {
            let right = [mouse::Event::ButtonPressed(Button::Right), mouse::Event::ButtonReleased(Button::Right)];
            right.map(iced::Event::Mouse).to_vec()
        });
    for m in open {
        let _ = app.update(m);
    }
    assert_eq!(app.chat_menu.as_ref().map(|m| m.0.as_str()), Some("account:inbox"));
    let pick = |app: &mut BeeperGuiApp, item: &str| {
        let msgs: Vec<UiMessage> = {
            let mut sim = simulator(app.view_account_menu("inbox"));
            assert!(sim.find("Add/Remove Accounts…").is_ok() && sim.find("Edit Spaces…").is_ok());
            let _ = sim.click(item).expect(item);
            sim.into_messages().collect()
        };
        for m in msgs {
            let _ = app.update(m);
        }
    };
    pick(&mut app, "Hide Space Bar");
    assert!(!app.store.read().settings.on("showSidebar") && app.chat_menu.is_none());
    pick(&mut app, "Show Space Bar");
    assert!(app.store.read().settings.on("showSidebar"));
    pick(&mut app, "Settings");
    assert!(app.settings_window.is_some() && matches!(app.active_settings_tab, SettingsTab::General));
}

#[test]
fn background_menus_like_desktop() {
    let mut app = demo_app("background-menus");
    let main = app.main_window.expect("a main window");
    let right_click = |app: &mut BeeperGuiApp, at: Point| {
        let right = [mouse::Event::ButtonPressed(Button::Right), mouse::Event::ButtonReleased(Button::Right)];
        for m in interact(app, main, Size::new(1440.0, 900.0), at, right.map(iced::Event::Mouse).to_vec()) {
            let _ = app.update(m);
        }
        app.chat_menu.as_ref().map(|m| m.0.clone())
    };
    assert_eq!(right_click(&mut app, Point::new(62.0, 300.0)).as_deref(), Some("account:inbox"), "the space bar");
    let _ = app.update(UiMessage::CloseModals);
    assert_eq!(right_click(&mut app, Point::new(72.0, 300.0)).as_deref(), Some(SIDEBAR_MENU), "the sidebar");
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(SIDEBAR_MENU, false, 900.0));
        for item in ["Add/Remove Accounts", "Hide Space Bar", "Settings"] {
            assert!(sim.find(item).is_ok(), "{item}");
        }
        let _ = sim.click("Start New Chat").expect("Start New Chat");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(app.creating_chat.is_some() && app.chat_menu.is_none());
    let _ = app.update(UiMessage::CloseModals);

    assert_eq!(right_click(&mut app, Point::new(100.0, 26.0)).as_deref(), Some("head:inbox"), "the header");
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu("head:inbox", false, 900.0));
        assert!(sim.find("Show in Inbox").is_err());
        let _ = sim.click("Select Chats").expect("Select Chats");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(app.select_mode && app.selection.is_empty() && app.chat_menu.is_none());
    let first = app.list_ids(&app.store.read())[0].clone();
    let _ = app.update(UiMessage::ToggleSelected(first.clone()));
    assert!(!app.select_mode && app.selection == [first.clone()]);
    let _ = app.update(UiMessage::ToggleSelected(first));
    assert!(app.selection.is_empty() && !app.select_mode, "the mode ends with the last chat");
    let _ = app.update(UiMessage::SelectAccount(Some("label:lbl_work".into())));
    let has = |app: &BeeperGuiApp, key: &str, items: &[&str]| {
        let mut sim = simulator(app.view_chat_menu(key, false, 900.0));
        items.iter().all(|i| sim.find(*i).is_ok())
    };
    assert!(has(&app, "head:label:lbl_work", &["Select Chats", "Show in Inbox", "Edit Label...", "Delete Label"]));
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    assert!(has(&app, &format!("head:{account}"), &["Select Chats", "Show in Inbox", "Account Settings..."]));
    let _ = app.update(UiMessage::SelectAccount(None));

    let chat = app.store.read().chats[0].id.clone();
    let _ = app.update(UiMessage::ClickChat(chat.clone()));
    let at = Point::new(68.0 + SIDEBAR_WIDTH + 40.0, 60.0);
    assert_eq!(right_click(&mut app, at), Some(chat.clone()), "the chat header has the chat menu");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::ToggleCommandPalette);
    let _ = app.update(UiMessage::OpenChatMenu(chat));
    assert!(simulator(app.view_main()).find("Begin Selecting").is_ok(), "the chat menu shows over the command bar");
    let _ = app.update(UiMessage::CloseChatMenu);
    assert!(app.show_command_palette && app.chat_menu.is_none());
}

#[test]
fn message_requests_like_desktop() {
    let mut app = demo_app("message-requests");
    let dms: Vec<String> = {
        let st = app.store.read();
        let dm = |c: &&Chat| c.chat_type == "single" && c.id != NOTE_CHAT_ID && c.is_pinned != Some(true);
        st.chats.iter().filter(dm).take(2).map(|c| c.id.clone()).collect()
    };
    for id in &dms {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(id).expect("a chat");
        st.update_chat(idx, |c| {
            (c.is_request, c.can_accept_request, c.delete_modes) = (Some(true), true, DELETE_FOR_ME)
        });
    }
    let (first, second) = (&dms[0], &dms[1]);
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(first, false, 900.0));
        assert!(sim.find("Decline message request").is_ok());
        let _ = sim.click("Accept message request").expect("Accept message request");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(first).expect("the accepted chat")];
        assert!(c.is_request == Some(false) && c.flags & CHAT_REQUEST == 0 && app.chat_menu.is_none());
    }
    let _ = app.update(UiMessage::ClickChat(second.clone()));
    let heading = {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(second).expect("a request")];
        let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).expect("the other person");
        let id = handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone());
        format!("Accept message request from {}?", id.map_or(member_name(u), |id| format!("{} ({id})", member_name(u))))
    };
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_conversation());
        assert!(sim.find(heading.as_str()).is_ok(), "{heading}");
        assert!(sim.find("Accept").is_ok());
        let _ = sim.click("Decline").expect("Decline");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert!(app.store.read().resolve_chat_idx(second).is_none(), "Decline deletes the chat");
}

#[test]
fn sidebar_overflow_menu_lists_hidden_spaces_like_desktop() {
    let mut app = demo_app("overflow-menu");
    let hide = |id: &str| SpaceOrder { id: id.into(), visible: false };
    app.patch_settings(|s| s.spaces_order = vec![hide("requests"), hide("label:lbl_work")]);
    let unread = app.store.read().chats.iter().any(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0);
    let _ = app.update(UiMessage::OpenChatMenu(RAIL_MORE.into()));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(RAIL_MORE, false, 900.0));
        assert!(sim.find(if unread { "• Requests" } else { "Requests" }).is_ok());
        assert!(sim.find("New Label…").is_ok() && sim.find("Edit Spaces…").is_ok());
        let _ = sim.click("Work").expect("the hidden label");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert_eq!(app.selected_account_filter.as_deref(), Some("label:lbl_work"));
    assert!(app.chat_menu.is_none());
}

#[test]
fn chat_menu_acts_on_the_selection_like_desktop() {
    let mut app = demo_app("menu-selection");
    let pick = |app: &BeeperGuiApp, unread: bool| {
        let st = app.store.read();
        let visible = app.visible_chats(&st);
        let c = visible.iter().find(|c| c.is_pinned != Some(true) && has_flag(c, ChatFlag::MarkedUnread) == unread);
        c.expect("a listed chat").id.clone()
    };
    let (unread, read) = (pick(&app, true), pick(&app, false));
    app.selection = vec![unread.clone(), read.clone()];
    let _ = app.update(UiMessage::OpenChatMenu(read.clone()));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(&read, false, 900.0));
        assert!(sim.find("2 chats selected").is_ok() && sim.find("Mark All as Unread").is_ok());
        assert!(sim.find("Mark as Read").is_err() && sim.find("Pin").is_ok());
        let _ = sim.click("Mark All as Read").expect("a Mark All as Read item");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    let st = app.store.read();
    let is_unread = |id: &str| has_flag(&st.chats[st.resolve_chat_idx(id).unwrap()], ChatFlag::MarkedUnread);
    assert!(!is_unread(&unread) && !is_unread(&read));
    assert!(app.chat_menu.is_none() && app.selection.is_empty());
}

#[test]
fn label_inbox_rules_like_desktop() {
    let mut app = demo_app("label-inbox");
    let _ = app.update(UiMessage::OpenChatMenu("account:label:lbl_work".into()));
    let _ = app.update(UiMessage::ChatSub(Some(0)));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu("label:lbl_work"));
        let _ = sim.click("Only DMs").expect("the Only DMs option");
        sim.into_messages().collect()
    };
    assert!(matches!(&picks[..], [UiMessage::SetLabelInbox(id, Some("dmOnly"))] if id == "lbl_work"));
    let hidden = |app: &BeeperGuiApp| {
        let st = app.store.read();
        let work = st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label");
        let chats = work.rooms.iter().map(|r| &st.chats[st.chat_idx_by_id[r]]);
        chats.map(|c| (c.inbox_group, c.flags & CHAT_HIDDEN != 0)).collect::<Vec<_>>()
    };
    let shown = hidden(&app);
    assert!(!shown.is_empty() && shown.iter().all(|&(_, h)| !h));
    for m in picks {
        let _ = app.update(m);
    }
    assert!(hidden(&app).iter().all(|&(group, h)| group == h), "Only DMs hides group chats");
    assert!(app.chat_menu.is_none());
    let _ = app.update(UiMessage::SetLabelInbox("lbl_work".into(), Some("none")));
    assert!(hidden(&app).iter().all(|&(_, h)| h), "None hides all label chats");
    let _ = app.update(UiMessage::SetLabelInbox("lbl_work".into(), None));
    assert_eq!(hidden(&app), shown);
}

#[test]
fn label_chats_from_the_chat_menu_like_desktop() {
    let mut app = demo_app("chat-labels");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::OpenChatMenu(chat.into()));
    let _ = app.update(UiMessage::ChatSub(Some(2)));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(chat, false, 900.0));
        assert!(sim.find("Label").is_ok() && sim.find("New Label…").is_ok());
        let _ = sim.click("Work").expect("the Work label");
        sim.into_messages().collect()
    };
    assert!(matches!(&picks[..], [UiMessage::ToggleLabel(l, c, true)] if l == "lbl_work" && c == chat));
    for m in picks {
        let _ = app.update(m);
    }
    let labels = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.chat_idx_by_id[chat]].label_ids.clone()
    };
    assert_eq!(labels(&app), ["lbl_work"]);
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.rooms.iter().any(|r| r == chat)));
    assert_eq!(app.status_banner, "Added to Work");
    let _ = app.update(app.toast.as_ref().and_then(|t| t.open.clone()).expect("the toast opens the label"));
    assert_eq!(app.selected_account_filter.as_deref(), Some("label:lbl_work"));
    let _ = app.update(UiMessage::Undo(false));
    assert!(labels(&app).is_empty());
    assert_eq!(app.status_banner, "Undo: Add to Work");
    assert!(app.toast.is_none() && app.undo_stack.is_empty(), "the undo is not an action");
    app.focused = true;
    for _ in 0..(UNDO_NOTE_TIMEOUT.as_millis() / TOAST_TICK.as_millis()) {
        let _ = app.update(UiMessage::ToastTick);
    }
    assert!(app.status_banner.is_empty(), "the Undo: toast closes after a second");
    let _ = app.update(UiMessage::ToggleLabel("lbl_work".into(), chat.into(), true));
    let _ = app.update(UiMessage::ToggleLabel("lbl_work".into(), chat.into(), false));
    assert!(labels(&app).is_empty());
    assert!(app.status_banner == "Removed from Work" && app.toast.as_ref().is_some_and(|t| t.open.is_none()));
    let _ = app.update(UiMessage::OpenCreateLabel(vec![chat.into()]));
    assert_eq!(app.label_window.as_ref().expect("the label window").chats, [chat]);
}

/// A simulator tall enough for the whole Chat Info panel.
fn tall<'a>(el: El<'a>) -> Simulator<'a, UiMessage> {
    Simulator::with_size(Default::default(), Size::new(1024.0, 2000.0), el)
}

#[test]
fn chat_info_labels_like_desktop() {
    let mut app = demo_app("chat-info-labels");
    let chat = "!note_self:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_info());
        assert!(sim.find("Labels").is_ok());
        let _ = sim.click("Add to a Label").expect("an Add to a Label button");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::OpenEditLabels(c)] if *c == chat));
    for m in msgs {
        let _ = app.update(m);
    }
    let _ = app.update(UiMessage::OpenEditLabels(chat.clone()));
    assert_eq!(app.labels_windows.len(), 1, "one window for each chat");
    {
        let mut sim = simulator(app.view_edit_labels(&chat));
        assert!(sim.find("Edit Labels").is_ok() && sim.find("Work").is_ok());
        let _ = sim.click("New Label").expect("a New Label button");
        assert!(
            matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::OpenCreateLabel(c)] if c.is_empty())
        );
    }
    let _ = app.update(UiMessage::SwitchLabel("lbl_work".into(), chat.clone(), true));
    let st = app.store.read();
    assert_eq!(st.chats[st.chat_idx_by_id[&chat]].label_ids, ["lbl_work"]);
    drop(st);
    assert!(app.toast.is_none() && app.undo_stack.len() == 1, "the switch has no toast but can be undone");
    let mut sim = simulator(app.view_info());
    assert!(sim.find("Work").is_ok() && sim.find("Add to a Label").is_err());
}

#[test]
fn chat_info_disappearing_messages_like_desktop() {
    let mut app = demo_app("chat-info-timer");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Disappearing messages").is_ok());
        assert!(sim.find("For added privacy, new messages disappear after the set time.").is_ok());
        sim.into_messages().collect()
    };
    assert!(msgs.is_empty());
    let _ = app.update(UiMessage::SetDisappearing(id.clone(), 604_800_000));
    let disappearing = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.chat_idx_by_id[&id]].disappearing
    };
    assert_eq!(disappearing(&app), Some(604_800_000));
    let _ = app.update(UiMessage::SetDisappearing(id.clone(), 0));
    assert_eq!(disappearing(&app), None);
    app.store.write().features.flags["features"]["*"]["enable-disappearing-message-timer-config"] = false.into();
    assert!(tall(app.view_info()).find("Disappearing messages").is_err(), "the flag hides the section");
}

#[test]
fn disappearing_timer_changes_in_the_timeline_like_desktop() {
    let mut app = demo_app("timer-notice");
    let id = "!tg_rust:beeper.com".to_string();
    {
        let mut st = app.store.write();
        let last = st.chat_msg_indices[&id].last().map(|&i| st.messages[i].clone()).expect("a message");
        let me = st.chats[st.chat_idx_by_id[&id]].participants.items.iter().find(|u| u.is_self == Some(true));
        let me = me.expect("the user").id.clone();
        for (n, (sender, timer)) in
            [(me.as_str(), 3_600_000), (me.as_str(), 604_800_000), ("@tulir:beeper.com", 0)].into_iter().enumerate()
        {
            st.upsert_message(Message {
                id: format!("timer_{n}"),
                sender_id: sender.into(),
                sort_key: beeper_core::store::sort_key(
                    last.sort_key.parse::<i64>().expect("a sort key") + 1 + n as i64,
                ),
                message_type: Some("DISAPPEARING_TIMER".into()),
                timer: Some(timer),
                ..last.clone()
            });
        }
    }
    let _ = app.update(UiMessage::SelectChat(id));
    let mut sim = tall(app.view_conversation());
    assert!(sim.find("You set new messages to disappear after 1 hour.").is_err(), "a later change by the same sender");
    assert!(sim.find("You set new messages to disappear after 7 days.").is_ok());
    assert!(sim.find("Tulir Asokan turned off disappearing messages.").is_ok());
    let _ = sim.click("Change Setting").expect("Change Setting on the last change");
    assert!(matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::ToggleRightPanel]));
}

#[test]
fn chat_info_member_actions_like_desktop() {
    let mut app = demo_app("chat-info-member-actions");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let labels = |app: &BeeperGuiApp, uid: &str| -> Vec<(String, bool)> {
        let menu = info_menu(&app.store.read(), &format!("member:{id} {uid}"), |_| true);
        menu.into_iter().flatten().map(|(l, m)| (l, m.is_some())).collect()
    };
    let tulir = labels(&app, "@tulir:beeper.com");
    assert!(tulir.contains(&("Send Message".into(), true)));
    assert!(tulir.ends_with(&[("Make Group Admin…".into(), true), ("Remove from Group…".into(), true)]));
    let me = labels(&app, "@self_telegram:beeper.com");
    assert!(me.ends_with(&[("Make Group Admin…".into(), false), ("Remove from Group…".into(), false)]));
    let ask = |a| UiMessage::AskMember(a, id.clone(), "@tulir:beeper.com".into());
    let _ = app.update(ask(ASK_MAKE_ADMIN));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Are you sure you want to make Tulir Asokan an admin of this group?").is_ok());
        let _ = sim.click("Confirm").expect("the confirm button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.confirm.is_none());
    assert!(labels(&app, "@tulir:beeper.com").contains(&("Make Group Admin…".into(), false)));
    let _ = app.update(ask(ASK_REMOVE_MEMBER));
    assert!(simulator(app.view_main()).find("Are you sure you want to remove Tulir Asokan from this group?").is_ok());
    let _ = app.update(UiMessage::RemoveMember(id.clone(), "@tulir:beeper.com".into()));
    assert!(tall(app.view_info()).find("Tulir Asokan").is_err());
}

#[test]
fn member_commands_like_desktop() {
    use crate::commands::{SubMenu, MEMBERS_ADMIN, MEMBERS_DISMISS, MEMBERS_KICK};
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("member-commands");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let cmds = app.commands(&app.store.read(), "");
    let chat: Vec<&str> = cmds.iter().filter(|c| c.category == "Chat").map(|c| c.title.as_str()).collect();
    let at = chat.iter().position(|t| *t == "Remove member from group").expect("the kick command");
    let want = ["Remove member from group", "Leave chat", "Make group admin", "Dismiss group admin", "Edit group"];
    assert_eq!(chat[at..at + 5], want);
    assert!(chat[at + 5].ends_with("chat info"));
    let users = |app: &mut BeeperGuiApp, kind| -> Vec<String> {
        app.cmd_submenu = Some(SubMenu::Members(kind));
        app.commands(&app.store.read(), "").into_iter().filter_map(|c| c.user.map(|u| u.id)).collect()
    };
    assert_eq!(users(&mut app, MEMBERS_KICK), ["@tulir:beeper.com", "@batuhan:beeper.com"]);
    assert_eq!(users(&mut app, MEMBERS_DISMISS), Vec::<String>::new());
    app.cmd_submenu = None;
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::OpenSubMenu(SubMenu::Members(MEMBERS_ADMIN)));
    let _ = app.update(UiMessage::CommandQueryChanged("tulir".into()));
    assert!(simulator(app.view_main()).find("Tulir Asokan").is_ok());
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::OpenSubMenu(SubMenu::Members(MEMBERS_DISMISS)));
    assert_eq!(users(&mut app, MEMBERS_DISMISS), ["@tulir:beeper.com"]);
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let st = app.store.read();
    let c = &st.chats[st.resolve_chat_idx(&id).expect("the chat")];
    let tulir = c.participants.items.iter().find(|u| u.id == "@tulir:beeper.com").expect("tulir");
    assert_eq!(tulir.is_admin, Some(false));
}

#[test]
fn add_members_like_desktop() {
    use crate::commands::SubMenu;
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("add-members");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        let _ = sim.click("Add Members").expect("the Add Members button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(app.cmd_submenu, Some(SubMenu::AddMembers));
    let chat = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.resolve_chat_idx(&id).expect("the chat")].clone()
    };
    let account = chat(&app).account_id;
    let cmds = app.commands(&app.store.read(), "");
    assert!(!cmds.is_empty() && cmds.iter().all(|c| c.category == "Chats"));
    let members: Vec<String> = chat(&app).participants.items.iter().map(|u| u.id.clone()).collect();
    assert!(cmds.iter().all(|c| !members.contains(&c.user.as_ref().expect("a user row").id)));
    let pick = cmds[0].user.clone().expect("a user row");
    assert!(simulator(app.view_main()).find("Add members to chat").is_ok());
    let found = User { id: "@found:beeper.com".into(), full_name: Some("Found".into()), ..Default::default() };
    let _ = app.update(UiMessage::CommandQueryChanged("found".into()));
    app.new_chat_search = Some(("found".into(), 0, vec![(account, found)]));
    let cmds = app.commands(&app.store.read(), "found");
    assert_eq!(cmds.iter().map(|c| (c.category, c.title.as_str())).collect::<Vec<_>>(), [("Connections", "Found")]);
    let _ = app.update(UiMessage::CommandQueryChanged(pick.full_name.clone().expect("a name")));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(chat(&app).participants.items.iter().any(|u| u.id == pick.id));
    assert!(!app.show_command_palette);
}

#[test]
fn export_chat_like_desktop() {
    let mut app = demo_app("export-chat");
    let _ = app.update(UiMessage::SelectChat("!tg_rust:beeper.com".into()));
    let cmds = app.commands(&app.store.read(), "");
    let at = cmds.iter().position(|c| c.title == "Send GIF").expect("the GIF command");
    assert_eq!(cmds[at + 1].title, "Export all loaded messages to .txt file");
    let st = app.store.read();
    let text = app.export_text(&st).expect("loaded messages");
    assert!(text.starts_with('[') && text.contains("] Tulir Asokan: "));
    assert_eq!(crate::safe_file_name("a/b: c?. "), "ab c");
}

#[test]
fn mute_account_command_like_desktop() {
    let mut app = demo_app("mute-account-command");
    let _ = app.update(UiMessage::SelectChat("!tg_rust:beeper.com".into()));
    let account = |app: &BeeperGuiApp| {
        let cmds = app.commands(&app.store.read(), "");
        cmds.into_iter().find(|c| c.category == "Telegram Account").expect("the account command")
    };
    let mute = account(&app);
    assert!(mute.title.starts_with("Mute account: "));
    let _ = app.update(mute.msg.expect("an action"));
    let st = app.store.read();
    let a = &st.chats[st.resolve_chat_idx("!tg_rust:beeper.com").expect("the chat")].account_id;
    assert!(!st.settings.account(a).notifications);
    drop(st);
    assert!(account(&app).title.starts_with("Unmute account: "));
}

#[test]
fn member_card_like_desktop() {
    let mut app = demo_app("member-card");
    let id = "!tg_rust:beeper.com";
    let _ = app.update(UiMessage::SelectChat(id.into()));
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        let _ = sim.click("Batuhan").expect("the member row");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(app.chat_menu.as_ref().map(|m| m.0.as_str()), Some("card:!tg_rust:beeper.com @batuhan:beeper.com"));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("@batuhan").is_ok());
        let _ = sim.click("Send Message").expect("the Send Message button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.chat_menu.is_none());
    assert_eq!(app.sticky_banner.as_deref(), Some("Composing chat..."), "Send Message makes the direct chat");
}

#[test]
fn new_group_like_desktop() {
    let mut app = demo_app("new-group");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let (account, people) = {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx("!tg_rust:beeper.com").expect("the chat")];
        let others = c.participants.items.iter().filter(|u| u.is_self != Some(true)).cloned();
        (c.account_id.clone(), others.collect::<Vec<User>>())
    };
    let _ = app.update(UiMessage::NewChat);
    let click = |app: &mut BeeperGuiApp, label: &str| {
        let msgs: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            let _ = sim.click(label).expect("the button");
            sim.into_messages().collect()
        };
        for m in msgs {
            let _ = app.update(m);
        }
    };
    click(&mut app, "New Group");
    assert!(app.new_chats[0].group && !found(&app, "New Group") && found(&app, "Start group chat"));
    let _ = app.update(UiMessage::PickUser(account.clone(), Box::new(people[0].clone())));
    let _ = app.update(UiMessage::GroupTitle("Rustaceans".into()));
    assert_eq!(app.new_chats[0].title, "Rustaceans");
    assert!(found(&app, "Rustaceans"), "the name bar shows when the bridge allows a name");
    click(&mut app, "Start group chat");
    assert!(app.sticky_banner.is_none(), "the demo bridge needs two people");
    let _ = app.update(UiMessage::PickUser(account, Box::new(people[1].clone())));
    click(&mut app, "Start group chat");
    assert_eq!(app.sticky_banner.as_deref(), Some("Creating group chat..."));
}

#[test]
fn chat_info_contact_like_desktop() {
    let mut app = demo_app("chat-info-contact");
    let id = "!im_jordan:beeper.com";
    let _ = app.update(UiMessage::SelectChat(id.into()));
    {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Contact").is_ok() && sim.find("Email").is_ok() && sim.find("Username").is_ok());
        assert!(sim.find("Phone").is_err(), "Jordan has no phone number");
    }
    let menu = |kind: &str| -> Vec<String> {
        let items = info_menu(&app.store.read(), &format!("contact:{id} {kind}"), |_| true);
        items.into_iter().flatten().map(|i| i.0).collect()
    };
    assert_eq!(menu("email"), ["Email jordan@example.com", "Copy jordan@example.com"]);
    assert_eq!(menu("username"), ["Copy Username"]);
    let _ = app.update(UiMessage::SelectChat("!tg_rust:beeper.com".into()));
    assert!(tall(app.view_info()).find("Contact").is_err(), "groups have no Contact section");
    let sarah = "!im_sarah:beeper.com";
    let _ = app.update(UiMessage::SelectChat(sarah.into()));
    assert!(tall(app.view_info()).find("Message Other Accounts").is_err());
    {
        let mut st = app.store.write();
        let other = |c: &Chat| c.participants.items.iter().position(|u| u.is_self != Some(true)).expect("a member");
        let from = &st.chats[st.chat_idx_by_id[sarah]];
        let user = from.participants.items[other(from)].clone();
        let to = st.chat_idx_by_id["!sg_lena:beeper.com"];
        let at = other(&st.chats[to]);
        st.chats[to].participants.items[at] = user;
    }
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Message Other Accounts").is_ok());
        let _ = sim.click("Signal").expect("the Signal chat");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::SelectChat(c)] if c == "!sg_lena:beeper.com"));
}

#[test]
fn chat_info_media_like_desktop() {
    let mut app = demo_app("chat-info-media");
    let _ = app.update(UiMessage::SelectChat("!sg_security:beeper.com".into()));
    assert!(tall(app.view_info()).find("Media").is_err());
    let _ = app.update(UiMessage::SelectChat("!im_jordan:beeper.com".into()));
    {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Media").is_ok() && sim.find("Show all").is_err() && sim.find("Images").is_err());
    }
    let _ = app.update(UiMessage::InfoMedia(Some(0)));
    assert!(tall(app.view_info()).find("Nothing here").is_err());
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        let _ = sim.click("Audio").expect("the Audio tab");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::InfoMedia(Some(2))]));
    let _ = app.update(UiMessage::InfoMedia(Some(2)));
    assert!(tall(app.view_info()).find("Nothing here").is_ok());
    let _ = app.update(UiMessage::SelectChat("!im_jordan:beeper.com".into()));
    assert!(app.info_media.is_none());
}

#[test]
fn chat_info_bookmarks_of_messages_that_are_not_loaded() {
    let mut app = demo_app("chat-info-bookmarks-missing");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    {
        let mut st = app.store.write();
        let mut old = st.messages[st.chat_msg_indices[&id][0]].clone();
        (old.id, old.text, old.html) = ("$old".into(), Some("Older bookmarked text".into()), None);
        let mark = |order| Bookmark { room_id: id.clone(), event_id: format!("$b{order}"), order };
        st.apply_bookmarks("!bookmarks", vec![("$old".into(), Some(mark(3))), ("$gone".into(), Some(mark(2)))]);
        st.apply_bookmarks("!bookmarks", vec![("$wait".into(), Some(mark(1)))]);
        st.bookmarked.extend([("$old".to_string(), Some(old)), ("$gone".to_string(), None)]);
    }
    {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Older bookmarked text").is_ok());
        assert!(sim.find("Message unavailable").is_ok() && sim.find("Loading…").is_ok());
        assert!(sim.find("It was deleted or is no longer available.").is_ok());
    }
    let menu = info_menu(&app.store.read(), &format!("bookmark:{id} $old"), |_| true);
    assert!(matches!(&menu[0], Some((_, None))), "Jump to Message needs the message in the timeline");
    let _ = app.update(UiMessage::RemoveBookmark("$gone".into()));
    app.store.write().apply_bookmarks("!bookmarks", vec![("$gone".into(), None)]);
    assert!(tall(app.view_info()).find("Message unavailable").is_err());
}

#[test]
fn chat_info_bookmarks_like_desktop() {
    let mut app = demo_app("chat-info-bookmarks");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Bookmarks").is_ok() && sim.find("No Bookmarks").is_ok());
        assert!(sim.find("Bookmarked messages will appear here.").is_ok());
    }
    let (mid, date) = {
        let st = app.store.read();
        let m = &st.messages[st.chat_msg_indices[&id][0]];
        (m.id.clone(), list_ts(&m.timestamp, st.settings.pref("timeFormat") == 1))
    };
    let _ = app.update(UiMessage::ToggleBookmark(mid.clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        assert!(sim.find("No Bookmarks").is_err());
        let _ = sim.click(date.as_str()).expect("the bookmark row");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::JumpTo(c, m)] if *c == id && *m == mid));
    let menu = info_menu(&app.store.read(), &format!("bookmark:{id} {mid}"), |_| true);
    let labels: Vec<_> = menu.iter().map(|i| i.as_ref().map(|i| i.0.as_str())).collect();
    assert_eq!(labels, [Some("Jump to Message"), None, Some("Remove Bookmark")]);
    let _ = app.update(UiMessage::OpenChatMenu(format!("bookmark:{id} {mid}")));
    let _ = app.update(menu[2].clone().and_then(|i| i.1).expect("Remove Bookmark"));
    assert!(app.chat_menu.is_none());
    assert!(tall(app.view_info()).find("No Bookmarks").is_ok());
}

#[test]
fn timer_labels_like_date_fns() {
    let labels: Vec<String> = DEFAULT_TIMERS.iter().map(|&ms| crate::timer_label(ms)).collect();
    assert_eq!(
        labels,
        [
            "10 seconds",
            "5 minutes",
            "30 minutes",
            "1 hour",
            "4 hours",
            "1 day",
            "2 days",
            "3 days",
            "7 days",
            "1 month",
            "1 year",
        ]
    );
    assert_eq!(crate::timer_label(7_776_000_000), "3 months");
    assert_eq!(crate::timer_label(0), "Off");
}

#[test]
fn mute_labels_like_desktop() {
    let mut app = demo_app("label-mute");
    let _ = app.update(UiMessage::OpenChatMenu("account:label:lbl_work".into()));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu("label:lbl_work"));
        let _ = sim.click("Mute Label").expect("a Mute Label item");
        sim.into_messages().collect()
    };
    assert!(matches!(&picks[..], [UiMessage::MuteLabel(id, true)] if id == "lbl_work"));
    let (only, both) = {
        let st = app.store.read();
        let work = &st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label").rooms;
        let other = st.labels.iter().find(|l| l.id != "lbl_work").expect("a label").id.clone();
        let only = st.chats.iter().find(|c| work.contains(&c.id) && c.is_muted != Some(true)).expect("a work chat");
        assert!(!st.notifications_muted(only));
        (only.id.clone(), other)
    };
    for m in picks {
        let _ = app.update(m);
    }
    simulator(app.view_account_menu("label:lbl_work")).find("Unmute Label").expect("an Unmute Label item");
    let muted = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.notifications_muted(&st.chats[st.chat_idx_by_id[&only]])
    };
    assert!(muted(&app), "a chat is muted when all its labels are muted");
    {
        let mut sim = simulator(app.view_chat_menu(&only, false, 900.0));
        sim.find("Muted by Label Work").expect("the mute reason");
        let _ = sim.click("Unmute").expect("an Unmute item");
        let toggles = sim.into_messages().filter(|m| matches!(m, UiMessage::ToggleFlag(..)));
        assert_eq!(toggles.count(), 0, "a label mute is not undone from the chat");
    }
    let i = app.store.read().chat_idx_by_id[&only];
    app.store.write().update_chat(i, |c| c.label_ids.push(both.clone()));
    assert!(!muted(&app), "an unmuted label keeps the chat unmuted");
    let _ = app.update(UiMessage::MuteLabel("lbl_work".into(), false));
    app.store.write().update_chat(i, |c| c.label_ids.retain(|l| l != &both));
    assert!(!muted(&app));
}

#[test]
fn toasts_time_out_like_desktop() {
    let mut app = demo_app("toast-timeout");
    app.focused = true;
    let _ = app.update(UiMessage::CopyLink("https://example.com".into()));
    let shown = app.status_banner.clone();
    assert!(!shown.is_empty() && app.toast_running());
    let _ = app.update(UiMessage::ToastHover(true));
    assert!(!app.toast_running(), "hover pauses the toast");
    let _ = app.update(UiMessage::ToastHover(false));
    let ticks = (TOAST_TIMEOUT.as_millis() / TOAST_TICK.as_millis()) as usize;
    for _ in 1..ticks {
        let _ = app.update(UiMessage::ToastTick);
    }
    assert_eq!(app.status_banner, shown);
    let _ = app.update(UiMessage::ToastTick);
    assert!(app.status_banner.is_empty() && !app.toast_running());
    app.status_banner = "a".into();
    app.sticky_banner = Some("a".into());
    assert!(!app.toast_running(), "a sticky toast stays");
}

#[test]
fn new_chats_start_like_desktop() {
    let mut app = demo_app("start-chat");
    let (dm, account) = {
        let mut st = app.store.write();
        let i = st.chats.iter().position(|c| c.chat_type == "single" && c.participants.items.len() == 2).expect("a DM");
        let other =
            st.chats[i].participants.items.iter().position(|u| u.is_self != Some(true)).expect("the other person");
        st.update_chat(i, |c| c.participants.items[other].phone_number = Some("+1 (555) 010-0".into()));
        (st.chats[i].id.clone(), st.chats[i].account_id.clone())
    };
    let contact = |id: &str, name: &str, phone: &str| -> User {
        serde_json::from_value(serde_json::json!({ "id": id, "fullName": name, "phoneNumber": phone })).expect("a user")
    };
    let sam = contact("contact:a:tel:+15550100", "Sam Example", "+15550100");
    let bo = contact("contact:b:tel:+15550199", "Bo Example", "+15550199");
    app.store.write().set_contacts(vec![sam.clone(), bo.clone()]);
    let label = {
        let st = app.store.read();
        account_display_name(&st, &account, &rail_accounts(&st))
    };
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat);
    let n = app.creating_chat.expect("a draft");
    assert!(found(&app, "Sam Example") && found(&app, "Bo Example") && !found(&app, &label));
    let _ = app.update(UiMessage::StartChat);
    assert_eq!(app.creating_chat, Some(n), "Start chat needs a person");
    let _ = app.update(UiMessage::NewChatQueryChanged("sam".into()));
    let _ = app.update(UiMessage::ToggleContact(sam.id.clone()));
    assert!(found(&app, &label), "an account with any_phone reaches the phone number");
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let user = nets.iter().find(|(a, _)| *a == account).and_then(|(_, u)| u.clone().flatten()).expect("a user");
    assert_eq!((user.id.as_str(), user.phone_number.as_deref()), ("+15550100", Some("+15550100")));
    let _ = app.update(UiMessage::PickUser(account.clone(), Box::new(user.clone())));
    assert!(app.new_chat_query.is_empty() && app.new_chats[0].users.len() == 1);
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Backspace), Modifiers::empty()));
    assert!(app.new_chats[0].users.is_empty(), "Backspace removes the pill");
    let _ = app.update(UiMessage::PickUser(account.clone(), Box::new(user)));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.new_chats.is_empty() && app.creating_chat.is_none(), "the draft goes");
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(dm.as_str()), "the DM with the same phone opens");

    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "lookup_phone": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::NewChat);
    let n = app.creating_chat.expect("a draft");
    let _ = app.update(UiMessage::ToggleContact(bo.id.clone()));
    assert!(!found(&app, &label) && !found(&app, "No networks found"), "the lookup loads");
    let slot = |app: &BeeperGuiApp| {
        app.contact_networks.as_ref().expect("the networks").1.iter().position(|(a, _)| *a == account).expect("a slot")
    };
    let _ = app.update(UiMessage::ContactNetwork(bo.id.clone(), slot(&app), Ok(None)));
    assert!(!found(&app, &label) && !found(&app, "No networks found"), "the other accounts reach the phone number");
    let _ = app.update(UiMessage::NewChatAccount(Some(account.clone())));
    assert!(found(&app, "No networks found"), "the other accounts are not picked");
    let _ = app.update(UiMessage::NewChatAccount(None));
    let _ = app.update(UiMessage::ToggleContact(bo.id.clone()));
    assert!(app.contact_networks.is_none() && !found(&app, "No networks found"), "a second click collapses");
    let _ = app.update(UiMessage::ToggleContact(bo.id.clone()));
    let ghost = serde_json::json!({ "id": "199", "mxid": "@bridge_199:beeper.local", "name": "Bo Example" });
    let _ = app.update(UiMessage::ContactNetwork(bo.id.clone(), slot(&app), Ok(Some(ghost))));
    assert!(found(&app, &label));
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let user = nets.iter().find(|(a, _)| *a == account).and_then(|(_, u)| u.clone().flatten()).expect("a user");
    assert_eq!(user.id, "@bridge_199:beeper.local", "a resolved user has its Matrix ID");
    let _ = app.update(UiMessage::PickUser(account, Box::new(user)));
    assert!(found(&app, "Bo Example"), "the draft is titled with the person");
    let _ = app.update(UiMessage::StartChat);
    assert_eq!(app.status_banner, "Composing chat...");
    assert!(!app.toast_running(), "the composing toast stays");
    let _ = app.update(UiMessage::DmCreated(Some(n), String::new(), None, Err("timeout".into())));
    assert!(
        app.status_banner.starts_with("Something went wrong creating thread (")
            && app.status_banner.ends_with(")\ntimeout")
    );
    assert!(
        matches!(&app.shown_toast().expect("a toast").buttons[..], [("Copy Error", UiMessage::CopyText(e))] if e == "timeout")
    );
    let _ = app.update(UiMessage::StartChat);
    let _ = app.update(UiMessage::DmCreated(Some(n), String::new(), None, Ok("!new:beeper.local".into())));
    assert!(app.new_chats.is_empty() && app.creating_chat.is_none() && app.status_banner.is_empty());
    assert_eq!(app.pending_dm, Some(("!new:beeper.local".into(), None)), "the room opens when it syncs");
}

#[test]
fn new_chat_accounts_filter_like_desktop() {
    let mut app = demo_app("new-chat-accounts");
    let account = rail_accounts(&app.store.read()).into_iter().find(|a| a.contains("telegram")).expect("telegram");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat);
    let _ = app.update(UiMessage::ToggleNewChatAccounts);
    assert!(found(&app, "Telegram") && found(&app, "WhatsApp"));
    let _ = app.update(UiMessage::NewChatAccount(Some(account.clone())));
    assert!(!app.new_chat_accounts_open && app.new_chat_account.as_deref() == Some(account.as_str()));
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(serde_json::json!({ "resolve_identifier": {} }))));
    let _ = app.update(UiMessage::NewChat);
    assert!(app.new_chat_account.is_none(), "a new draft shows all accounts");
    let _ = app.update(UiMessage::ToggleNewChatAccounts);
    assert!(!found(&app, "Telegram") && found(&app, "WhatsApp"), "a bridge without create_dm is not listed");
}

#[test]
fn email_links_and_media_items_match_desktop() {
    assert_eq!(email("a.b+c@mail.example.com)."), Some("a.b+c@mail.example.com"));
    assert!([email("@example.com"), email("a@b"), email("a@b.c"), email("a@b.c1"), email("ab")]
        .iter()
        .all(Option::is_none));
    let mut app = demo_app("email-menu");
    let mid = {
        let mut st = app.store.write();
        let chat = app.pane.selected_chat_id.clone().expect("a chat");
        let i = *st.chat_msg_indices[&chat].last().expect("a message");
        let m = &mut st.messages[i];
        (m.text, m.html, m.links, m.unresolved_media) =
            (Some("Mail a.b@example.com.".into()), None, Vec::new(), Some("{}".into()));
        m.id.clone()
    };
    let click = |app: &mut BeeperGuiApp, label: &str| {
        let _ = app.update(UiMessage::OpenMessageMenu(mid.clone()));
        let mut sim = simulator(app.view_main());
        let _ = sim.click(label).unwrap_or_else(|_| panic!("{label}"));
        sim.into_messages().collect::<Vec<_>>()
    };
    let copy = click(&mut app, "Copy a.b@example.com");
    assert!(
        matches!(&copy[..], [UiMessage::MessageMenu(m)] if matches!(&**m, UiMessage::CopyText(e) if e == "a.b@example.com"))
    );
    let mail = click(&mut app, "Email a.b@example.com…");
    assert!(
        matches!(&mail[..], [UiMessage::MessageMenu(m)] if matches!(&**m, UiMessage::OpenUrl(u) if u == "mailto:a.b@example.com"))
    );
    assert!(
        matches!(&click(&mut app, "Load Media")[..], [UiMessage::MessageMenu(m)] if matches!(**m, UiMessage::ResolveMedia(..)))
    );
    app.resolving_media.insert(mid.clone(), true);
    let off = click(&mut app, "Load Media");
    assert!(!off.iter().any(|m| matches!(m, UiMessage::MessageMenu(_))), "a resolving media item is disabled");
    app.resolving_media.insert(mid.clone(), false);
    assert_eq!(click(&mut app, "Try Loading Media Again").len(), 1);
}

#[test]
fn errors_toast_like_desktop() {
    let mut app = demo_app("error-toast");
    let failed = UiMessage::MediaResolved("acc".into(), "$ev".into(), "{}".into(), Err("timeout".into()));
    let _ = app.update(failed);
    let t = app.shown_toast().expect("an error toast");
    assert!(t.error && app.status_banner == "Could not load this media.");
    let [(label, again)] = &t.buttons[..] else { panic!("one button") };
    assert!(*label == "Try again" && matches!(again, UiMessage::ResolveMedia(a, e, _) if a == "acc" && e == "$ev"));
    let _ = app.update(again.clone());
    assert_eq!(app.resolving_media.get("$ev"), Some(&true), "Try again resolves the media again");
    let _ = app.update(UiMessage::CopyLink("https://example.com".into()));
    assert!(app.shown_toast().is_none_or(|t| !t.error), "a later toast is not an error");
}

#[test]
fn double_clicks_like_desktop() {
    let mut app = demo_app("sidebar-reset");
    let click = [mouse::Event::ButtonPressed(Button::Left), mouse::Event::ButtonReleased(Button::Left)];
    let twice: Vec<iced::Event> = click.into_iter().chain(click).map(iced::Event::Mouse).collect();
    let main = app.main_window.expect("a main window");
    assert!(app.pane.selected_chat_id.is_some());
    let header = interact(&app, main, Size::new(1440.0, 900.0), Point::new(500.0, 20.0), twice.clone());
    assert!(matches!(&header[..], [UiMessage::ToggleMaximize(None)]), "the chat header maximizes: {header:?}");
    let edge = interact(&app, main, Size::new(1440.0, 900.0), Point::new(68.0 + SIDEBAR_WIDTH + 1.0, 400.0), twice);
    assert!(edge.iter().any(|m| matches!(m, UiMessage::ResetSidebarWidth)), "the sidebar edge resets: {edge:?}");
    let _ = app.update(UiMessage::DragStart);
    let _ = app.update(UiMessage::DragMoved(Point::new(500.0, 10.0)));
    let _ = app.update(UiMessage::DragEnd);
    assert!(app.sidebar_width > SIDEBAR_WIDTH);
    let _ = app.update(UiMessage::DragStart);
    let _ = app.update(UiMessage::ResetSidebarWidth);
    let _ = app.update(UiMessage::DragEnd);
    assert_eq!(app.sidebar_width, SIDEBAR_WIDTH);
    let saved = std::fs::read_to_string(app.store.read().data_dir.join("sidebar-width")).expect("the saved width");
    assert_eq!(saved.parse::<f32>().expect("a width"), SIDEBAR_WIDTH);
}

#[test]
fn mod_z_undoes_actions_like_desktop() {
    let mut app = demo_app("undo-stack");
    let z = |app: &mut BeeperGuiApp| {
        let _ = app.update(UiMessage::KeyPressed(Key::Character("z".into()), Modifiers::COMMAND));
    };
    let order = app.store.read().settings.spaces_order.clone();
    let _ = app.update(UiMessage::OpenEditSpaces);
    let id = |app: &BeeperGuiApp, i: usize| app.spaces_window.as_ref().expect("Edit Spaces").order[i].clone();
    let (first, second) = (id(&app, 0), id(&app, 1));
    let _ = app.update(UiMessage::Spaces(SpacesEdit::Drag(second.clone())));
    let _ = app.update(UiMessage::Spaces(SpacesEdit::Over(first.clone())));
    let _ = app.update(UiMessage::Spaces(SpacesEdit::Drop));
    assert_ne!(app.store.read().settings.spaces_order, order);
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::ToggleFlag(chat.into(), ChatFlag::Muted));
    let _ = app.update(UiMessage::SetLabelInbox("lbl_work".into(), Some("none")));
    assert!(app.status_banner.is_empty(), "these actions have no toast");
    let muted = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.resolve_chat_idx(chat).expect("the chat")].is_muted == Some(true)
    };
    assert!(muted(&app));
    z(&mut app);
    assert_eq!(app.status_banner, "Undo: Set \"Work\" inbox visibility to None");
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.inbox.is_none()));
    z(&mut app);
    assert!(!muted(&app) && app.status_banner.starts_with("Undo: Mute "));
    z(&mut app);
    assert_eq!(app.status_banner, "Undo: Reorder sidebar");
    assert_eq!(app.store.read().settings.spaces_order, order);
    assert_eq!((id(&app, 0), id(&app, 1)), (first, second), "Edit Spaces shows the old order");
    assert!(app.undo_stack.is_empty());
}

#[test]
fn new_chat_lists_chat_people_like_desktop() {
    let mut app = demo_app("new-chat-people");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat);
    assert!(!found(&app, "Chats"), "Desktop lists chat people only for a query");
    let _ = app.update(UiMessage::NewChatQueryChanged("ERIC".into()));
    assert!(found(&app, "Chats"));
    let people = {
        let st = app.store.read();
        let listed: Vec<String> = rail_accounts(&st).into_iter().filter(|a| app.can(a, "create_dm")).collect();
        chat_people(&st, &listed, None, "eric")
    };
    let mut keys: Vec<(&str, &str)> = people.iter().map(|(a, u)| (a.as_str(), u.id.as_str())).collect();
    let n = keys.len();
    keys.dedup();
    assert!(n >= 2 && keys.len() == n, "one row per account and person");
    let (account, eric) = people.iter().find(|(a, _)| a.contains("whatsapp")).cloned().expect("Eric on WhatsApp");
    let dm = {
        let st = app.store.read();
        let dms = st.chats.iter().filter(|c| c.account_id == account && c.chat_type == "single");
        dms.into_iter().find(|c| c.participants.items.iter().any(|u| u.id == eric.id)).expect("the DM").id.clone()
    };
    let _ = app.update(UiMessage::PickUser(account, Box::new(eric)));
    let _ = app.update(UiMessage::StartChat);
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(dm.as_str()), "the DM with the Matrix ID opens");
    let _ = app.update(UiMessage::NewChat);
    let _ = app.update(UiMessage::NewChatQueryChanged("zzzz".into()));
    assert!(found(&app, "No results found") && !found(&app, "Chats"));
}

#[test]
fn new_chat_looks_up_identifiers_like_desktop() {
    let mut app = demo_app("new-chat-lookup");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    assert!(typed_phone("+1 (415) 555-0177") && !typed_phone("555") && !typed_phone("call 4155550177"));
    let _ = app.update(UiMessage::NewChat);
    let _ = app.update(UiMessage::NewChatQueryChanged("+1 415 555 0177".into()));
    let _ = app.update(UiMessage::LookupIdentifier("+1 415".into()));
    assert!(app.new_chat_lookup.is_none(), "a lookup of an old query does nothing");
    let _ = app.update(UiMessage::LookupIdentifier("+1 415 555 0177".into()));
    let nets = &app.new_chat_lookup.as_ref().expect("a lookup").1;
    assert!(nets.len() > 1 && nets.iter().all(|(_, n)| n.as_ref().is_some_and(Option::is_some)), "any_phone");
    assert!(found(&app, &fmt_phone("+14155550177")) && !found(&app, "No results found"));
    let account = rail_accounts(&app.store.read()).into_iter().next().expect("an account");
    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "lookup_username": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::NewChatQueryChanged("@samx".into()));
    let _ = app.update(UiMessage::LookupIdentifier("@samx".into()));
    let nets = &app.new_chat_lookup.as_ref().expect("a lookup").1;
    assert!(matches!(&nets[..], [(a, None)] if *a == account), "only the account with lookup_username asks");
    assert!(found(&app, "No results found"), "rows show when all lookups end");
    let user = serde_json::json!({ "id": "x", "mxid": "@x_samx:beeper.local", "name": "Sam X", "identifiers": ["mailto:sam@example.com"] });
    let _ = app.update(UiMessage::ContactNetwork("lookup:@samx".into(), 0, Ok(Some(user))));
    assert!(found(&app, "Sam X") && found(&app, "sam@example.com"));
    let _ = app.update(UiMessage::NewChatQueryChanged("@sam".into()));
    assert!(app.new_chat_lookup.is_none() && !found(&app, "Sam X"), "typing drops the lookup");
}

#[test]
fn new_chat_searches_connections_like_desktop() {
    let mut app = demo_app("new-chat-connections");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let account = rail_accounts(&app.store.read()).into_iter().next().expect("an account");
    let _ = app.update(UiMessage::NewChat);
    let _ = app.update(UiMessage::NewChatQueryChanged("dana".into()));
    let _ = app.update(UiMessage::LookupIdentifier("dana".into()));
    assert!(app.new_chat_search.is_none(), "no bridge can search");
    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "search": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::LookupIdentifier("dana".into()));
    assert!(found(&app, "Connections") && found(&app, "Searching..."));
    let user = |name: &str, phone: &str| serde_json::json!({ "id": name, "mxid": format!("@x_{name}:beeper.local"), "name": name, "identifiers": [format!("tel:{phone}")] });
    let results = vec![user("Dana One", "+15550111"), user("Dana Two", "+15550111"), user("Ann Dana", "+15550112")];
    let _ = app.update(UiMessage::SearchedUsers("dan".into(), account.clone(), Ok(results.clone())));
    assert!(found(&app, "Searching..."), "results of an old query do nothing");
    let _ = app.update(UiMessage::SearchedUsers("dana".into(), account, Ok(results)));
    assert!(!found(&app, "Searching...") && found(&app, "Dana One") && found(&app, "Ann Dana"));
    assert!(!found(&app, "Dana Two"), "one row per bridge and phone number");
    let _ = app.update(UiMessage::NewChatQueryChanged("danae".into()));
    assert!(app.new_chat_search.is_none() && !found(&app, "Connections"));
}

#[test]
fn new_chat_groups_follow_bridge_capabilities_like_desktop() {
    let mut app = demo_app("new-chat-groups");
    let accounts: Vec<String> = rail_accounts(&app.store.read()).into_iter().filter(|a| app.can_group(a)).collect();
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat);
    assert!(accounts.len() > 1 && found(&app, "New Group"));
    let user = |id: &str| Box::new(User { id: id.into(), full_name: Some(id.into()), ..Default::default() });
    let (a, b) = (accounts[0].clone(), accounts[1].clone());
    let _ = app.update(UiMessage::PickUser(a.clone(), user("@one:x")));
    let _ = app.update(UiMessage::PickUser(a.clone(), user("@two:x")));
    let _ = app.update(UiMessage::PickUser(a.clone(), user("@two:x")));
    let ids = |app: &BeeperGuiApp| app.new_chats[0].users.iter().map(|(_, u)| u.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&app), ["@one:x", "@two:x"], "a group bridge adds people once");
    let single = serde_json::json!({ "resolve_identifier": { "create_dm": true } });
    for acc in &accounts {
        let _ = app.update(UiMessage::Capabilities(acc.clone(), Ok(single.clone())));
    }
    let _ = app.update(UiMessage::PickUser(b, user("@three:x")));
    assert_eq!(ids(&app), ["@three:x"], "a bridge without groups replaces the person");
    let _ = app.update(UiMessage::NewChat);
    assert!(!found(&app, "New Group"), "no bridge can create groups");
}

#[test]
fn reply_privately_like_desktop() {
    let mut app = demo_app("reply-privately");
    let (theirs, mine, account) = {
        let st = app.store.read();
        let c = st.chats.iter().find(|c| c.id == "!tg_rust:beeper.com").expect("the group");
        let msgs = st.chat_msg_indices[&c.id].iter().map(|&i| &st.messages[i]);
        let mut msgs = msgs.filter(|m| m.text.is_some() && m.is_deleted != Some(true));
        let theirs = msgs
            .clone()
            .find(|m| m.is_sender != Some(true) && c.participants.items.iter().any(|u| u.id == m.sender_id));
        let mine = msgs.find(|m| m.is_sender == Some(true)).map(|m| m.id.clone());
        (theirs.expect("a message from someone").clone(), mine, c.account_id.clone())
    };
    let has = |app: &BeeperGuiApp, mid: &str| simulator(app.view_message_menu(mid).0).find("Reply Privately…").is_ok();
    assert!(has(&app, &theirs.id));
    assert!(mine.is_none_or(|m| !has(&app, &m)), "not for your own messages");
    let _ = app.update(UiMessage::ReplyPrivately(theirs.id.clone()));
    assert_eq!(app.status_banner, "Composing chat...");
    let quote = format!("> {}\n", theirs.text.clone().expect("text").lines().next().expect("a line"));
    let room = "!sg_security:beeper.com".to_string();
    let _ = app.update(UiMessage::DmCreated(None, account, Some(quote.clone()), Ok(room.clone())));
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(room.as_str()));
    assert!(app.pane.composer.text().starts_with(&quote), "the composer quotes the message");
}

#[test]
fn message_actions_follow_room_features_like_desktop() {
    let mut app = demo_app("message-rules");
    let (id, mine) = {
        let mut st = app.store.write();
        let i = (0..st.messages.len()).rfind(|&i| {
            let m = &st.messages[i];
            let plain = m.text.is_some() && m.poll.is_none() && m.attachments.is_empty();
            plain && m.is_sender == Some(true)
        });
        let i = i.expect("a message of yours");
        st.update_message(i, |m| m.timestamp = "2020-01-01T00:00:00.000Z".into());
        (st.messages[i].chat_id.clone(), st.messages[i].id.clone())
    };
    let network = {
        let st = app.store.read();
        network_name(&st.chats[st.chat_idx_by_id[&id]].account_id)
    };
    let set = |app: &mut BeeperGuiApp, rules: MessageRules| {
        let mut st = app.store.write();
        let i = st.chat_idx_by_id[&id];
        st.update_chat(i, |c| c.messages = rules);
    };
    let old = MessageRules { edit_max_age: Some(60), react: false, reply: false, ..BEEPER_MESSAGES };
    set(&mut app, old);
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let clicked = |app: &BeeperGuiApp, label: &str| -> Vec<UiMessage> {
        let mut sim = simulator(app.view_message_menu(&mine).0);
        let _ = sim.click(label).expect(label);
        sim.into_messages().collect()
    };
    assert!(simulator(app.view_message_menu(&mine).0).find("Reply…").is_err(), "the network has no replies");
    assert!(clicked(&app, "Edit…").is_empty() && clicked(&app, "React…").is_empty(), "disabled like Desktop");
    let ask = clicked(&app, "Delete for Everyone…");
    assert!(
        matches!(&ask[..], [UiMessage::MessageMenu(m)] if matches!(&**m, UiMessage::AskDeleteMessage(v) if v == std::slice::from_ref(&mine)))
    );
    assert!(app.last_own_message().is_none_or(|m| m.id != mine), "Up Arrow skips a message past edit_max_age");
    app.pane.selected_msgs = vec![mine.clone()];
    let titles = |app: &BeeperGuiApp| -> Vec<String> {
        app.commands(&app.store.read(), "").into_iter().filter(|c| c.category == "Message").map(|c| c.title).collect()
    };
    let t = titles(&app);
    assert!(t.contains(&"Delete for everyone".into()) && t.contains(&"Edit".into()) && !t.contains(&"Reply".into()));
    set(&mut app, MessageRules { delete: false, ..old });
    assert!(simulator(app.view_message_menu(&mine).0).find("Delete for Everyone…").is_err());
    assert!(titles(&app).contains(&format!("Delete message is not supported for {network} yet")));
    let _ = app.update(UiMessage::AskDeleteMessage(vec![mine.clone(), mine.clone()]));
    assert!(simulator(app.view_main()).find("Are you sure you want to delete 2 messages?").is_ok());
}

#[test]
fn chat_info_header_like_desktop() {
    let mut app = demo_app("chat-info");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let set_edits = |app: &mut BeeperGuiApp, edits: u8| {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| c.edits = edits);
    };
    let click = |app: &BeeperGuiApp, label: &str| -> Vec<UiMessage> {
        let mut sim = simulator(app.view_info());
        let _ = sim.click(label).expect(label);
        sim.into_messages().collect()
    };
    set_edits(&mut app, 0);
    assert!(matches!(&click(&app, "Edit")[..], [UiMessage::OpenChatTheme(c)] if *c == id), "one action runs at once");
    set_edits(&mut app, ALL_EDITS);
    let menu = format!("info:{id}");
    assert!(matches!(&click(&app, "Edit")[..], [UiMessage::OpenChatMenu(k)] if *k == menu));
    let _ = app.update(UiMessage::OpenChatMenu(menu));
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Edit Group").is_ok() && sim.find("Chat Theme").is_ok());
        assert!(sim.find("Copy Beeper Rust Core Team").is_err(), "the Edit menu has no copy item");
    }
    let copy = format!("copy:{id}");
    assert!(matches!(&click(&app, "Beeper Rust Core Team")[..], [UiMessage::OpenChatMenu(k)] if *k == copy));
    let _ = app.update(UiMessage::OpenChatMenu(copy));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("Copy Beeper Rust Core Team").expect("the title menu copies the title");
    assert!(
        matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::CopyText(t)] if t == "Beeper Rust Core Team")
    );
}

#[test]
fn chat_info_description_like_desktop() {
    let mut app = demo_app("chat-info-desc");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let set = |app: &mut BeeperGuiApp, edits: u8, desc: Option<&str>| {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| (c.edits, c.description) = (edits, desc.map(Into::into)));
    };
    let has = |app: &BeeperGuiApp, label: &str| simulator(app.view_info()).find(label).is_ok();
    set(&mut app, 0, None);
    assert!(!has(&app, "Add a description") && !has(&app, "Group Description"));
    set(&mut app, 1 << 2, None);
    let _ = simulator(app.view_info()).click("Add a description").expect("an editor can add a description");
    let _ = app.update(UiMessage::InfoDesc(None));
    let _ = app.update(UiMessage::InfoDesc(Some(text_editor::Action::Edit(text_editor::Edit::Paste(
        std::sync::Arc::new("We build https://example.com".into()),
    )))));
    let _ = simulator(app.view_info()).click("Done").expect("Done saves");
    let _ = app.update(UiMessage::SaveInfoDesc);
    assert!(app.info_desc.is_none());
    {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(&id).expect("the group")];
        assert_eq!(c.description.as_deref(), Some("We build https://example.com"));
    }
    assert!(has(&app, "Group Description") && has(&app, "Edit") && !has(&app, "more"), "a short description");
    let long = ["word"; 120].join(" ");
    set(&mut app, 0, Some(&long));
    assert!(has(&app, "more"), "a description of more than three lines can expand");
    let _ = app.update(UiMessage::InfoMore);
    assert!(has(&app, "more"), "the toggle also collapses the description");
}

#[test]
fn chat_info_members_like_desktop() {
    let mut app = demo_app("chat-info-members");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let has = |app: &BeeperGuiApp, label: &str| simulator(app.view_info()).find(label).is_ok();
    assert!(has(&app, "Members") && !has(&app, "Show all"), "three members fit");
    {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| {
            let base = c.participants.items[0].clone();
            let extra = (0..5).map(|n| User {
                id: format!("@m{n}:beeper.local"),
                full_name: Some(format!("Member {n}")),
                phone_number: (n == 0).then(|| "+15550100".into()),
                email: None,
                is_self: Some(false),
                ..base.clone()
            });
            c.participants.items.extend(extra);
        });
    }
    assert!(has(&app, "Search 8 group members") && has(&app, "Show all"));
    assert!(!has(&app, "Member 4"), "the section shows six members");
    let _ = app.update(UiMessage::InfoAll);
    assert!(has(&app, "Member 4") && has(&app, "Show less"));
    let _ = app.update(UiMessage::InfoAll);
    let _ = app.update(UiMessage::InfoQuery("member 4".into()));
    assert!(has(&app, "Member 4") && !has(&app, "Member 3"), "the search filters the members");
    let _ = app.update(UiMessage::OpenChatMenu(format!("member:{id} @m0:beeper.local")));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Call").is_ok() && sim.find("Copy Phone Number").is_ok() && sim.find("Copy Email").is_err());
    let _ = sim.click("Copy Name").expect("the menu copies the name");
    assert!(matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::CopyText(t)] if t == "Member 0"));
}

#[test]
fn chat_info_notes_are_local_per_chat() {
    let mut app = demo_app("chat-info-notes");
    let (a, b) = ("!tg_rust:beeper.com".to_string(), "!note_self:beeper.com".to_string());
    let _ = app.update(UiMessage::SelectChat(a.clone()));
    let paste = |s: &str| text_editor::Action::Edit(text_editor::Edit::Paste(std::sync::Arc::new(s.into())));
    let _ = app.update(UiMessage::InfoNote(paste("Ask about the release")));
    let note = |app: &BeeperGuiApp, id: &str| {
        let st = app.store.read();
        st.chats[st.resolve_chat_idx(id).expect("chat")].note.clone()
    };
    assert_eq!(note(&app, &a).as_deref(), Some("Ask about the release"));
    let _ = app.update(UiMessage::SelectChat(b.clone()));
    assert_eq!((app.info_note.text(), note(&app, &b)), (String::new(), None), "each chat has its own note");
    let _ = app.update(UiMessage::SelectChat(a));
    assert_eq!(app.info_note.text(), "Ask about the release");
}

#[test]
fn chat_info_leave_chat_like_desktop() {
    let mut app = demo_app("chat-info-leave");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let set = |app: &mut BeeperGuiApp, actions: u8| {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| c.member_actions = actions);
    };
    set(&mut app, ALL_MEMBER_ACTIONS & !MEMBER_LEAVE);
    assert!(tall(app.view_info()).find("Leave Chat").is_err(), "the room does not support leaving");
    set(&mut app, ALL_MEMBER_ACTIONS);
    let mut sim = tall(app.view_info());
    let _ = sim.click("Leave Chat").expect("a Leave Chat link");
    assert!(matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::AskLeaveChat(c)] if *c == id));
    let _ = app.update(UiMessage::AskLeaveChat(id.clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_confirm(ASK_LEAVE_CHAT));
        assert!(sim.find("Are you sure you want to leave Beeper Rust Core Team?").is_ok());
        let _ = sim.click("Leave").expect("a Leave button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.confirm.is_none() && app.store.read().resolve_chat_idx(&id).is_none(), "the chat is gone");
}

#[test]
fn edit_group_like_desktop() {
    let mut app = demo_app("edit-group");
    let id = "!tg_rust:beeper.com".to_string();
    let set_edits = |app: &mut BeeperGuiApp, edits: u8| {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| c.edits = edits);
    };
    let in_menu = |app: &mut BeeperGuiApp| {
        let _ = app.update(UiMessage::OpenChatMenu(id.clone()));
        simulator(app.view_main()).find("Edit Group...").is_ok()
    };
    set_edits(&mut app, 0);
    assert!(!in_menu(&mut app), "a group without edits has no item");
    set_edits(&mut app, ALL_EDITS);
    assert!(in_menu(&mut app));
    let _ = app.update(UiMessage::OpenEditGroup(id.clone()));
    let d = app.group_window.as_ref().expect("the window opens");
    assert!(app.chat_menu.is_none() && d.title == "Beeper Rust Core Team");
    assert_eq!(d.changes(&app.store.read()), (None, None, false), "nothing changed yet");
    fn view(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
        simulator(app.view_edit_group(app.group_window.as_ref().expect("open")))
    }
    assert!(view(&app).find("Remove Photo").is_err(), "no photo to remove");
    let _ = app.update(UiMessage::Group(GroupEdit::Title("Core".into())));
    app.group_window.as_mut().expect("open").topic = text_editor::Content::with_text("Our team");
    let _ = app.update(UiMessage::Group(GroupEdit::Save));
    {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(&id).expect("the group")];
        assert_eq!((c.title.as_str(), c.description.as_deref()), ("Core", Some("Our team")));
    }
    let _ = app.update(UiMessage::WindowClosed(app.group_window.as_ref().expect("open").id));
    assert!(app.group_window.is_none());
    {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the group");
        st.update_chat(i, |c| c.img_url = Some("mxc://x/y".into()));
    }
    set_edits(&mut app, 1 << 1);
    let _ = app.update(UiMessage::OpenEditGroup(id.clone()));
    let _ = view(&app).click("Remove Photo").expect("an avatar editor can remove the photo");
    let _ = app.update(UiMessage::Group(GroupEdit::Remove));
    assert_eq!(app.group_window.as_ref().expect("open").changes(&app.store.read()), (None, None, true));
    let _ = app.update(UiMessage::Group(GroupEdit::Save));
    let st = app.store.read();
    assert_eq!(st.chats[st.resolve_chat_idx(&id).expect("the group")].img_url, None, "Remove Photo clears the avatar");
}
