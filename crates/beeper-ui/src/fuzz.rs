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
        .filter(|(_, p)| !matches!(p.control, prefs::Control::Microphone) && p.key != "appLanguage")
        .map(move |(page, p)| (tab(page), p))
}

pub(crate) fn demo_app(name: &str) -> BeeperGuiApp {
    let dir = std::env::temp_dir().join(format!("beeper-rs-fuzz-{name}-{}", std::process::id()));
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
    let branch = rng.below(63);
    let what =
        format!("message #{branch} chat {chat} message {msg} text {text:?} pref {} key {key:?} {mods:?}", pref.key);
    let needs_msg = (7..=14).contains(&branch) || branch == 53;
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
        53 => UiMessage::Forward(msg),
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
