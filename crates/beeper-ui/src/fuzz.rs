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
    let hits = search_settings(&text);
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
        12 => UiMessage::AskDeleteMessage(msg),
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
    assert!(!on(&app, "incognito") && on(&app, "typingIndicators"));
    let titles = |app: &BeeperGuiApp| app.commands(&app.store.read(), "stealth").into_iter().map(|c| c.title);
    assert_eq!(titles(&app).next().as_deref(), Some("Turn on incognito mode"));
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert!(on(&app, "incognito"));
    assert_eq!(app.status_banner, "Incognito mode on. Also hide when you're typing from recipients?");
    assert_eq!(titles(&app).next().as_deref(), Some("Turn off incognito mode"));
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
    let _ = app.update(UiMessage::SetPref("incognito", 1));
    assert_eq!(app.status_banner, "Incognito mode on.");
    let _ = app.update(UiMessage::SetPref("typingIndicators", 1));
    let _ = app.update(UiMessage::SetPref("incognito", 0));
    assert_eq!(app.status_banner, "Incognito mode off.");
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
    assert_eq!(titles, ["Edit", "Forward", "Copy", "Copy with timestamps", "Open link", "Unselect", "Reply"]);
    assert!(matches!(&cmds[4].msg, Some(UiMessage::OpenUrl(u)) if u.starts_with("https://")));
    let _ = app.update(cmds[5].msg.clone().unwrap());
    assert!(app.pane.selected_msgs.is_empty(), "Unselect clears the selection");
    let _ = app.update(UiMessage::SelectMessages(linked[..2].to_vec()));
    let cmds = app.commands(&app.store.read(), "");
    let titles: Vec<&str> = cmds.iter().take_while(|c| c.category == "Messages").map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Forward", "Copy", "Copy with timestamps", "Open link", "Unselect"]);
    let _ = app.update(UiMessage::Commands(vec![UiMessage::CloseModals, cmds[3].msg.clone().unwrap()]));
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
