//! Random clicks, keys and messages on every window of a demo account, with every value of every preference.
//! `FUZZ_SEED` is the first of the 16 seeds, and `FUZZ_STEPS` is the number of steps of each seed.

use super::*;
use crate::composer::{composer_action, hidden_actions, mention_label};
use crate::forward::forward_chats;
use crate::info::info_menu;
use crate::new_chat::{account_display_name, chat_people, typed_phone};
use crate::rail::{all_spaces, LABEL_NUDGE};
use crate::rail::{draft_rail_spaces, rail_accounts, rail_owner, rail_spaces, RAIL_MORE, RAIL_VIEWS};
use crate::rail::{SpacesEdit, SPACES_DIVIDER, SPACES_SIZE};
use crate::reminders::reminder_day;
use crate::scheduled::send_day;
use crate::time::{clock, format_short_ts, list_ts};
use crate::toasts::{TOAST_TIMEOUT, UNDO_NOTE_TIMEOUT};
use crate::units::{compact, grouped, human_size};
use crate::windows::write_window_state;
use beeper_core::config::CentralConfig;
use beeper_core::matrix::ROOM_MENTION;
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
        .filter(|(_, p)| {
            !matches!(
                p.control,
                prefs::Control::Microphone
                    | prefs::Control::Multi(_)
                    | prefs::Control::Analytics
                    | prefs::Control::Downloads
                    | prefs::Control::BlockedUsers
                    | prefs::Control::ReactionEmoji { .. }
                    | prefs::Control::QuickReactions(_)
            )
        })
        .filter(|(_, p)| p.key != "appLanguage")
        .map(move |(page, p)| (tab(page), p))
}

pub(crate) fn demo_app(name: &str) -> BeeperGuiApp {
    BeeperGuiApp::boot(beeper_core::testing::demo_store(name)).0
}

/// The messages that a task outputs. The window and clipboard actions of the task do nothing here: each is dropped
/// as it comes, so that a task that waits for its answer goes on.
pub(crate) async fn task_messages(task: Task<UiMessage>) -> Vec<UiMessage> {
    use iced::futures::StreamExt;
    let Some(stream) = iced_runtime::task::into_stream(task) else { return Vec::new() };
    let out = stream.filter_map(|a| async move {
        match a {
            iced_runtime::Action::Output(m) => Some(m),
            _ => None,
        }
    });
    out.collect().await
}

/// The actions of a task that does not wait for the answer of an action.
pub(crate) async fn task_actions(task: Task<UiMessage>) -> Vec<iced_runtime::Action<UiMessage>> {
    use iced::futures::StreamExt;
    let Some(stream) = iced_runtime::task::into_stream(task) else { return Vec::new() };
    stream.collect().await
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
        | UiMessage::OpenAddAccount(..)
        | UiMessage::Window(windows::Msg::Relaunch)
        | UiMessage::Notify(notify::Msg::Timer(..))
        | UiMessage::Record(_)
        | UiMessage::Voice(_)
        | UiMessage::Video(_)
        | UiMessage::Logout
        | UiMessage::Message(message::Msg::CopyImage(..) | message::Msg::PressButton(..)) => false,
        UiMessage::KeyPressed(Key::Character(c), mods) => !(c == "p" && mods.command() && mods.shift()),
        UiMessage::InWindow(_, m)
        | UiMessage::Message(message::Msg::Menu(m))
        | UiMessage::Media(media_view::Msg::ItemAct(m)) => local(m),
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
    let open = app.pane.selected_chat_id.as_deref().map(|c| st.timeline_indices(c)).filter(|m| !m.is_empty());
    let msgs: Vec<(String, String)> = match open {
        Some(idx) => idx.iter().map(|&i| (st.messages[i].chat_id.clone(), st.messages[i].id.clone())).collect(),
        None => vec![("!note_self:beeper.com".into(), "none".into())],
    };
    let open = open.is_some();
    let accounts: Vec<String> = st.accounts.iter().map(|a| a.account_id.clone()).collect();
    let rail = crate::rail_accounts(&st);
    let spaces: Vec<String> = crate::rail_spaces(&st).into_iter().filter(|s| s != crate::LABEL_NUDGE).collect();
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
    let needs_chat = [16, 18, 63].contains(&branch);
    if (needs_msg && !open) || (needs_chat && app.pane.selected_chat_id.is_none()) {
        return (what, UiMessage::Noop);
    }
    let m = match branch {
        0 => UiMessage::ChatList(chat_list::Msg::SelectTab(tab)),
        1 => UiMessage::SelectChat(chat),
        2 => UiMessage::ChatList(chat_list::Msg::Click(chat)),
        3 => UiMessage::ChatList(chat_list::Msg::OpenMenu(chat)),
        4 => UiMessage::ChatList(chat_list::Msg::ToggleFlag(chat, flag)),
        5 => UiMessage::ChatList(chat_list::Msg::ToggleRead(chat)),
        6 => UiMessage::Reminders(reminders::Msg::Set(chat, rng.coin().then_some(now + 3_600_000))),
        7 => UiMessage::Message(message::Msg::OpenMenu(msg)),
        8 => UiMessage::Message(message::Msg::Click(msg)),
        9 => UiMessage::Message(message::Msg::DoubleClick(msg)),
        10 => UiMessage::Composer(composer::Msg::StartReply(msg)),
        11 => UiMessage::Composer(composer::Msg::StartEdit(msg, text)),
        12 => UiMessage::Message(message::Msg::AskDelete(vec![msg])),
        13 => UiMessage::Message(message::Msg::ToggleReaction(mchat, msg, "👍".into())),
        14 => UiMessage::Message(message::Msg::ToggleBookmark(msg)),
        15 => UiMessage::Composer(composer::Msg::Changed(text)),
        16 => UiMessage::Composer(composer::Msg::Submit),
        17 => UiMessage::Composer(composer::Msg::CancelMode),
        18 => UiMessage::Message(message::Msg::Step(rng.coin(), rng.coin())),
        19 => UiMessage::Settings(settings::Msg::Open(*rng.pick(&SettingsTab::ALL))),
        20 => UiMessage::Settings(settings::Msg::SelectTab(*rng.pick(&SettingsTab::ALL))),
        21 => UiMessage::Settings(settings::Msg::Query(text)),
        22..=24 => {
            UiMessage::Settings(settings::Msg::SetPref(pref.key, rng.below(usize::from(pref.max_value()) + 1) as u8))
        }
        25 => UiMessage::Hotkeys(hotkeys::Msg::Toggle(rng.pick(&hotkeys))),
        26 => UiMessage::Hotkeys(hotkeys::Msg::RevertAll),
        27 => UiMessage::Zoom(zoom::Msg::Step(*rng.pick(&[-1, 0, 1]))),
        28 => UiMessage::Settings(settings::Msg::SelectTheme(*rng.pick(&ThemeChoice::ALL))),
        29 => UiMessage::Settings(settings::Msg::SystemTheme(
            *rng.pick(&[iced::theme::Mode::Dark, iced::theme::Mode::Light]),
        )),
        30 => UiMessage::ToggleRightPanel,
        31 => UiMessage::CommandBar(commands::Msg::TogglePalette),
        32 => UiMessage::CommandBar(commands::Msg::Query(text)),
        33 => UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu),
        34 => UiMessage::ChatList(chat_list::Msg::PickFilter(rng.below(8) as u8)),
        35 => UiMessage::NewChat(new_chat::Msg::Open),
        36 => UiMessage::NewChat(new_chat::Msg::Query(text)),
        37 => UiMessage::ChatList(chat_list::Msg::BeginSelecting(chat)),
        38 => UiMessage::ChatList(chat_list::Msg::ToggleSelected(chat)),
        39 => UiMessage::ChatList(chat_list::Msg::SelectionSet(flag, rng.coin())),
        40 => UiMessage::CommandBar(commands::Msg::SearchRoom(text)),
        41 => UiMessage::Media(media_view::Msg::Open),
        42 => UiMessage::CloseModals,
        43 => UiMessage::Composer(composer::Msg::ToggleAttachMenu),
        44 if rng.coin() => UiMessage::Window(windows::Msg::OpenChat(chat)),
        44 => UiMessage::Window(windows::Msg::OpenSpace(rng.pick(&spaces).clone())),
        45 => UiMessage::Window(windows::Msg::CloseRequested(*rng.pick(&windows(app)))),
        46..=48 => UiMessage::KeyPressed(key, mods),
        49 if !deletable.is_empty() => UiMessage::ChatList(chat_list::Msg::AskDelete(rng.pick(&deletable).clone())),
        50 => UiMessage::Reminders(reminders::Msg::Menu),
        51 => UiMessage::CommandBar(commands::Msg::ChatSub(Some(rng.below(SUB_MENU_W.len()) as u8))),
        52 => UiMessage::ChatList(chat_list::Msg::ArchiveStale),
        53 => UiMessage::Forward(forward::Msg::Open(vec![msg])),
        54 if app.forward.is_some() => UiMessage::Forward(forward::Msg::Pick(chat)),
        55 => UiMessage::Rail(rail::Msg::Select(rng.coin().then(|| rng.pick(&accounts).clone()))),
        56 if !hits.is_empty() => UiMessage::Settings(settings::Msg::Pick(*rng.pick(&hits))),
        57 => UiMessage::Window(windows::Msg::Focused(rng.coin())),
        58 => UiMessage::Hotkeys(hotkeys::Msg::Query(text)),
        59 => UiMessage::Accounts(accounts::Msg::Open(rng.coin().then(|| rng.pick(&accounts).clone()))),
        60 => UiMessage::Accounts(accounts::Msg::SetProps(
            rng.pick(&accounts).clone(),
            beeper_core::models::AccountProps { notifications: rng.coin(), previews: rng.coin(), silent: rng.coin() },
        )),
        61 => UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("account:{}", rng.pick(&rail)))),
        62 => UiMessage::Message(message::Msg::Select(vec![msg])),
        63 => UiMessage::Message(message::Msg::CopySelected(rng.coin())),
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
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    press(&mut app, true, typewrite("hello big world").chain([key(Named::Backspace, WORD)]));
    assert_eq!(text(&app), "hello big ");
    press(&mut app, true, [key(Named::Backspace, WORD)]);
    assert_eq!(text(&app), "hello ");
    press(&mut app, true, [key(Named::Home, Modifiers::empty()), key(Named::Delete, WORD)]);
    assert_eq!(text(&app), " ");
    press(&mut app, true, typewrite("a bc"));
    let _ =
        app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Select(text_editor::Motion::Left))));
    let _ = app.update(UiMessage::Composer(composer::Msg::DeleteTo(text_editor::Motion::WordLeft, true)));
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
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
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
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    let tick = || key(Key::Character("`".into()), Modifiers::empty());
    press(&mut app, true, typewrite("b").chain([key(Named::Home, Modifiers::SHIFT), tick()]));
    assert_eq!(text(&app), "`b`", "a backtick formats the selection as code");
    press(&mut app, true, [tick()]);
    assert_eq!(text(&app), "`b``", "a backtick without a selection is a backtick");
    let tilde = iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Character("`".into()),
        modified_key: Key::Character("~".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backquote),
        location: keyboard::Location::Standard,
        modifiers: Modifiers::SHIFT,
        repeat: false,
        text: Some("~".into()),
    });
    press(&mut app, true, [tilde]);
    assert_eq!(text(&app), "`b``~", "Shift and the backtick key type a tilde");
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("b".into())));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(note.into(), ChatFlag::Pinned)));
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
    let files = [file("a.png", &png), file("b.pdf", b"%PDF")];
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files.iter().cloned().map(Ok).collect())));
    assert!(simulator(app.view_main()).find("PDF ・ 4 B").is_ok(), "a file shows its type and size");
    let card = file("c.vcf", b"BEGIN:VCARD\nFN:Jo Doe\nTEL:+1\nTEL:+2\nEND:VCARD\n");
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Ok(card)])));
    let shown = |t: &str| simulator(app.view_main()).find(t).is_ok();
    assert!(["Jo Doe", "Phone Numbers", "+1", "+ 1 more"].into_iter().all(shown), "a contact card");
    let _ = app.update(UiMessage::Composer(composer::Msg::DiscardAttachment(2)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("hi".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let last = |app: &BeeperGuiApp, n: usize| {
        let st = app.store.read();
        let ids = &st.timeline_indices(note);
        let msgs = ids[ids.len() - n..].iter().map(|&i| &st.messages[i]);
        msgs.map(|m| (m.text.clone(), m.attachments.iter().map(|a| a.attachment_type.clone()).collect())).collect()
    };
    let sent: Vec<(Option<String>, Vec<String>)> = last(&app, 3);
    let want = [(Some("hi".into()), vec![]), (None, vec!["img".into()]), (None, vec!["unknown".into()])];
    assert_eq!(sent, want, "the text first, then each attachment");
    assert!(app.pane.attachments.is_empty() && app.pane.composer.text().is_empty());
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files[1..].iter().cloned().map(Ok).collect())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("cap".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    assert_eq!(last(&app, 1), [(Some("cap".into()), vec!["unknown".into()])], "one attachment has the text as caption");
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files.iter().cloned().map(Ok).collect())));
    let _ = app.update(UiMessage::Composer(composer::Msg::DiscardAttachment(0)));
    assert_eq!(app.pane.attachments.len(), 1);
    let _ = app.update(UiMessage::Composer(composer::Msg::DiscardAttachment(0)));
    let id = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::Composer(composer::Msg::FileHover(id, true)));
    assert!(simulator(app.view_main()).find("Drop files here to attach").is_ok(), "a drag shows the drop zone");
    let _ = app.update(UiMessage::Modifiers(Modifiers::SHIFT));
    // winit gives each file of a drop in its own event; Desktop's `onDrop` attaches them together, in their order.
    let _ = app.update(UiMessage::Composer(composer::Msg::FileDropped(id, "/a".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::FileDropped(id, "/b".into())));
    assert_eq!(app.dropped, ["/a", "/b"].map(std::path::PathBuf::from), "the files of a drop wait for each other");
    let _ = app.update(UiMessage::Composer(composer::Msg::DropEnded));
    assert!(app.dropped.is_empty(), "the drop loads its files at once");
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files.iter().cloned().map(Ok).collect())));
    assert!(!app.dropping && app.pane.attachments.is_empty(), "a drop with Shift sends");
    assert_eq!(last(&app, 2), [(None, vec!["img".into()]), (None, vec!["unknown".into()])], "all the files at once");
    let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files[1..].iter().cloned().map(Ok).collect())));
    assert_eq!(app.pane.attachments.len(), 1, "a file picked after a drop with Shift waits for the send");
    let _ = app.update(UiMessage::Composer(composer::Msg::DiscardAttachment(0)));
    let _ = app.update(UiMessage::Composer(composer::Msg::FileHover(id, true)));
    let _ = app.update(UiMessage::Composer(composer::Msg::FileDropped(id, "/none".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::DropEnded));
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files[1..].iter().cloned().map(Ok).collect())));
    assert_eq!(app.pane.attachments.len(), 1, "a drop attaches");
    let _ = app.update(UiMessage::Composer(composer::Msg::DiscardAttachment(0)));
    // Desktop's `processAttachment`: a video goes as a video with its measured size and length.
    let path = std::env::temp_dir().join(format!("beeper-rs-{}-c.mp4", std::process::id()));
    let lavfi = ["-f", "lavfi", "-i", "testsrc=size=64x48:rate=10", "-t", "1.5", "-pix_fmt", "yuv420p", "-y"];
    assert!(std::process::Command::new("ffmpeg").args(lavfi).arg(&path).output().unwrap().status.success());
    let clip = portal::load_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!((clip.mime.as_str(), clip.size, clip.duration), ("video/mp4", (64, 48), Some(1.5)));
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Ok(clip)])));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    let a = &st.messages[*st.timeline_indices(note).last().unwrap()].attachments[0];
    let size = a.size.as_ref().map(|s| (s.width, s.height));
    assert_eq!((a.attachment_type.as_str(), size, a.duration), ("video", Some((Some(64), Some(48))), Some(1.5)));
    drop(st);
    // The room features limit the files, and a file that does not load shows "Couldn't attach %s".
    let i = app.store.read().resolve_chat_idx(note).unwrap();
    app.store.write().chats[i].file.get_mut("m.file").unwrap().max_size = Some(3);
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(files.iter().cloned().map(Ok).collect())));
    assert_eq!(app.pane.attachments.len(), 1, "the image attaches");
    assert_eq!(app.status_banner, "File size (4 B) exceeds the maximum allowed size (3 B)");
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Err("Couldn't attach c.mp4".into())])));
    assert_eq!(app.status_banner, "Couldn't attach c.mp4");
    // Desktop's `attachFiles` adds a toast for each file that does not attach, and they stack.
    let two = vec![Err("Couldn't attach d.mp4".into()), Err("Couldn't attach e.mp4".into())];
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(two)));
    assert_eq!(app.status_banner, "Couldn't attach e.mp4", "the last file's toast is the newest");
    let older: Vec<&str> = app.older_toasts.iter().map(|t| t.0.text.as_str()).collect();
    assert!(
        older.ends_with(&["Couldn't attach c.mp4", "Couldn't attach d.mp4"]),
        "the others stack in order: {older:?}"
    );
    assert!(app.older_toasts.last().is_some_and(|t| t.0.error), "an error toast");
}

#[test]
fn pastes_attach_files_like_desktop() {
    let mut app = demo_app("paste");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    // Ctrl+V (Cmd+V on macOS) in the composer reads the files of the clipboard first, and the window does not paste a second time.
    let id = app.main_window.expect("a main window");
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id));
    sim.click(iced::widget::Id::new("composer-input")).expect("a composer");
    let _ = sim.simulate([key(Key::Character("v".into()), Modifiers::COMMAND)]);
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    assert!(
        matches!(msgs[..], [UiMessage::Composer(composer::Msg::Edit(_)), UiMessage::Composer(composer::Msg::Paste)]),
        "{msgs:?}"
    );
    let files = app.update(UiMessage::Composer(composer::Msg::Pasted(Ok(Clip::Files(vec!["/tmp/a.pdf".into()])))));
    assert_eq!(files.units(), 1, "copied files load like picked files");
    let _ = app.update(UiMessage::Composer(composer::Msg::Pasted(Ok(Clip::Image((vec![1], (3, 2)))))));
    assert_eq!(app.pane.attachments[0].size, (3, 2), "a copied image attaches");
    assert_eq!(
        app.update(UiMessage::Composer(composer::Msg::Pasted(Ok(Clip::Text)))).units(),
        2,
        "a text goes into the composer"
    );
}

#[test]
fn auto_sent_gifs_keep_the_composer_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("gifsend");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("autoSendGifs", 1)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("keep".into())));
    let mut gif = Vec::new();
    ::image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut gif), ::image::ImageFormat::Gif).unwrap();
    let frames = decode_media(gif.clone()).unwrap();
    let g = beeper_core::cloud::KlipyGif {
        id: "g".into(),
        preview_url: String::new(),
        preview_size: (2, 2),
        gif_url: String::new(),
        gif_size: (2, 2),
        gif_bytes: None,
        mp4_url: String::new(),
        mp4_size: (2, 2),
    };
    let size = g.gif_size;
    let gif = PendingMedia {
        bytes: gif,
        name: "g.gif".into(),
        mime: "image/gif".into(),
        is_gif: true,
        size,
        frames,
        duration: None,
        sticker: None,
    };
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Downloaded(Ok(gif))));
    let st = app.store.read();
    let last = &st.messages[*st.timeline_indices(note).last().unwrap()];
    assert!(last.text.is_none() && last.attachments[0].is_gif == Some(true), "only the GIF goes");
    assert_eq!(app.pane.composer.text(), "keep", "the composer keeps its text");
}

#[test]
fn gif_picker_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("gifpicker");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    app.pane.picker = Some(Picker::Gif);
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Query("cats".into())));
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Loaded("ca".into(), false, Err("late".into()))));
    assert!(app.gif_loading && app.gif_error.is_none(), "an older query does not change the picker");
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Loaded("cats".into(), false, Ok((Vec::new(), String::new())))));
    assert!(simulator(app.view_main()).find("No results. Try searching something else?").is_ok());
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Loaded("cats".into(), false, Err("boom".into()))));
    let text = "Something went wrong (boom). Try searching something else?";
    assert!(simulator(app.view_main()).find(text).is_ok());
    // Desktop's pages: the next page joins the carousel without the GIFs that it has already.
    let gif = |id: &str| beeper_core::cloud::KlipyGif {
        id: id.into(),
        preview_url: format!("https://x/{id}.gif"),
        preview_size: (100, 100),
        gif_url: String::new(),
        gif_size: (1, 1),
        gif_bytes: None,
        mp4_url: String::new(),
        mp4_size: (1, 1),
    };
    let page = |ids: &[&str], next: &str| Ok((ids.iter().map(|i| gif(i)).collect(), next.to_string()));
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Loaded("cats".into(), false, page(&["a", "b"], "2"))));
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Scrolled(0.0, 1000.0, 400.0)));
    assert!(!app.gif_loading, "the start of the carousel loads nothing");
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Scrolled(520.0, 1000.0, 400.0)));
    assert!(app.gif_loading, "the end of the carousel loads the next page");
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Loaded("cats".into(), true, page(&["b", "c"], ""))));
    assert_eq!(app.gifs.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    let _ = app.update(UiMessage::Gifs(gifs::Msg::Scrolled(600.0, 1000.0, 400.0)));
    assert!(!app.gif_loading, "the last page has no next page");
    // Desktop's keys: Down selects, Left and Right move, Enter sends, Escape closes.
    let key =
        |app: &mut BeeperGuiApp, k: Named| drop(app.update(UiMessage::KeyPressed(Key::Named(k), Modifiers::empty())));
    key(&mut app, Named::ArrowDown);
    key(&mut app, Named::ArrowRight);
    key(&mut app, Named::ArrowRight);
    key(&mut app, Named::ArrowRight);
    key(&mut app, Named::ArrowLeft);
    assert_eq!(app.gif_sel, Some(1));
    key(&mut app, Named::ArrowUp);
    assert_eq!(app.gif_sel, None, "Up goes back to the search");
    key(&mut app, Named::ArrowDown);
    key(&mut app, Named::Enter);
    assert_eq!(app.pane.picker, None, "Enter sends the GIF and closes the picker");
    app.pane.picker = Some(Picker::Gif);
    key(&mut app, Named::Escape);
    assert_eq!(app.pane.picker, None);
}

/// Desktop keeps the chat list filters in `threadsFilters`, and a later launch shows them again.
#[test]
fn chat_list_filters_stay_after_a_relaunch_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("filters-relaunch");
    let relaunch = |app: &BeeperGuiApp| BeeperGuiApp::new(app.store.clone()).list.filters;
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::PickFilter(1 << 1)));
    assert_eq!(relaunch(&app), 1 << 1, "Drafts");
    let path = app.store.read().config.data_dir.join("threadsFilters");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"["drafts"]"#, "Desktop's value");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("y".into()), Modifiers::CTRL | Modifiers::SHIFT));
    assert_eq!(relaunch(&app), 1 | 1 << 1, "Filter: Unread adds Unread");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ClearFilters));
    assert_eq!(relaunch(&app), 0);
    std::fs::write(&path, r#"["unread","default","nope"]"#).unwrap();
    assert_eq!(relaunch(&app), 1, "without unknown values");
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
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("telegram".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(
        !app.show_command_palette && app.list.selected_account_filter == target,
        "the network name finds the account"
    );
    let _ = app.update(UiMessage::RunHotkey("Filter By Account"));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.list.selected_account_filter.is_none(), "All Accounts clears the filter");
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
    let _ = app.update(UiMessage::Toast(toasts::Msg::DismissBanner));
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert_eq!(app.status_banner, "Incognito mode on.");
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert!(!on(&app, "incognito") && app.status_banner == "Undo: Enable Incognito Mode");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("typingIndicators", 1)));
    let _ = app.update(UiMessage::Toast(toasts::Msg::DismissBanner));
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("incognito", 1)));
    assert!(app.status_banner.is_empty(), "Settings change incognito mode without a toast");
    let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
    assert_eq!(app.status_banner, "Incognito mode off.");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("incognito", 1)));
    assert!(app.store.read().incognito());
    app.store.write().features.usage = trial("2020-01-04T00:00:00Z");
    assert!(!app.store.read().incognito() && app.store.read().typing_notices(), "the trial ended");
}

#[test]
fn status_pills_like_desktop() {
    let mut app = demo_app("pills");
    let texts = |app: &BeeperGuiApp| app.status_pills().into_iter().map(|p| p.1).collect::<Vec<_>>();
    assert!(texts(&app).is_empty());
    let _ = app.update(UiMessage::Online(false));
    app.store.write().syncing = true;
    assert_eq!(texts(&app), ["Offline", "Syncing..."]);
    assert!(simulator(app.view_main()).find("Offline").is_ok());
    let _ = app.update(UiMessage::Online(true));
    app.store.write().syncing = false;
    for a in app.store.write().bridge_accounts.iter_mut().filter(|a| a.account_id == "acc_whatsapp") {
        a.state = "TRANSIENT_DISCONNECT".into();
    }
    assert_eq!(texts(&app), ["Connecting..."]);
    let _ = app.update(UiMessage::PillHover(true));
    assert_eq!(texts(&app), ["Connecting... (WhatsApp)"], "the pointer shows the accounts");
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("acc_telegram".into()))));
    assert!(texts(&app).is_empty(), "an account space shows only its own account");
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
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("compact chat list".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.settings_window.is_some() && app.active_settings_tab == SettingsTab::Appearance);
    assert_eq!(app.pref_flash.map(|f| f.0), Some("compactChatList"));
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("Accounts → Telegram".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    // Desktop's network hit selects the account on that network.
    let telegram = app
        .store
        .read()
        .bridge_accounts
        .iter()
        .find(|a| a.bridge_id.contains("telegram"))
        .map(|a| a.account_id.clone());
    assert!(telegram.is_some(), "the demo has a Telegram account");
    assert!(app.active_settings_tab == SettingsTab::Accounts && app.account_page == telegram);
}

/// Desktop's command bar: react by typing an emoji, filter by type, a new group and chat selection mode.
#[test]
fn command_bar_commands_like_desktop() {
    let mut app = demo_app("cmd-more");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let mid = {
        let st = app.store.read();
        app.timeline(&st).0.iter().rev().find(|m| m.reactable()).expect("a message").id.clone()
    };
    app.pane.selected_msgs = vec![mid.clone()];
    let titles = |app: &BeeperGuiApp, q: &str| -> Vec<String> {
        app.commands(&app.store.read(), q).iter().map(|c| c.title.clone()).collect()
    };
    assert!(titles(&app, "").contains(&"Type any emoji to react with it".to_string()));
    let react = app
        .commands(&app.store.read(), "🎸")
        .into_iter()
        .find(|c| c.title == "React with 🎸")
        .expect("a react command");
    let _ = app.update(react.msg.expect("an action"));
    assert!(app.store.read().message(&mid).reactions.iter().any(|r| r.reaction_key == "🎸"), "it reacts");
    assert!(titles(&app, "🎸").contains(&"Remove 🎸 reaction".to_string()), "and then removes");
    app.pane.selected_msgs.clear();
    let all = titles(&app, "");
    for t in ["Create new group", "Filter by type", "Toggle chat selection mode"] {
        assert!(all.contains(&t.to_string()), "{t}");
    }
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Filters)));
    assert!(titles(&app, "").contains(&"Unread".to_string()));
}

/// Desktop's notification of a send that failed: once, and only for messages since the start.
#[test]
fn failed_sends_notify_like_desktop() {
    let mut app = demo_app("failed-notify");
    let m = {
        let mut st = app.store.write();
        let i = st.messages.iter().position(|m| m.is_sender == Some(true)).expect("an own message");
        let old = st.messages[i].clone();
        st.update_message(i, |m| {
            m.timestamp = beeper_core::config::now_rfc3339_millis();
            m.send_error = Some(beeper_core::models::SendError { message: Some("no".into()), retrying: false });
        });
        (old, st.messages[i].clone())
    };
    let key = format!("failed:{}", m.1.id);
    let mut stale = m.0.clone();
    stale.send_error = m.1.send_error.clone();
    let _ = app.notify_failed(&stale);
    assert!(!app.notified.contains(&key), "a message from before the start does not notify");
    let _ = app.update(UiMessage::Upserted(Box::new(m.1.clone())));
    assert!(app.notified.contains(&key), "a failed send notifies");
}

/// Desktop's `b2t`: a click on a mention of someone else opens Message and the profile on the network.
#[test]
fn mention_menu_like_desktop() {
    let mut app = demo_app("mention-menu");
    let (cid, user) = {
        let st = app.store.read();
        let c = st.chats.iter().find(|c| c.chat_type == "group").expect("a group");
        (c.id.clone(), c.participants.items.iter().find(|u| u.is_self != Some(true)).expect("a member").clone())
    };
    let _ = app.update(UiMessage::OpenUrl(format!("{}{cid} {}", crate::MENTION, user.id)));
    let key = format!("mention:{cid} {}", user.id);
    assert_eq!(app.list.chat_menu.as_ref().map(|m| m.0.as_str()), Some(key.as_str()));
    let items = info::info_menu(&app.store.read(), &key, |_| true);
    let first = items[0].as_ref().expect("an item");
    assert!(first.0.starts_with("Message ") && first.1.is_some(), "{}", first.0);
}

#[test]
fn closing_the_main_window_hides_it_like_desktop() {
    let mut app = demo_app("closehide");
    let main = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("showTrayIcon", 0)));
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(main)));
    assert!(app.hidden, "the main window hides without a tray icon too");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    assert_eq!(app.sub_windows(), [app.settings_window.expect("the settings window")]);
    let _ = app.update(UiMessage::Window(windows::Msg::Tray(Ok(tray::Event::Activate))));
    assert!(!app.hidden, "a click on the tray icon shows the windows");
    // Desktop's main process reads `quit_on_close` once at launch, and the setting asks for a relaunch.
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("quitOnClose", 1)));
    assert_eq!(app.relaunch_toast, Some("Preference changed"));
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(main)));
    assert!(app.hidden, "until a relaunch, the window still hides");
    let mut app = BeeperGuiApp::boot(app.store.clone()).0;
    let main = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(main)));
    assert!(!app.hidden, "after a relaunch with Quit on close, the window closes");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("quitOnClose", 0)));
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(main)));
    assert!(!app.hidden, "turning it off also waits for a relaunch");
}

/// Desktop has one window, so closing it while signed out hides the app too, and opening the app again shows it.
#[test]
fn closing_the_sign_in_window_hides_it_like_desktop() {
    let mut app = demo_app("closelogin");
    let login = iced::window::Id::unique();
    (app.main_window, app.login_window) = (None, Some(login));
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(login)));
    assert!(app.hidden && app.login_window == Some(login), "the sign-in window hides instead of quitting");
    let _ = app.update(UiMessage::Window(windows::Msg::FocusRequested(None)));
    assert!(!app.hidden, "opening the app again shows it");
}

#[test]
fn an_edit_keeps_the_draft_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("editdraft");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let (_, own, _) = app.store.write().send_message(note, Some("old".into()), None, None).unwrap();
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("my draft".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::StartReply(own.clone())));
    let state = |app: &BeeperGuiApp| (app.pane.composer.text(), app.pane.reply_to_message_id.clone());
    let draft = ("my draft".to_string(), Some(own.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(own.clone(), "old".into())));
    assert_eq!(state(&app), ("old".into(), None), "the edit puts the draft aside");
    let _ = app.update(UiMessage::Composer(composer::Msg::CancelMode));
    assert_eq!(state(&app), draft, "a canceled edit gives the draft back");
    let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(own.clone(), "old".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("new".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    assert_eq!(state(&app), draft, "a sent edit gives the draft back");
    let st = app.store.read();
    assert_eq!(st.message(&own).text.as_deref(), Some("new"));
    drop(st);
    let _ = app.update(UiMessage::Composer(composer::Msg::CancelMode));
    assert_eq!(state(&app), (draft.0.clone(), None), "the close button removes only the quoted message");
    let _ = app.update(UiMessage::Composer(composer::Msg::StartReply(own.clone())));
    let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(own.clone(), "new".into())));
    let other = app.visible_chats(&app.store.read()).into_iter().find(|c| c.id != note).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    let st = app.store.read();
    let saved = st.chats[st.resolve_chat_idx(note).unwrap()].draft.clone().expect("a draft");
    assert_eq!((saved.text, saved.reply_to_message_id), (Some(draft.0), draft.1), "a left edit saves the draft");
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
    let _ = app.update(UiMessage::CommandBar(commands::Msg::NoReply(false)));
    assert!(simulator(app.view_main()).find("Remind Me marks this chat as new at the scheduled time").is_ok());
    let counted = labels(&app, "3");
    for unit in ["3 minutes", "3 hours", "3 days", "3 weeks"] {
        assert!(counted.contains(&unit.into()), "{unit} in {counted:?}");
    }
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("friday at 5pm".into())));
    assert_eq!(labels(&app, "friday at 5pm")[0], "friday at 5pm", "the typed time comes first");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let at = parse_when_millis("friday at 5pm", now_unix_millis()).unwrap();
    let st = app.store.read();
    let r = st.chats[st.resolve_chat_idx(&id).unwrap()].reminder.clone().expect("a reminder");
    assert!(r.remind_at_ms == at && !r.dismiss_on_incoming_message, "Always keeps the reminder after a reply");
    drop(st);
    let pill = format!("{} {}", reminder_day(at), clock(at, false));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_conversation());
        let _ = sim.click(pill.as_str()).expect("the reminder pill");
        sim.into_messages().collect()
    };
    let opens = matches!(&picked[..], [UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(c)))] if *c == id);
    assert!(opens, "the header pill opens Remind Me");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(id.clone()))));
    assert!(app.cmd_no_reply, "the choice starts at the setting");
    assert_eq!(labels(&app, "").last().map(String::as_str), Some("dismiss reminder"));
    assert_eq!(app.store.read().settings.last_time, "friday at 5pm", "the phrase is kept for Last Used");
    assert_eq!(labels(&app, "")[0], "last used");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(1))));
    assert!(simulator(app.view_main()).find("Last Used").is_ok(), "the chat menu offers the last phrase too");
    assert!(simulator(app.view_main()).find("Upgrade to Beeper Plus").is_err(), "free uses are left");
    let other = app.visible_chats(&app.store.read()).iter().find(|c| c.reminder.is_none()).map(|c| c.id.clone());
    let other = other.expect("a chat without a reminder");
    app.store.write().features.usage = serde_json::json!({ "remind-later": { "used": 5 } });
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(other.clone())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(1))));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("Tomorrow").expect("Tomorrow");
        let _ = sim.click("Custom…").expect("Custom…");
        let _ = sim.click("Upgrade to Beeper Plus").expect("Upgrade to Beeper Plus");
        let acts = |m: &UiMessage| {
            matches!(
                m,
                UiMessage::Settings(settings::Msg::Open(_))
                    | UiMessage::Commands(_)
                    | UiMessage::CommandBar(commands::Msg::OpenSubMenu(_))
                    | UiMessage::Reminders(reminders::Msg::Set(..))
            )
        };
        sim.into_messages().filter(acts).collect()
    };
    let upgrade = matches!(&picked[..], [UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus))]);
    assert!(upgrade, "without free uses only the upgrade works");
}

/// Desktop's chat list cursor (`threadCursorID`) for the keyboard: Escape leaves the open chat with the cursor on it,
/// a second Escape points at the first chat, ↓ and J, ↑ and K move the cursor without opening a chat, and Enter
/// opens the chat of the cursor.
#[test]
fn chat_list_cursor_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("listcursor");
    let main = app.main_window.expect("a main window");
    let ids = app.list_ids(&app.store.read());
    let key = |app: &mut BeeperGuiApp, k: Named| {
        drop(app.update(UiMessage::WindowKey(main, Key::Named(k), Modifiers::empty())))
    };
    let text = |app: &mut BeeperGuiApp, t: &str| drop(app.update(UiMessage::WindowText(main, t.into())));
    let _ = app.update(UiMessage::SelectChat(ids[2].clone()));
    key(&mut app, Named::Escape);
    assert_eq!(app.pane.selected_chat_id, None, "Escape leaves the chat");
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[2]), "the cursor stays on the chat that was open");
    key(&mut app, Named::ArrowDown);
    text(&mut app, "j");
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[4]), "↓ and J point at the next chats");
    assert_eq!(app.pane.selected_chat_id, None, "pointing opens no chat");
    text(&mut app, "k");
    key(&mut app, Named::ArrowUp);
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[2]), "↑ and K point at the previous chats");
    key(&mut app, Named::Escape);
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[0]), "Escape with no chat open points at the first chat");
    key(&mut app, Named::ArrowUp);
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[0]), "the cursor stops at the first chat");
    key(&mut app, Named::ArrowDown);
    key(&mut app, Named::Enter);
    assert_eq!(app.pane.selected_chat_id.as_ref(), Some(&ids[1]), "Enter opens the chat of the cursor");
    assert_eq!(app.list.cursor, None, "an open chat drops the cursor");
    text(&mut app, "j");
    assert_eq!(app.pane.composer.text(), "j", "with a chat open, J types in the composer");
    let _ = app.update(UiMessage::SelectChat(ids[1].clone()));
    key(&mut app, Named::Escape);
    let _ = app.update(UiMessage::RunHotkey("Search"));
    key(&mut app, Named::ArrowDown);
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[1]), "the command bar takes the arrows");
}

/// Desktop's `reset` empties the command bar each time it closes, so a time typed in Remind Me or a search that was
/// closed does not come back in the next search.
#[test]
fn command_bar_opens_empty_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("cmdbarempty");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let _ = app.update(UiMessage::KeyPressed(Key::Character("l".into()), Modifiers::CTRL));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("in 1 minute".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(!app.show_command_palette, "the time sets the reminder and closes the bar");
    assert!(app.store.read().find_chat(&id).and_then(|c| c.reminder.clone()).is_some());
    let _ = app.update(UiMessage::RunHotkey("Search"));
    assert!(app.show_command_palette);
    assert_eq!(app.command_palette_query, "", "the search does not start with the time");
    assert!(simulator(app.view_main()).find("in 1 minute").is_err());
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("lena".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(!app.show_command_palette);
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    assert_eq!(app.command_palette_query, "", "Escape emptied the bar");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("pebble".into())));
    let _ = app.update(UiMessage::RunHotkey("Search: Room"));
    assert_eq!(app.command_palette_query, "pebble", "an open bar keeps its text for the search of the chat");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::RunHotkey("Search: Room"));
    assert_eq!(app.command_palette_query, "", "a closed bar opens empty for the search of the chat");
}

/// Desktop's chat hotkeys take `selectedOrCursorThread`, so Ctrl E and E archive the chat of the cursor while no chat
/// is open. `toggleThreadArchive` first moves the cursor to the next chat, and a pinned chat stays with the toast
/// "Pinned chats can't be archived".
#[test]
fn archive_keys_take_the_chat_of_the_cursor_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("archivecursor");
    let main = app.main_window.expect("a main window");
    let ids = app.list_ids(&app.store.read());
    let flag = |app: &BeeperGuiApp, id: &str, f: ChatFlag| app.store.read().find_chat(id).expect("a chat").flag(f);
    assert!(flag(&app, &ids[0], ChatFlag::Pinned), "the demo list starts with a pinned chat");
    let _ = app.update(UiMessage::SelectChat(ids[0].clone()));
    let _ = app.update(UiMessage::WindowKey(main, Key::Named(Named::Escape), Modifiers::empty()));
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[0]));
    let _ = app.update(UiMessage::WindowKey(main, Key::Character("e".into()), Modifiers::CTRL));
    assert_eq!(app.status_banner, "Pinned chats can't be archived");
    assert!(!flag(&app, &ids[0], ChatFlag::Archived), "the pinned chat stays");
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[1]), "the cursor moves to the next chat");
    let row = ids.iter().position(|id| !flag(&app, id, ChatFlag::Pinned)).expect("a chat that is not pinned");
    app.list.cursor = Some(ids[row].clone());
    let _ = app.update(UiMessage::WindowText(main, "e".into()));
    assert!(flag(&app, &ids[row], ChatFlag::Archived), "E archives the chat of the cursor");
    assert_eq!(app.list.cursor.as_ref(), Some(&ids[row + 1]));
    assert_eq!(app.pane.selected_chat_id, None, "archiving the chat of the cursor opens no chat");
    let _ = app.update(UiMessage::RunHotkey("Chat: Toggle Pin"));
    assert!(flag(&app, &ids[row + 1], ChatFlag::Pinned), "Ctrl P pins the chat of the cursor");
    let _ = app.update(UiMessage::SelectChat(ids[row + 2].clone()));
    let _ = app.update(UiMessage::WindowText(main, "e".into()));
    assert_eq!(app.pane.composer.text(), "e", "with a chat open, E types in the composer");
    assert!(!flag(&app, &ids[row + 2], ChatFlag::Archived));
}

/// Desktop's `scrollThreadsListToItem` with `align: "nearest"`: the chat list scrolls the chat of the cursor and the
/// open chat into view with the least scroll, for ↓ and ↑, Ctrl ] and Ctrl 1 to 9. A pinned tile brings the top of the
/// list, where the pinned grid is.
#[test]
fn chat_list_scrolls_to_the_pointed_chat_like_desktop() {
    use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, RelativeOffset, Scrollable};
    use iced::keyboard::{key::Named, Key, Modifiers};
    use iced::Rectangle;
    struct Offset(Option<f32>);
    impl Scrollable for Offset {
        fn snap_to(&mut self, _: RelativeOffset<Option<f32>>) {}
        fn scroll_to(&mut self, to: AbsoluteOffset<Option<f32>>) {
            self.0 = to.y;
        }
        fn scroll_by(&mut self, _: AbsoluteOffset, _: Rectangle, _: Rectangle) {}
    }
    let mut app = demo_app("listreveal");
    let main = app.main_window.expect("a main window");
    let ids = app.list_ids(&app.store.read());
    let scroll = app.list.scroll.0.clone();
    // Where the task scrolls the chat list to.
    let scrolled = |task: Task<UiMessage>| -> Option<f32> {
        let actions = iced::futures::executor::block_on(task_actions(task));
        let mut offset = Offset(None);
        for a in actions {
            if let iced_runtime::Action::Widget(mut op) = a {
                op.scrollable(Some(&scroll), Rectangle::default(), Rectangle::default(), Vector::ZERO, &mut offset);
            }
        }
        offset.0
    };
    // A frame of the main window, 450 px high, after the wheel moved the list `wheel` px down.
    let draw = |app: &BeeperGuiApp, wheel: f32| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 450.0), app.view(main));
        sim.point_at(Point::new(290.0, 300.0));
        let delta = ScrollDelta::Pixels { x: 0.0, y: -wheel };
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled { delta })]);
        let _ = sim.simulate([iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
    };
    let key =
        |app: &mut BeeperGuiApp, k: Named| app.update(UiMessage::WindowKey(main, Key::Named(k), Modifiers::empty()));
    let _ = app.update(UiMessage::SelectChat(ids[2].clone()));
    let _ = key(&mut app, Named::Escape);
    draw(&app, 0.0);
    assert_eq!(scrolled(key(&mut app, Named::ArrowDown)), None, "a chat in view does not scroll the list");
    let mut offsets = Vec::new();
    while let Some(next) = ids.get(ids.iter().position(|i| Some(i) == app.list.cursor.as_ref()).unwrap() + 1) {
        let y = scrolled(key(&mut app, Named::ArrowDown));
        assert_eq!(app.list.cursor.as_ref(), Some(next));
        offsets.extend(y);
    }
    assert!(!offsets.is_empty(), "the chats below the list scroll it");
    assert!(offsets.is_sorted() && offsets[0] > 0.0, "each next chat ends at the bottom: {offsets:?}");
    let _ = app.update(UiMessage::SelectChat(ids[6].clone()));
    draw(&app, 0.0);
    let mut by_hotkey = Vec::new();
    for _ in 0..ids.len() {
        by_hotkey.extend(scrolled(app.update(UiMessage::RunHotkey("Select Next Chat"))));
    }
    assert_eq!(app.pane.selected_chat_id.as_ref(), ids.last());
    assert!(!by_hotkey.is_empty() && by_hotkey.is_sorted(), "Ctrl ] scrolls like the cursor: {by_hotkey:?}");
    draw(&app, 5000.0);
    let first = app.update(UiMessage::KeyPressed(Key::Character("1".into()), Modifiers::CTRL));
    assert_eq!(app.pane.selected_chat_id.as_ref(), Some(&ids[0]));
    assert_eq!(scrolled(first), Some(0.0), "Ctrl 1 on a pinned tile scrolls to the top");
    draw(&app, 5000.0);
    let ninth = scrolled(app.update(UiMessage::KeyPressed(Key::Character("9".into()), Modifiers::CTRL)));
    assert_eq!(app.pane.selected_chat_id.as_ref(), Some(&ids[8]));
    let end = *offsets.last().unwrap();
    assert!(ninth.is_some_and(|y| y > 0.0 && y < end), "Ctrl 9 brings a chat above the list to its top: {ninth:?}");
}

#[test]
fn menu_accelerators_work_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("menukeys");
    let main = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::WindowKey(main, Key::Character("w".into()), Modifiers::COMMAND | Modifiers::SHIFT));
    assert!(!app.hidden, "Ctrl Shift W is not the Close accelerator");
    let _ = app.update(UiMessage::WindowKey(main, Key::Character("w".into()), Modifiers::COMMAND));
    assert!(app.hidden, "Ctrl W closes the main window, which hides it");
}

#[test]
fn send_later_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("sendlater");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let edit = text_editor::Edit::Paste(Arc::new("see you then".into()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit))));
    let _ = app.update(UiMessage::KeyPressed(Key::Character("l".into()), Modifiers::CTRL | Modifiers::SHIFT));
    assert_eq!(app.cmd_submenu, Some(commands::SubMenu::Schedule(Vec::new())), "Ctrl Shift L opens Send Later");
    assert!(app.cmd_no_reply, "the choice starts at If no reply");
    let footer = "Send Later messages can only send if Beeper app is running (and your computer is awake)";
    assert!(simulator(app.view_main()).find(footer).is_ok());
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("friday at 5pm".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let at = parse_when_millis("friday at 5pm", now_unix_millis()).unwrap();
    assert_eq!(app.pane.send_later, Some((at, true)));
    let when = send_day(at, false);
    assert!(simulator(app.view_main()).find(format!("{when} if no reply")).is_ok(), "the composer banner");
    let sent = app.store.read().messages.len();
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    assert_eq!((st.messages.len(), app.pane.send_later), (sent, None), "nothing sends now");
    let s = st.scheduled[0].clone();
    assert!(s.text == "see you then" && s.send_on == at && s.send_only_on_no_response && !s.canceled);
    drop(st);
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find(format!("Send {when}")).is_ok() && sim.find("see you then").is_ok(), "the timeline");
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::SendLater)));
    assert_eq!(app.visible_chats(&app.store.read()).iter().map(|c| &c.id).collect::<Vec<_>>(), [&id]);
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::RescheduleTo(vec![s.id.clone()], at + 60_000, false)));
    assert!(!app.store.read().scheduled[0].send_only_on_no_response);
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::RescheduleTo(vec![s.id.clone()], at, true)));
    let mut st = app.store.write();
    let (_, reply, _) = st.send_message(&id, Some("sure".into()), None, None).unwrap();
    let i = st.message_idx(&reply);
    let m = st.update_message(i, |m| m.is_sender = Some(false));
    drop(st);
    let _ = app.update(UiMessage::Upserted(Box::new(m)));
    assert!(app.store.read().scheduled[0].canceled, "a reply cancels an If no reply message");
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    assert!(simulator(app.view_main()).find("Send Later message canceled by reply.").is_ok());
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Unschedule(vec![s.id.clone()])));
    assert!(app.store.read().scheduled.is_empty());
    app.store.write().schedule(&id, "now".into(), None, (now_unix_millis() - 1, false), None);
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Check));
    let st = app.store.read();
    assert!(
        st.scheduled.is_empty() && st.messages.last().unwrap().text.as_deref() == Some("now"),
        "a due message sends"
    );
    drop(st);
    // Desktop's `scheduleCurrentDraft` keeps each content part, and a new file keeps the Send Later time.
    let path = std::env::temp_dir().join(format!("beeper-rs-{}-later.pdf", std::process::id()));
    std::fs::write(&path, b"%PDF").unwrap();
    let file = portal::load_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Set(Some((at, false)))));
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Ok(file.clone()), Ok(file)])));
    assert_eq!(app.pane.send_later, Some((at, false)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("two files".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let parts = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.scheduled
            .iter()
            .map(|s| (s.text.clone(), s.upload.as_ref().map(|u| st.assets[u].file_name.clone())))
            .collect::<Vec<_>>()
    };
    let name = Some(format!("beeper-rs-{}-later.pdf", std::process::id()));
    assert_eq!(parts(&app), [("two files".into(), None), (String::new(), name.clone()), (String::new(), name.clone())]);
    assert!(simulator(app.view_main()).find(name.as_deref().unwrap()).is_ok(), "a file part shows its name");
    let ids: Vec<String> = app.store.read().scheduled.iter().map(|s| s.id.clone()).collect();
    let uploads: Vec<String> = app.store.read().scheduled.iter().filter_map(|s| s.upload.clone()).collect();
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::SendNow(ids)));
    let st = app.store.read();
    let last = &st.timeline_indices(&id)[st.timeline_indices(&id).len() - 3..];
    let sent: Vec<_> = last.iter().map(|&i| (st.messages[i].text.clone(), st.messages[i].attachments.len())).collect();
    assert_eq!(sent, [(Some("two files".into()), 0), (None, 1), (None, 1)], "the files send from the uploads");
    assert!(uploads.iter().all(|u| !st.assets.contains_key(u)), "a sent Send Later file goes away");
}

#[test]
fn drafts_keep_the_composer_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("drafts");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let (_, own, _) = app.store.write().send_message(note, Some("old".into()), None, None).unwrap();
    let path = std::env::temp_dir().join(format!("beeper-rs-{}-draft.pdf", std::process::id()));
    std::fs::write(&path, b"%PDF").unwrap();
    let file = PendingMedia { name: "b.pdf".into(), ..portal::load_file(&path).unwrap() };
    std::fs::remove_file(path).unwrap();
    let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Ok(file)])));
    let text = " see https://example.com ";
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(text.into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::StartReply(own.clone())));
    let later = (now_unix_millis() + 3_600_000, true);
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Set(Some(later))));
    let _ = app.update(UiMessage::Composer(composer::Msg::RemoveLinkPreview("https://example.com".into())));
    assert!(app.pane.draft_due, "a change starts the save timer");
    let saved = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.resolve_chat_idx(note).unwrap()].draft.clone().expect("a draft")
    };
    let _ = app.update(UiMessage::SaveDrafts);
    let draft = saved(&app);
    assert!(!app.pane.draft_due, "the timer saves the draft without a chat change");
    assert_eq!(draft.text.as_deref(), Some(text), "the draft keeps the spaces");
    assert_eq!((draft.reply_to_message_id.as_ref(), draft.send_later), (Some(&own), Some(later)));
    assert_eq!(draft.discarded_links, ["https://example.com"]);
    assert_eq!(draft.attachments.len(), 1);
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(format!("{text}!"))));
    let _ = app.update(UiMessage::SaveDrafts);
    assert_eq!(saved(&app).attachments, draft.attachments, "a saved attachment is not stored again");
    let other = app.visible_chats(&app.store.read()).into_iter().find(|c| c.id != note).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    assert!(app.pane.attachments.is_empty(), "the attachments stay with their chat");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let pane = &app.pane;
    assert_eq!((pane.composer.text(), pane.reply_to_message_id.as_ref()), (format!("{text}!"), Some(&own)));
    assert_eq!(pane.send_later, Some(later));
    assert_eq!(pane.attachments.iter().map(|a| (a.name.as_str(), a.bytes.len())).collect::<Vec<_>>(), [("b.pdf", 4)]);
    assert!(matches!(pane.composer_links[..], [(_, LinkState::Discarded)]), "a removed preview stays removed");
    // Desktop's `deletePendingMessage`: a sent draft takes its file with it.
    let upload = saved(&app).attachments[0].upload_id.clone();
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Set(None)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let draft = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats[st.resolve_chat_idx(note).unwrap()].draft.clone()
    };
    assert_eq!(draft(&app), None, "a send clears the draft at once");
    assert!(!app.store.read().assets.contains_key(&upload), "the upload of the sent draft goes away");
    // A restored draft that is sent as it is goes too, with no change to save it.
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("again".into())));
    let other = app.visible_chats(&app.store.read()).into_iter().find(|c| c.id != note).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    let _ = app.update(UiMessage::SelectChat(note.into()));
    assert!(!app.pane.draft_due && draft(&app).is_some(), "the draft is back, with nothing to save");
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    assert_eq!(draft(&app), None, "a send of a restored draft clears it");
}

#[test]
fn show_replies_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("replies");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let mut st = app.store.write();
    let root = st.messages[*st.timeline_indices(&id).last().unwrap()].id.clone();
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
    assert_eq!(app.composer_placeholder(&app.store.read(), &chat), place, "the composer replies in the chain");
    let edit = text_editor::Edit::Paste(Arc::new("third answer".into()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit))));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    assert_eq!(st.messages.last().unwrap().linked_message_id.as_ref(), Some(&one), "a send replies to the message");
    drop(st);
    let _ = app.update(UiMessage::CloseModals);
    assert_eq!(app.pane.reply_thread, None, "Escape closes the replies");
    let _ = app.update(UiMessage::Message(message::Msg::Select(vec![two])));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::CTRL));
    assert_eq!(app.pane.reply_thread.as_ref(), Some(&root), "Ctrl Down shows the chain of the selected message");
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(one)));
    assert!(simulator(app.view_main()).find("Jump to Message").is_ok());
}

/// Desktop keeps the width of the thread pane in `threaded-replies-pane-fraction`, like the sidebar width.
#[test]
fn the_thread_pane_keeps_its_width_after_a_relaunch_like_desktop() {
    let mut app = demo_app("thread-fraction");
    let relaunch = |app: &BeeperGuiApp| BeeperGuiApp::new(app.store.clone()).thread_fraction;
    assert_eq!(relaunch(&app), threads::THREAD_FRACTION);
    app.pane.size = Size::new(1208.0, 800.0);
    app.thread_area.set(808.0);
    let _ = app.update(UiMessage::Thread(threads::Msg::Drag(None)));
    let _ = app.update(UiMessage::Thread(threads::Msg::Drag(Some(Point::new(900.0, 400.0)))));
    let _ = app.update(UiMessage::Thread(threads::Msg::Drag(None)));
    assert_eq!(app.thread_fraction, 0.375);
    assert_eq!(relaunch(&app), 0.375, "the dragged width");
    let _ = app.update(UiMessage::Thread(threads::Msg::ResetWidth));
    assert_eq!(relaunch(&app), threads::THREAD_FRACTION, "a double click resets it");
    let path = app.store.read().config.data_dir.join("threaded-replies-pane-fraction");
    std::fs::write(&path, "0.95").unwrap();
    assert_eq!(relaunch(&app), 0.9, "at most 0.9");
}

/// Desktop's threaded replies: the replies button and the root line in the chat, the thread pane with its divider and
/// its composer, which sends a thread reply with "Also send to chat", and "Reply in thread…".
#[test]
fn threads_like_desktop() {
    let mut app = demo_app("threads");
    let (chat, root) = ("!sl_infra:beeper.com", "msg_1007");
    app.select_chat(chat);
    {
        let st = app.store.read();
        let ids: Vec<&str> = app.timeline(&st).0.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"msg_1010") && !ids.contains(&"msg_1011"), "a reply only in the thread stays out");
    }
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Replying to: ").is_ok(), "a reply also sent to the chat shows its root");
    let _ = sim.click("4 replies").expect("the replies button under the root");
    for m in sim.into_messages().collect::<Vec<_>>() {
        let _ = app.update(m);
    }
    assert_eq!(app.pane.thread.as_ref().and_then(|t| t.thread_root.as_deref()), Some(root));
    {
        let mut sim = simulator(app.view_main());
        for t in ["Thread", "4 replies", "Eric Migicovsky", "Also sent to the chat"] {
            assert!(sim.find(t).is_ok(), "the thread pane shows {t}");
        }
    }
    let in_thread = |m: UiMessage| UiMessage::InThread(Box::new(m));
    assert!(simulator(app.view_main()).find("Also send to chat").is_err(), "an empty composer hides the toggle");
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("main draft".into())));
    let _ = app.update(UiMessage::SaveDrafts);
    let edit = text_editor::Edit::Paste(Arc::new("done".into()));
    let _ = app.update(in_thread(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit)))));
    assert!(simulator(app.view_main()).find("Also send to chat").is_ok(), "a draft shows the toggle in Slack");
    let _ = app.update(in_thread(UiMessage::Thread(threads::Msg::SetBroadcast(true))));
    let _ = app.update(in_thread(UiMessage::Composer(composer::Msg::Submit)));
    {
        let st = app.store.read();
        let m = st.messages.last().unwrap();
        assert_eq!((m.text.as_deref(), m.thread_root.as_deref(), m.broadcast), (Some("done"), Some(root), true));
        assert_eq!(m.linked_message_id.as_deref(), Some("msg_1011"), "the reply falls back to the newest reply");
        let c = st.chat(chat);
        assert_eq!(c.preview.as_ref().map(|p| p.id.as_str()), Some(m.id.as_str()), "it is also sent to the chat");
        let draft = c.draft.as_ref().and_then(|d| d.text.as_deref());
        assert_eq!(draft, Some("main draft"), "a send in the thread keeps the draft of the chat");
    }
    let thread = app.pane.thread.as_ref().unwrap();
    assert!(thread.composer.text().trim().is_empty() && !thread.broadcast, "a send clears the thread composer");
    let _ = app.update(UiMessage::Thread(threads::Msg::Hide));
    assert!(app.pane.thread.is_none());
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(root.into())));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("Reply in thread…").expect("the message menu replies in the thread");
    for m in sim.into_messages().collect::<Vec<_>>() {
        let _ = app.update(m);
    }
    assert!(app.pane.thread.is_some() && !app.show_right_panel, "the thread pane takes the place of Chat Info");
    let _ = app.update(UiMessage::ToggleRightPanel);
    assert!(app.pane.thread.is_none() && app.show_right_panel, "Chat Info takes the place of the thread pane");
    // The command bar replies in the thread of the selected message, and the chat keeps its thread pane.
    let _ = app.update(UiMessage::Message(message::Msg::Select(vec![root.into()])));
    let cmds = app.commands(&app.store.read(), "");
    let cmd = cmds.iter().find(|c| c.title == "Reply in thread").expect("a Reply in thread command");
    assert_eq!(cmd.hotkey, Some("Show Replies"));
    let _ = app.update(cmd.msg.clone().unwrap());
    let other = app.store.read().chats.iter().find(|c| c.id != chat).unwrap().id.clone();
    app.select_chat(&other);
    assert!(app.pane.thread.is_none());
    app.select_chat(chat);
    assert_eq!(app.pane.thread.as_ref().and_then(|t| t.thread_root.as_deref()), Some(root), "the thread comes back");
    // A new reply in the open thread pane is read when it comes into view.
    let mut st = app.store.write();
    let t = st.thread_send(root, None, false);
    let (_, id, _) = st.send_in_thread(chat, Some("new".into()), None, &t).unwrap();
    let i = st.message_idx(&id);
    let m = st.update_message(i, |m| m.is_sender = Some(false));
    let ci = st.chat_idx(chat);
    st.update_chat(ci, |c| drop(c.thread_reads.insert(root.into(), beeper_core::store::sort_key(0))));
    assert!(st.thread_unread(&st.chats[ci], root).0 > 0);
    drop(st);
    app.focused = true;
    let _ = app.update(UiMessage::Upserted(Box::new(m)));
    let st = app.store.read();
    assert_eq!(st.thread_unread(st.chat(chat), root), (0, 0));
}

/// Desktop's `FocusScope`s: the open thread pane takes the arrow keys until a click in the chat, a composer action
/// in the pane takes them back, and a jump to a reply only in a thread opens its pane.
#[test]
fn thread_pane_keys_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("thread-keys");
    let (chat, root) = ("!sl_infra:beeper.com", "msg_1007");
    app.select_chat(chat);
    let up = UiMessage::KeyPressed(Key::Named(Named::ArrowUp), Modifiers::empty());
    let _ = app.update(UiMessage::Thread(threads::Msg::Show(root.into())));
    let _ = app.update(up.clone());
    let thread = |app: &BeeperGuiApp| app.pane.thread.as_ref().unwrap().selected_msgs.clone();
    assert_eq!((thread(&app), app.pane.selected_msgs.clone()), (vec!["msg_1011".to_string()], vec![]));
    let _ = app.update(UiMessage::Message(message::Msg::Click("msg_1010".into())));
    let _ = app.update(up.clone());
    assert_eq!(app.pane.selected_msgs, ["msg_1007"], "a click in the chat takes the keys back");
    assert_eq!(thread(&app), ["msg_1011"]);
    let _ = app.update(UiMessage::InThread(Box::new(UiMessage::Composer(composer::Msg::Edit(
        text_editor::Action::SelectAll,
    )))));
    let _ = app.update(up);
    assert_eq!(thread(&app), ["msg_1010"], "a composer action in the pane takes the keys");
    let draft = app.pane.composer.text();
    let _ = app
        .update(UiMessage::InThread(Box::new(UiMessage::CommandBar(commands::Msg::JumpTo(chat.into(), root.into())))));
    assert_eq!(app.pane.composer.text(), draft, "the pane jumps in its thread and keeps the chat");
    assert_eq!(app.flash.as_ref().map(|f| f.0.as_str()), Some(root));
    let _ = app.update(UiMessage::Thread(threads::Msg::Hide));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat.into(), "msg_1011".into())));
    assert_eq!(app.pane.thread.as_ref().and_then(|t| t.thread_root.as_deref()), Some(root), "the reply opens its pane");
    assert_eq!(app.flash.as_ref().map(|f| f.0.as_str()), Some("msg_1011"));
}

/// Desktop's unread threads: the "Unread threads" pill and its menu, the Unread threads view with its cards, the badge
/// count, and the thread pane that reads a thread and keeps its unread divider.
#[test]
fn unread_threads_like_desktop() {
    let mut app = demo_app("unread-threads");
    let (chat, root) = ("!sl_infra:beeper.com", "msg_1007");
    let badge = app.badge_count();
    assert!(simulator(app.view_sidebar()).find("Unread threads").is_ok(), "the pill shows an unread thread");
    let _ = app.update(UiMessage::Thread(threads::Msg::UnreadMenu(true)));
    assert!(simulator(app.view_main()).find("Mark 1 Thread as Read").is_ok());
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::Thread(threads::Msg::ShowUnread));
    assert!(app.pane.selected_chat_id.is_none() && app.unread_view);
    {
        let mut sim = simulator(app.view_main());
        for t in ["Select a thread", "Ship it"] {
            assert!(sim.find(t).is_ok(), "the view shows {t}");
        }
    }
    let _ = app.update(UiMessage::Thread(threads::Msg::Open(chat.into(), root.into())));
    let thread = app.pane.thread.as_ref().expect("the card opens the thread pane");
    assert_eq!(thread.thread_divider.as_deref(), Some("0000000000001009"), "the divider stays at the old read mark");
    {
        let st = app.store.read();
        assert_eq!(st.thread_unread(st.chat(chat), root), (0, 0), "the open thread is read");
    }
    assert!(simulator(app.view_main()).find("You’re all caught up.").is_ok());
    assert!(simulator(app.view_sidebar()).find("Unread threads").is_err(), "the pill goes with the last unread thread");
    assert_eq!(app.badge_count(), badge - 1, "Desktop counts one more with unread threads");
    app.store.write().beeper_state.access_token.clear();
    assert_eq!(app.badge_count(), 0, "nothing while signed out");
}

/// The "Unread threads" pill counts only the threads of the chats in the list.
#[test]
fn unread_threads_pill_counts_the_listed_chats() {
    let mut app = demo_app("unread-threads-listed");
    let pill = |app: &BeeperGuiApp| simulator(app.view_sidebar()).find("Unread threads").is_ok();
    assert!(pill(&app), "the inbox lists the chat of the unread thread");
    let _ =
        app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag("!sl_infra:beeper.com".into(), ChatFlag::Archived)));
    assert!(!pill(&app), "the archived chat takes its thread out of the inbox");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Archived)));
    assert!(pill(&app), "the archive lists the chat of the unread thread");
}

/// Desktop's thread follows: "Unfollow Thread" in the message menu, which drops the thread from the unread threads,
/// a reply that follows the thread again, "Stop Following All Threads" in the chat menu, and the thread window.
#[test]
fn thread_follows_like_desktop() {
    let mut app = demo_app("thread-follows");
    let (chat, root) = ("!sl_infra:beeper.com", "msg_1007");
    app.select_chat(chat);
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(root.into())));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("Unfollow Thread").expect("a room of 3 members follows all threads");
    for m in sim.into_messages().collect::<Vec<_>>() {
        let _ = app.update(m);
    }
    let follows = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chat(chat).follows.clone()
    };
    assert_eq!(follows(&app).unfollowed, [root]);
    let st = app.store.read();
    assert!(st.unread_threads(0..st.chats.len()).is_empty(), "an unfollowed thread is not unread");
    drop(st);
    let _ = app.update(UiMessage::Thread(threads::Msg::Show(root.into())));
    let edit = text_editor::Edit::Paste(Arc::new("back".into()));
    let _ = app.update(UiMessage::InThread(Box::new(UiMessage::Composer(composer::Msg::Edit(
        text_editor::Action::Edit(edit),
    )))));
    let _ = app.update(UiMessage::InThread(Box::new(UiMessage::Composer(composer::Msg::Submit))));
    assert!(follows(&app).unfollowed.is_empty(), "a reply ends the unfollow like Desktop's refollowThread");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(chat.into())));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("Stop Following All Threads").expect("the chat menu of a room with threads");
    for m in sim.into_messages().collect::<Vec<_>>() {
        let _ = app.update(m);
    }
    assert_eq!(follows(&app).follow_all, Some(false));
    let _ = app.update(UiMessage::Thread(threads::Msg::OpenWindow(chat.into(), root.into())));
    let id = app.chat_windows.last().expect("a thread window").0;
    assert_eq!(app.title(id), "Thread in #desktop-release");
    let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::CloseModals)));
    let pane = &app.chat_windows.iter().find(|w| w.0 == id).expect("the window closes later").1;
    assert!(pane.selected_chat_id.is_none(), "Escape closes the thread window");
}

/// Desktop's `StickerPicker`: the empty text, the packs, an animated sticker that the chat does not take, and the
/// picked sticker that becomes an attachment, a recent sticker, and a sticker send.
#[test]
fn sticker_picker_like_desktop() {
    let mut app = demo_app("stickers");
    let chat = app.store.read().chats[0].id.clone();
    let webp = FileRule { mime_types: [("image/webp".to_string(), 2)].into(), ..FileRule::default() };
    app.store.write().chats[0].file.insert(FILE_STICKER.into(), webp);
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("stickerPacks", 1)));
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker)));
    let _ = app.update(UiMessage::Gifs(gifs::Msg::StickerPacks(Err("Matrix is not connected yet".into()))));
    assert!(simulator(app.view_main()).find("No sticker packs yet.").is_ok(), "a failed load and no recent stickers");
    let source = StickerSource { room_id: "!r:x".into(), state_key: "a".into(), shortcode: "cat".into() };
    let info = serde_json::json!({ "mimetype": "image/png", "w": 2, "h": 2 });
    let cat =
        Sticker { url: "mxc://x/cat".into(), shortcodes: vec!["cat".into()], body: None, info, source: Some(source) };
    let tgs = Sticker {
        url: "mxc://x/tgs".into(),
        info: serde_json::json!({ "mimetype": "application/x-tgsticker" }),
        ..cat.clone()
    };
    let pack = StickerPack {
        id: "m.room.image_pack|!r:x|a".into(),
        title: "Cats".into(),
        avatar: None,
        stickers: vec![cat.clone(), tgs.clone()],
    };
    let _ = app.update(UiMessage::Gifs(gifs::Msg::StickerPacks(Ok(vec![pack]))));
    assert!(simulator(app.view_main()).find("No sticker packs yet.").is_err());
    let _ = app.update(UiMessage::Gifs(gifs::Msg::PickSticker(tgs)));
    assert_eq!(app.status_banner, "This animated sticker can't be sent in this chat.");
    assert_eq!(app.pane.picker, None, "a pick closes the picker");
    let mut png = Vec::new();
    ::image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).unwrap();
    let frames = decode_media(png.clone()).unwrap();
    let sticker = Some(Box::new(cat.clone()));
    let f = PendingMedia {
        bytes: png,
        name: "cat.png".into(),
        mime: "image/png".into(),
        is_gif: false,
        size: (2, 2),
        frames,
        duration: None,
        sticker,
    };
    let _ = app.update(UiMessage::Gifs(gifs::Msg::StickerLoaded(Ok(f))));
    assert_eq!(app.pane.attachments.len(), 1);
    assert_eq!(app.store.read().settings.recent_stickers, std::slice::from_ref(&cat), "the pick is a recent sticker");
    online(&app);
    app.store.write().features.flags["features"]["*"]["undo-send"] = false.into();
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    let sent = st.outbox.iter().find_map(|(_, _, o)| match o {
        Outgoing::Media { sticker, name, .. } => Some((sticker.clone(), name.clone())),
        _ => None,
    });
    assert_eq!(sent, Some((Some(Box::new(cat)), "cat.png".into())), "the attachment goes as a sticker");
    let echo = st.messages.iter().rev().find(|m| m.chat_id == chat).unwrap();
    assert_eq!(echo.attachments[0].is_sticker, Some(true));
}

#[test]
fn pickers_check_the_network_like_desktop() {
    let mut app = demo_app("pickers");
    let mut st = app.store.write();
    let (wa, other) = (st.chats[0].id.clone(), st.chats[1].id.clone());
    st.chats[0].file.insert(FILE_STICKER.into(), FileRule::default());
    assert!(!st.chats[1].takes(FILE_STICKER), "the generic room features have no stickers");
    drop(st);
    let _ = app.update(UiMessage::SelectChat(wa));
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker)));
    assert_eq!((app.pane.picker, app.status_banner.as_str()), (None, ""), "the picker needs Use sticker packs");
    assert!(!hidden_actions(&app.store.read()).contains(&7));
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("stickerPacks", 1)));
    assert_eq!(hidden_actions(&app.store.read())[0], 7, "Add a sticker follows the emoji button");
    let cmds = app.commands(&app.store.read(), "");
    assert_eq!(cmds.iter().find(|c| c.title == "Send Sticker").and_then(|c| c.hotkey), Some("Message: Send Sticker"));
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker)));
    assert_eq!(app.pane.picker, Some(Picker::Sticker));
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker)));
    assert_eq!(app.pane.picker, None, "the hotkey closes the open picker");
    let _ = app.update(UiMessage::SelectChat(other.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Sticker)));
    assert_eq!(app.pane.picker, None);
    assert!(app.status_banner.starts_with("Stickers aren't supported on "), "{}", app.status_banner);
    let mut st = app.store.write();
    let i = st.resolve_chat_idx(&other).unwrap();
    st.chats[i].file.clear();
    drop(st);
    let _ = app.update(UiMessage::Composer(composer::Msg::TogglePicker(Picker::Gif)));
    assert_eq!(app.pane.picker, None);
    assert!(app.status_banner.starts_with("GIFs aren't supported on "), "{}", app.status_banner);
    let _ = app.update(UiMessage::Record(record::Msg::Start));
    assert!(app.recorder.is_none());
    assert!(app.status_banner.starts_with("Recording audio isn't supported on "), "{}", app.status_banner);
}

#[test]
fn unavailable_chats_hide_the_composer_like_desktop() {
    let mut app = demo_app("unavailable");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().take(2).map(|c| c.id.clone()).collect();
    let _ = app.update(UiMessage::SelectChat(ids[0].clone()));
    let set = |app: &BeeperGuiApp, read_only: bool, room: Option<&str>| {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&ids[0]).unwrap();
        st.update_chat(i, |c| (c.is_read_only, c.replacement_room) = (read_only, room.map(str::to_string)));
    };
    let composer = |sim: &mut Simulator<UiMessage>| sim.find(iced::widget::Id::new("composer-input")).is_ok();
    assert!(composer(&mut simulator(app.view_main())));
    set(&app, true, None);
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Messaging isn't available in this chat").is_ok() && !composer(&mut sim), "a read-only chat");
    set(&app, true, Some("!gone:beeper.com"));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Chat was upgraded.").is_ok() && !composer(&mut sim), "the new room is not in the list");
    set(&app, true, Some(&ids[1]));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Messaging isn't available in this chat").is_err() && !composer(&mut sim));
    let _ = sim.click("Click here to follow the conversation").expect("a link to the new room");
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    assert!(matches!(&msgs[..], [UiMessage::SelectChat(id)] if *id == ids[1]), "the link opens the new room");
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

/// Desktop's suggestion match takes only letters, digits and `+-_` after the trigger.
#[test]
fn emoticons_and_urls_suggest_nothing_like_desktop() {
    let mut app = demo_app("suggest-words");
    let picks = |app: &mut BeeperGuiApp, text: &str| {
        let _ = app.update(UiMessage::Composer(composer::Msg::Changed(text.into())));
        let st = app.store.read();
        app.suggestions(&st).map_or(0, |(_, rows)| rows.len())
    };
    assert!(picks(&mut app, "hi :smi") > 0, "a word suggests emoji");
    assert_eq!(picks(&mut app, "hi :)"), 0, "an emoticon waits for its space");
    assert_eq!(picks(&mut app, "see https://x"), 0);
    assert_eq!(picks(&mut app, "mail @a.b"), 0);
}

#[test]
fn mentions_like_desktop() {
    let mut app = demo_app("mentions");
    let chat = {
        let st = app.store.read();
        app.visible_chats(&st).into_iter().find(|c| c.participants.items.len() > 2).unwrap().clone()
    };
    let (id, ann) =
        (chat.id.clone(), chat.participants.items.iter().find(|u| u.is_self != Some(true)).unwrap().clone());
    let label = mention_label(&ann);
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("hi @".into())));
    let rows = |app: &BeeperGuiApp| {
        let st = app.store.read();
        let rows = app.suggestions(&st).map_or_else(Vec::new, |(_, rows)| rows);
        rows.into_iter()
            .map(|s| match s {
                Suggestion::Mention(label, id, _) => (label, id),
                Suggestion::Emoji(..) => panic!("an emoji row for @"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&app).len(), chat.participants.items.len().min(8), "an empty query lists the members");
    assert!(simulator(app.view_main()).find(format!("@{label}")).is_ok(), "the list shows the members");
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(format!("hi @{}", &label[..2]))));
    assert!(rows(&app).iter().all(|(_, id)| *id != ROOM_MENTION), "the chat cannot mention the room");
    let at = rows(&app).iter().position(|(_, id)| *id == ann.id).unwrap();
    let _ = app.update(UiMessage::Composer(composer::Msg::PickSuggestion(at)));
    assert_eq!(app.pane.composer.text(), format!("hi @{label} "));
    assert_eq!(app.pane.mentions, [(label.clone(), ann.id.clone())]);
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(format!("hi @{label} @ro"))));
    assert!(rows(&app).iter().all(|(_, id)| *id != ROOM_MENTION), "the chat cannot mention the room");
    {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(i, |c| c.can_mention_room = true);
    }
    assert_eq!(rows(&app)[0], ("room".to_string(), ROOM_MENTION.to_string()), "\"@room\" comes first");
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    assert_eq!(app.pane.composer.text(), format!("hi @{label} @room "), "Enter picks the row");
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    let sent = st.outbox.iter().find_map(|(_, _, o)| match o {
        Outgoing::Send { text, mentions, .. } => Some((text.clone(), mentions.clone())),
        _ => None,
    });
    let want = vec![(label.clone(), ann.id.clone()), ("room".to_string(), ROOM_MENTION.to_string())];
    assert_eq!(sent, Some((format!("hi @{label} @room"), want)), "the send has the pills");
    drop(st);
    // The draft keeps the pills, and an edit takes the pills of the message and gives the draft pills back.
    let pill = vec![(label.clone(), ann.id.clone())];
    app.pane.composer = composer_with(&format!("yo @{label}"));
    app.pane.mentions = pill.clone();
    let other = app.store.read().chats.iter().find(|c| c.id != id).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    assert_eq!(app.pane.mentions, pill, "the draft keeps the pills");
    let html = format!("<a href=\"https://matrix.to/#/{}\">@{label}</a> hi", ann.id);
    let mut st = app.store.write();
    let (_, own, _) = st.send_message(&id, Some(format!("@{label} hi")), None, None).unwrap();
    let i = st.message_idx(&own);
    st.update_message(i, |m| m.html = Some(html));
    drop(st);
    let zed = vec![("Zed".to_string(), "@z:x".to_string())];
    app.pane.mentions = zed.clone();
    let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(own, format!("@{label} hi"))));
    assert_eq!(app.pane.mentions, pill, "the edit has the pills of the message");
    let _ = app.update(UiMessage::Composer(composer::Msg::CancelMode));
    assert_eq!((app.pane.mentions.clone(), app.pane.composer.text()), (zed, format!("yo @{label}")));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("a :smile".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::PickSuggestion(0)));
    let text = app.pane.composer.text();
    assert!(text.starts_with("a ") && !text.contains(':') && text.ends_with(' '), "an emoji replaces its query");
    let _ = app.update(UiMessage::Composer(composer::Msg::PickSuggestion(0)));
    assert_eq!(app.pane.composer.text(), text, "the second click of a double click picks nothing");
}

#[test]
fn chat_commands_like_desktop() {
    let mut app = demo_app("chatcmds");
    let id = app.store.read().chats.iter().find(|c| profile_link(c).is_some()).map(|c| c.id.clone()).unwrap();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let cmds = app.commands(&app.store.read(), "");
    let titled = |t: &str| cmds.iter().find(|c| c.title == t).unwrap_or_else(|| panic!("no command {t}"));
    assert_eq!(cmds[0].title, "Remind me later");
    assert!(cmds.iter().all(|c| c.title != "Report spam" && c.title != "Block"), "the demo has no blocking");
    assert_eq!(titled("Send file").hotkey, Some("Chat: Send File"));
    assert_eq!(titled("Filter by account").category, "Filter Chats");
    let open = cmds.iter().find(|c| c.hotkey == Some("Open User Profile")).expect("a profile command");
    let network = network_name(&app.store.read().chats.iter().find(|c| c.id == id).unwrap().account_id);
    assert!(open.title.starts_with("Open @") && open.title.ends_with(&format!(" on {network}")), "{}", open.title);
    let _ = app.update(UiMessage::Commands(vec![UiMessage::CloseModals, cmds[0].msg.clone().unwrap()]));
    assert!(app.show_command_palette && app.cmd_submenu == Some(commands::SubMenu::Remind(id)));
}

#[test]
fn composer_actions_follow_the_chat_like_desktop() {
    let app = demo_app("actions");
    let st = app.store.read();
    let mut chat = app.visible_chats(&st)[0].clone();
    let net = network_name(&chat.account_id);
    let send = Some("Chat: Send File");
    assert_eq!(composer_action(&st, &chat, 3, false), ("Add Attachment".into(), send, true));
    assert_eq!(
        composer_action(&st, &chat, 3, true),
        ("Add Attachment (Unavailable while editing)".into(), None, false)
    );
    assert_eq!(composer_action(&st, &chat, 4, false), ("Send Later".into(), Some("Message: Send Later"), true));
    chat.file.clear();
    chat.polls = None;
    assert_eq!(composer_action(&st, &chat, 2, false), (format!("Add GIF (Unavailable for {net})"), None, false));
    assert!(!composer_action(&st, &chat, 5, false).2, "the chat takes no voice notes");
    assert_eq!(composer_action(&st, &chat, 6, false), (format!("Create Poll (Unavailable for {net})"), None, false));
    assert_eq!(composer_action(&st, &chat, 7, false), (format!("Add a sticker (Unavailable for {net})"), None, false));
}

/// Desktop's chat keys follow the list (`chatListItems`): the pinned tiles in their order, then the rows. Select
/// Next and Previous Chat stop at the ends.
#[test]
fn chat_keys_follow_the_list_order_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("list-order-keys");
    let list = app.list_ids(&app.store.read());
    let by_activity: Vec<String> = app.visible_chats(&app.store.read()).iter().map(|c| c.id.clone()).collect();
    assert_ne!(list[..9], by_activity[..9], "the pinned tiles are not the most recent chats");
    let key = |app: &mut BeeperGuiApp, k: &str| {
        let _ = app.update(UiMessage::KeyPressed(Key::Character(k.into()), Modifiers::CTRL));
        app.pane.selected_chat_id.clone().expect("a selected chat")
    };
    for (i, id) in list.iter().take(9).enumerate() {
        assert_eq!(&key(&mut app, &(i + 1).to_string()), id, "Ctrl {}", i + 1);
    }
    let _ = key(&mut app, "1");
    assert_eq!(key(&mut app, "]"), list[1], "Select Next Chat");
    let _ = key(&mut app, "[");
    assert_eq!(key(&mut app, "["), list[0], "no chat before the first");
    let last = list.last().expect("a chat").clone();
    let _ = app.update(UiMessage::SelectChat(last.clone()));
    assert_eq!(key(&mut app, "]"), last, "no chat after the last");
    let unread = app.listed_chats(&app.store.read()).iter().find(|c| c.is_unread()).map(|c| c.id.clone());
    assert_eq!(Some(key(&mut app, "u")), unread, "Select Next Unread Chat takes the first in the list");
}

#[test]
fn history_and_folder_submenus_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("historysub");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().take(3).map(|c| c.id.clone()).collect();
    for id in &ids {
        let _ = app.update(UiMessage::SelectChat(id.clone()));
    }
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::History)));
    let rows = app.commands(&app.store.read(), "");
    let chats: Vec<&str> = rows.iter().filter_map(|c| c.chat.as_deref()).collect();
    assert_eq!(chats[..3], [&*ids[2], &*ids[1], &*ids[0]], "the most recent chat first");
    assert_eq!((rows[0].hotkey, rows[1].hotkey), (None, Some("History: Next Chat")));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::empty()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(&*ids[1]));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Folders)));
    let titles: Vec<String> = app.commands(&app.store.read(), "").into_iter().map(|c| c.title).collect();
    assert_eq!(titles, ["Archived", "Low Priority", "Requests"], "the open folder is not listed");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("low".into())));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert_eq!(app.list.active_tab, InboxTab::LowPriority);
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

/// Desktop's `thread-history`: the opened chats, which the command bar shows as its recent chats, stay after a
/// relaunch.
#[test]
fn opened_chats_are_the_recent_chats_after_a_relaunch_like_desktop() {
    let mut app = demo_app("history-relaunch");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().rev().take(2).map(|c| c.id.clone()).collect();
    for id in &ids {
        let _ = app.update(UiMessage::SelectChat(id.clone()));
    }
    let _ = app.update(UiMessage::SelectChat(ids[0].clone()));
    let config = app.store.read().config.clone();
    let st = beeper_core::store::BeeperStore::from_config(config);
    assert_eq!(st.recent_chat_ids[..2], [&*ids[0], &*ids[1]], "the most recent first, once each");
    let mut app = BeeperGuiApp::new(beeper_core::store::StoreLock::shared(st));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::History)));
    let rows = app.commands(&app.store.read(), "");
    let chats: Vec<&str> = rows.iter().filter_map(|c| c.chat.as_deref()).collect();
    assert!(chats[1..].starts_with(&[&*ids[0], &*ids[1]]), "the History menu after the restart: {chats:?}");
}

#[test]
fn the_composer_pill_quotes_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("pill");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let long = "word ".repeat(40);
    let (_, own, _) = app.store.write().send_message(note, Some(long.clone()), None, None).unwrap();
    let _ = app.update(UiMessage::Composer(composer::Msg::StartReply(own.clone())));
    let quoted = long.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(125).collect::<String>() + "...";
    assert!(simulator(app.view_main()).find(quoted).is_ok(), "the quote shows 128 characters");
    let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(own, "old".into())));
    assert!(simulator(app.view_main()).find("Editing message").is_err(), "Desktop has no edit banner");
}

/// Desktop's Media window: the filter menu opens at the cursor, the selection bar counts with its plural string, and a
/// link row shows the preview image of the link.
#[test]
fn media_window_menu_and_rows_like_desktop() {
    let mut app = demo_app("media-rows");
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    let id = app.media_window.as_ref().map(|w| w.id).expect("a media window");
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(id, Size::new(1000.0, 720.0))));
    app.cursor.set(Point::new(900.0, 40.0));
    let _ = app.update(UiMessage::Media(media_view::Msg::Menu(true)));
    assert_eq!(app.media_window.as_ref().unwrap().menu, Some(Point::new(900.0, 40.0)), "the menu opens at the cursor");
    let _ = app.update(UiMessage::Media(media_view::Msg::Menu(false)));
    let (mid, k) = {
        let mut st = app.store.write();
        let (i, k) = image(&st);
        st.update_message(i, |m| m.attachments[k].file_size = Some(2048));
        (st.messages[i].id.clone(), k)
    };
    media_view::load_all_media(&mut app);
    let _ = app.update(UiMessage::Media(media_view::Msg::Select(true)));
    let _ = app.update(UiMessage::Media(media_view::Msg::Pick(mid, k)));
    let count = format!("1 selected ({})", human_size(2048));
    assert!(simulator(app.view(id)).find(count.as_str()).is_ok(), "the selection bar counts the items");
    let _ = app.update(UiMessage::Media(media_view::Msg::Select(false)));
    let img = "https://example.com/preview.png";
    {
        let mut st = app.store.write();
        let i = *st.timeline_indices("!note_self:beeper.com").last().expect("a note");
        let url = "https://example.com/page".into();
        st.update_message(i, |m| m.links = vec![MessageLink { url, img: Some(img.into()), ..Default::default() }]);
    }
    let _ = app.update(UiMessage::Media(media_view::Msg::Tab(1)));
    media_view::load_all_media(&mut app);
    drop(app.view(id));
    assert!(app.wants.media.borrow().contains(img), "the row loads the preview image");
}

/// The index of a loaded message with an image, and the index of the image.
fn image(st: &BeeperStore) -> (usize, usize) {
    let img = |a: &Attachment| a.attachment_type == "img";
    let i = st.messages.iter().position(|m| m.attachments.iter().any(img)).expect("an image");
    (i, st.messages[i].attachments.iter().position(img).expect("an image"))
}

/// Desktop's Media window viewer has Delete only when the chat deletes for everyone, you sent the message, and the
/// network time limit has not passed. The delete dialog shows over the viewer and Escape closes it first.
#[test]
fn media_viewer_deletes_like_desktop() {
    let mut app = demo_app("media-delete");
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    let id = app.media_window.as_ref().map(|w| w.id).expect("a media window");
    let (mid, ai, chat) = {
        let mut st = app.store.write();
        let (i, ai) = image(&st);
        st.update_message(i, |m| m.is_sender = Some(true));
        let c = st.chat_idx(&st.messages[i].chat_id);
        (st.chats[c].messages.delete, st.chats[c].messages.delete_max_age) = (true, None);
        (st.messages[i].id.clone(), ai, c)
    };
    media_view::load_all_media(&mut app);
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(id, Size::new(1000.0, 720.0))));
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), ai, true)));
    let click = |app: &BeeperGuiApp| -> Vec<UiMessage> {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view(id));
        sim.point_at(Point::new(898.0, 25.0));
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    };
    let delete = click(&app);
    assert!(
        matches!(&delete[..], [UiMessage::InWindow(w, m)] if *w == id && matches!(&**m, UiMessage::Media(media_view::Msg::AskDelete(v)) if v[..] == [(mid.clone(), ai)])),
        "Delete shows"
    );
    let _ = app.update(delete.into_iter().next().unwrap());
    assert!(simulator(app.view(id)).find("Are you sure you want to delete this message?").is_ok());
    let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), Modifiers::empty()));
    let w = app.media_window.as_ref().unwrap();
    assert!(w.deleting.is_none() && w.viewer.is_some(), "Escape closes the dialog first");
    app.store.write().chats[chat].messages.delete_max_age = Some(60);
    let go = click(&app);
    assert!(
        matches!(&go[..], [UiMessage::InWindow(_, m)] if matches!(**m, UiMessage::Message(message::Msg::GoTo(..)))),
        "an old message has no Delete"
    );
    let _ = app.update(UiMessage::Media(media_view::Msg::AskDelete(vec![(mid, ai)])));
    assert_eq!(app.status_banner, "These messages can’t be deleted");
}

/// Desktop's filmstrip of the Media window viewer scrolls with the wheel, and a step centers the current item again.
#[test]
fn media_filmstrip_scrolls_like_desktop() {
    let mut app = demo_app("media-strip");
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    let id = app.media_window.as_ref().map(|w| w.id).expect("a media window");
    let size = Size::new(300.0, 720.0);
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(id, size)));
    media_view::load_all_media(&mut app);
    let (mid, ai) = app.media_window.as_ref().map(|w| (w.items[0].0.id.clone(), w.items[0].1)).unwrap();
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, ai, true)));
    let wheel = {
        let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
        sim.point_at(Point::new(150.0, 700.0));
        let delta = mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 };
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled { delta })]);
        sim.into_messages().collect::<Vec<_>>()
    };
    assert!(
        matches!(&wheel[..], [UiMessage::InWindow(_, m)] if matches!(**m, UiMessage::Media(media_view::Msg::ScrollFilmstrip(1.0)))),
        "the wheel scrolls the filmstrip"
    );
    let viewer = |app: &mut BeeperGuiApp, m| drop(app.update(UiMessage::InWindow(id, Box::new(UiMessage::Media(m)))));
    let strip = |app: &BeeperGuiApp| app.media_window.as_ref().unwrap().viewer.as_ref().unwrap().strip;
    viewer(&mut app, media_view::Msg::ScrollFilmstrip(2.0));
    assert_eq!(strip(&app), Some(2));
    viewer(&mut app, media_view::Msg::CarouselStep(true));
    assert_eq!(strip(&app), None, "a step centers the current item");
}

#[test]
fn media_carousel_draws() {
    let mut app = demo_app("mediacarousel");
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    let id = app.media_window.as_ref().map(|w| w.id).expect("a media window");
    let (mid, ai, chat) = {
        let st = app.store.read();
        let (i, ai) = image(&st);
        (st.messages[i].id.clone(), ai, st.messages[i].chat_id.clone())
    };
    media_view::load_all_media(&mut app);
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), ai, true)));
    drop(app.view(id));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(chat.clone(), ChatFlag::LowPriority)));
    drop(app.view(id));
    let _ = app.update(UiMessage::StoreChanged);
    let viewer = app.media_window.as_ref().unwrap().viewer.is_some();
    assert!(viewer, "like Desktop's loaded items, the image stays when its chat goes low priority");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(chat.clone(), ChatFlag::LowPriority)));
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), ai, false)));
    for next in [false, true] {
        let steps = app.carousel_items(&app.store.read()).expect("the chat carousel shows the image").0.len();
        (0..=steps).for_each(|_| drop(app.update(UiMessage::Media(media_view::Msg::CarouselStep(next)))));
    }
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), ai, false)));
    app.store.write().delete_message(&chat, &mid).expect("a sync deletes the image");
    drop(simulator(app.view_main()));
    let _ = app.update(UiMessage::StoreChanged);
    assert!(app.pane.carousel.is_none(), "the carousel closes when a sync deletes its image");
}

/// Desktop's `GalleryGrouper`: photos of one sender, 60 seconds apart and 2 minutes from the first, show as one
/// gallery of up to 20 messages, with "+N" on the fourth tile.
#[test]
fn photos_group_into_galleries_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("gallery");
    let base = {
        let st = app.store.read();
        st.messages[*st.timeline_indices(note).last().unwrap()].clone()
    };
    let photo = |i: usize, secs: i64, text: Option<&str>| {
        let img = Attachment {
            attachment_type: if i == 1 { "video".into() } else { "img".into() },
            src_url: Some(format!("https://example.com/{i}.png")),
            file_name: Some(format!("{i}.png")),
            duration: Some(75.0),
            ..Default::default()
        };
        let timestamp = rfc3339_from_millis(1_800_000_000_000 + secs * 1000);
        let (id, sort_key) = (format!("$photo{i:02}"), format!("{:016}", 9_000_000 + i));
        Message { id, sort_key, timestamp, attachments: vec![img], text: text.map(Into::into), ..base.clone() }
    };
    {
        let mut st = app.store.write();
        for (i, secs) in [0, 50, 100, 110, 115, 120].into_iter().enumerate() {
            st.upsert_message(photo(i, secs, None));
        }
        st.upsert_message(photo(6, 125, None));
        st.upsert_message(photo(7, 126, Some("caption")));
        st.upsert_message(photo(8, 300, None));
    }
    let st = app.store.read();
    let msgs: Vec<&Message> = st.timeline(note).collect();
    let starts = gallery::starts(&st, &msgs, &vec![0; msgs.len()]);
    let at = |id: &str| msgs.iter().position(|m| m.id == id).unwrap();
    let first = at("$photo00");
    assert!((first..=at("$photo05")).all(|i| starts[i] == first), "photos up to 2 minutes from the first group");
    assert_eq!(starts[at("$photo06")], at("$photo06"), "a photo after 2 minutes starts a new gallery");
    assert_eq!(starts[at("$photo07")], at("$photo07"), "a photo with a caption is not in a gallery");
    drop(st);
    app.select_chat(note);
    let mut sim = simulator(app.view_main());
    assert!(sim.find("+2").is_ok(), "the fourth tile counts the two hidden photos");
    assert!(sim.find("01:15").is_ok(), "a video tile shows its duration");
}

/// Desktop's gallery selection and menu: a click selects the gallery, Ctrl and a tile select one photo, and a right
/// click selects the gallery and shows "Reactions", "Download All" and "Forward All…". A menu that closes without an
/// action unselects the gallery.
#[test]
fn galleries_select_and_have_a_menu_like_desktop() {
    let mut app = demo_app("gallery-menu");
    let (chat, ids) = {
        let st = app.store.read();
        let chat = st.messages.iter().find(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap();
        let chat = chat.chat_id.clone();
        let msgs: Vec<&Message> = st.timeline(&chat).collect();
        let starts = gallery::starts(&st, &msgs, &vec![0; msgs.len()]);
        let first = (1..msgs.len()).find(|&i| starts[i] != i).map(|i| starts[i]).expect("a demo gallery");
        let ids: Vec<String> =
            (first..msgs.len()).filter(|&i| starts[i] == first).map(|i| msgs[i].id.clone()).collect();
        (chat, ids)
    };
    {
        let mut st = app.store.write();
        let i = st.message_idx(&ids[1]);
        let key = "🎉".into();
        let reaction = Reaction {
            id: "r".into(),
            participant_id: "@x:beeper.com".into(),
            reaction_key: key,
            animated: None,
            img_url: None,
            sort_key: String::new(),
        };
        st.messages[i].reactions = vec![reaction];
    }
    let _ = app.update(UiMessage::SelectChat(chat));
    let _ = app.update(UiMessage::Gallery(gallery::Msg::Click(ids.clone())));
    assert_eq!(app.pane.selected_msgs, ids, "a click selects the gallery");
    app.mods = Modifiers::COMMAND;
    let _ = app.update(UiMessage::Gallery(gallery::Msg::Tile(ids[1].clone())));
    assert!(
        !app.pane.selected_msgs.contains(&ids[1]) && app.pane.carousel.is_none(),
        "Ctrl/Cmd and a tile unselect a photo"
    );
    app.mods = Modifiers::empty();
    let _ = app.update(UiMessage::Gallery(gallery::Msg::Tile(ids[1].clone())));
    assert!(app.pane.carousel.is_some(), "a tile opens the carousel");
    let _ = app.update(UiMessage::Media(media_view::Msg::CloseCarousel));
    let _ = app.update(UiMessage::Gallery(gallery::Msg::Menu(ids.clone())));
    assert_eq!(app.pane.selected_msgs, ids, "a right click selects the gallery");
    let mut sim = simulator(app.view_main());
    for label in ["Reactions", "Download All", "Forward All…"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    assert!(sim.find("Copy").is_err(), "the gallery menu is not the message menu");
    let _ = sim.click("Reactions").expect("Reactions");
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    let [UiMessage::Message(message::Msg::Menu(m))] = &msgs[..] else { panic!("one menu action") };
    assert!(matches!(&**m, UiMessage::Gallery(gallery::Msg::Reactions(r, _)) if *r == ids));
    let _ = app.update(msgs.into_iter().next().unwrap());
    assert!(app.pane.gallery_reactions.is_some() && app.pane.selected_msgs == ids, "the list shows");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::Gallery(gallery::Msg::Menu(ids.clone())));
    let _ = app.update(UiMessage::CloseModals);
    assert!(app.pane.selected_msgs.is_empty(), "a menu without an action unselects the gallery");
}

/// Desktop's `AttachmentFileRow`: the name in `body-sm` (16 px lines) and the details in `label-sm` (14 px lines).
#[test]
fn file_rows_like_desktop() {
    let mut app = demo_app("file-rows");
    let _ = app.update(UiMessage::SelectChat("!tg_rust:beeper.com".into()));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 900.0), app.view_main());
    let meta = format!("PDF · {}", human_size(128_500));
    assert_eq!(sim.find(meta.as_str()).expect("the file details").bounds().height, 14.0);
    assert_eq!(sim.find("beeper-v4-spec.pdf").expect("the file name").bounds().height, 16.0);
}

/// Desktop's carousel shows every attachment of the chat but audio: a video plays with its controls, and a file other
/// than a PDF is a link that downloads it. A step away from a playing video pauses it.
#[test]
fn carousel_steps_through_videos_and_files_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("carousel-items");
    let base = {
        let st = app.store.read();
        st.messages[*st.timeline_indices(note).last().unwrap()].clone()
    };
    let kinds = [("img", "a.png"), ("unknown", "spec.zip"), ("audio", "voice.ogg"), ("video", "clip.mp4")];
    {
        let mut st = app.store.write();
        for (i, (kind, name)) in kinds.into_iter().enumerate() {
            let a = Attachment {
                attachment_type: kind.into(),
                src_url: Some(format!("https://example.com/{name}")),
                file_name: Some(name.into()),
                ..Default::default()
            };
            let (id, sort_key) = (format!("$item{i}"), format!("{:016}", 9_000_000 + i));
            st.upsert_message(Message { id, sort_key, attachments: vec![a], text: None, ..base.clone() });
        }
    }
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel("$item0".into(), 0, false)));
    let _ = app.update(UiMessage::Media(media_view::Msg::CarouselStep(true)));
    let msgs: Vec<UiMessage> = {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 700.0), app.view_main());
        sim.point_at(Point::new(500.0, 375.0));
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    };
    let download = |m: &UiMessage| matches!(m, UiMessage::Message(message::Msg::Download(m, Some(0))) if m == "$item1");
    assert!(msgs.iter().any(download), "the link downloads the file");
    let _ = app.update(UiMessage::Media(media_view::Msg::CarouselStep(true)));
    assert_eq!(app.pane.carousel.as_ref().unwrap().mid, "$item3", "the carousel skips the audio");
    let _ =
        app.update(UiMessage::Video(video::Msg::Play("$item3".into(), "https://example.com/clip.mp4".into(), 2, 2)));
    drop(app.view_main());
    let _ = app.update(UiMessage::Media(media_view::Msg::CarouselStep(false)));
    assert!(!app.video.as_ref().unwrap().playing(), "a step away pauses the video");
}

/// Desktop's inline video: a click opens the carousel, where the played video goes on, and the carousel shows
/// Fullscreen in place of Rotate. When the carousel closes on the video, the inline video plays again.
#[test]
fn inline_videos_open_the_carousel_like_desktop() {
    let note = "!note_self:beeper.com";
    let clip = "https://example.com/clip.mp4";
    let mut app = demo_app("carousel-video");
    {
        let mut st = app.store.write();
        let base = st.messages[*st.timeline_indices(note).last().unwrap()].clone();
        let a = Attachment {
            attachment_type: "video".into(),
            src_url: Some(clip.into()),
            file_name: Some("clip.mp4".into()),
            size: Some(AttachmentSize { width: Some(320), height: Some(180) }),
            ..Default::default()
        };
        let sort_key = format!("{:016}", 9_000_000);
        st.upsert_message(Message { id: "$clip".into(), sort_key, attachments: vec![a], text: None, ..base });
    }
    let _ = app.update(UiMessage::SelectChat(note.into()));
    (app.pane.size, app.main_window) = (Size::new(1000.0, 700.0), Some(window::Id::unique()));
    let msgs: Vec<UiMessage> = {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 700.0), app.view_main());
        sim.point_at(Point::new(700.0, 480.0));
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    };
    let open = video::Msg::Open("$clip".into(), 0, 320, 180);
    assert!(
        msgs.iter().any(|m| matches!(m, UiMessage::Video(video::Msg::Open(m, 0, 320, 180)) if m == "$clip")),
        "a click on the video opens the carousel"
    );
    let _ = app.update(UiMessage::Video(open.clone()));
    assert_eq!(app.pane.carousel.as_ref().unwrap().mid, "$clip");
    let _ = app.update(UiMessage::Media(media_view::Msg::CloseCarousel));
    assert!(app.video.as_ref().is_some_and(|v| v.url == clip && v.playing()), "the inline video plays on close");
    let _ = app.update(UiMessage::Video(open));
    assert!(app.video.as_ref().unwrap().playing(), "the video goes on in the carousel");
    let key = |app: &mut BeeperGuiApp, k: Key| drop(app.update(UiMessage::KeyPressed(k, Modifiers::empty())));
    key(&mut app, Key::Character("f".into()));
    assert!(app.video.as_ref().unwrap().is_full(), "F fills the screen with the video");
    drop(app.view_main());
    key(&mut app, Key::Named(Named::Escape));
    assert!(
        !app.video.as_ref().unwrap().is_full() && app.pane.carousel.is_some(),
        "Escape leaves the full screen first"
    );
    let file = std::env::temp_dir().join(format!("beeper-rs-clip-{}.mp4", std::process::id()));
    let lavfi = ["-f", "lavfi", "-i", "testsrc=size=320x180:rate=30", "-t", "2", "-pix_fmt", "yuv420p", "-y"];
    assert!(std::process::Command::new("ffmpeg").args(lavfi).arg(&file).output().unwrap().status.success());
    let media = video::probe(clip, &std::fs::read(&file).unwrap()).map(std::sync::Arc::new);
    std::fs::remove_file(&file).unwrap();
    let _ = app.update(UiMessage::Video(video::Msg::Loaded(clip.into(), media)));
    let _ = app.update(UiMessage::Video(video::Msg::Toggle));
    // Desktop's carousel video controls: the skip buttons, the progress bar, the speed button and the time.
    let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 700.0), app.view_main());
    assert!(sim.find("00:00/00:02").is_ok(), "the time shows");
    sim.point_at(Point::new(888.0, 654.0));
    let _ = sim.simulate(iced_test::simulator::click());
    let speed = sim.into_messages().any(|m| matches!(m, UiMessage::Video(video::Msg::Rate(1.25))));
    assert!(speed, "the speed button goes to the next speed");
    // A video that cannot load shows Desktop's error card with a Download button in place of the video.
    let _ = app.update(UiMessage::Media(media_view::Msg::CloseCarousel));
    let _ = app.update(UiMessage::Video(video::Msg::Play("$clip".into(), clip.into(), 320, 180)));
    let _ = app.update(UiMessage::Video(video::Msg::Loaded(clip.into(), Err("bad file".into()))));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 700.0), app.view_main());
    assert!(sim.find("Something went wrong displaying video").is_ok(), "the error card shows");
    let _ = sim.click("Download").expect("the Download button");
    let download =
        sim.into_messages().any(|m| matches!(m, UiMessage::Message(message::Msg::Download(m, None)) if m == "$clip"));
    assert!(download, "the card downloads the video");
}

/// Desktop's carousel zoom: a double click or `+` zooms in, so that a click beside the image hits the image and does
/// not close the carousel, and `-` zooms out again.
#[test]
fn carousel_zooms_like_desktop() {
    let mut app = demo_app("carousel-zoom");
    let (chat, mid, url) = {
        let st = app.store.read();
        let m = st.messages.iter().find(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap();
        (m.chat_id.clone(), m.id.clone(), m.attachments[0].src_url.clone().unwrap())
    };
    let frames = vec![(image::Handle::from_rgba(200, 100, vec![255; 200 * 100 * 4]), 0)];
    app.media.insert(url, Some(Ok(frames)));
    let _ = app.update(UiMessage::SelectChat(chat));
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, 0, false)));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
    let (middle, beside) = (Point::new(500.0, 389.0), Point::new(650.0, 389.0));
    let click = |sim: &mut Simulator<UiMessage>, at: Point| {
        sim.point_at(at);
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
        let _ = sim.simulate(iced_test::simulator::click());
    };
    let settle = |sim: &mut Simulator<UiMessage>| {
        let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let _ = sim.simulate([iced::Event::Window(window::Event::RedrawRequested(later))]);
    };
    click(&mut sim, middle);
    click(&mut sim, middle);
    settle(&mut sim);
    click(&mut sim, Point::new(10.0, 600.0));
    click(&mut sim, beside);
    let _ = sim.tap_key(Key::Character("-".into()));
    settle(&mut sim);
    click(&mut sim, beside + Vector::new(0.0, 20.0));
    let _ = sim.tap_key(Key::Character("=".into()));
    settle(&mut sim);
    click(&mut sim, beside);
    // Only the click on the free space and the click beside the image at scale 1.22 close the carousel.
    let closes = sim.into_messages().filter(|m| matches!(m, UiMessage::Media(media_view::Msg::CloseCarousel))).count();
    assert_eq!(closes, 2, "the zoomed image covers the point beside it");
}

/// Desktop's carousel menu: Download and Copy, then Reply…, React… and Forward… in a chat, or Go to Message in the
/// Media window. Forward… closes the carousel first, and a click beside the menu or Escape closes only the menu.
#[test]
fn carousel_menu_like_desktop() {
    let mut app = demo_app("carousel-menu");
    let (chat, mid) = {
        let st = app.store.read();
        let m = st.messages.iter().find(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap();
        (m.chat_id.clone(), m.id.clone())
    };
    let _ = app.update(UiMessage::SelectChat(chat));
    app.pane.size = Size::new(1000.0, 720.0);
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), 0, false)));
    let at = Point::new(500.0, 389.0);
    let right = {
        let mut sim = Simulator::with_size(Default::default(), app.pane.size, app.view_main());
        sim.point_at(at);
        let _ = sim.simulate([mouse::Event::ButtonPressed(Button::Right)].map(iced::Event::Mouse));
        sim.into_messages().collect::<Vec<_>>()
    };
    assert!(
        matches!(&right[..], [UiMessage::Media(media_view::Msg::CarouselMenu(true))]),
        "a right click opens the menu"
    );
    app.cursor.set(at);
    let _ = app.update(UiMessage::Media(media_view::Msg::CarouselMenu(true)));
    let mut sim = Simulator::with_size(Default::default(), app.pane.size, app.view_main());
    for label in ["Download", "Copy", "Reply…", "React…", "Forward…"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    let _ = sim.click("Forward…").expect("Forward…");
    sim.point_at(Point::new(700.0, 600.0));
    let _ = sim.simulate(iced_test::simulator::click());
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    let forward = |m: &UiMessage| matches!(m, UiMessage::Commands(c) if matches!(&c[..], [UiMessage::Media(media_view::Msg::CarouselMenu(false)), UiMessage::Commands(c)] if matches!(&c[..], [UiMessage::Media(media_view::Msg::CloseCarousel), UiMessage::Forward(forward::Msg::Open(f))] if f[..] == [mid.clone()])));
    assert!(
        matches!(&msgs[..], [f, UiMessage::Media(media_view::Msg::CarouselMenu(false))] if forward(f)),
        "Forward… closes the carousel"
    );
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(app.pane.carousel.as_ref().is_some_and(|c| c.menu.is_none()), "Escape closes only the menu");
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    let id = app.media_window.as_ref().unwrap().id;
    media_view::load_all_media(&mut app);
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, 0, true)));
    let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::Media(media_view::Msg::CarouselMenu(true)))));
    let mut sim = simulator(app.view(id));
    assert!(sim.find("Go to Message").is_ok() && sim.find("Reply…").is_err(), "the Media window menu");
}

/// Desktop's chat carousel: React opens the emoji picker under its button, Reply closes the carousel and starts a reply,
/// and the reactions of the message show at the bottom.
#[test]
fn carousel_react_and_reply_like_desktop() {
    let mut app = demo_app("carousel-react");
    let (chat, mid) = {
        let mut st = app.store.write();
        let i = st.messages.iter().position(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap();
        let reaction = Reaction {
            id: "r".into(),
            participant_id: "@x:beeper.com".into(),
            reaction_key: "🎉".into(),
            animated: None,
            img_url: None,
            sort_key: String::new(),
        };
        st.update_message(i, |m| m.reactions = vec![reaction]);
        (st.messages[i].chat_id.clone(), st.messages[i].id.clone())
    };
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    app.pane.size = Size::new(1000.0, 720.0);
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), 0, false)));
    let click = |app: &BeeperGuiApp, at: Point| -> Vec<UiMessage> {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
        sim.point_at(at);
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    };
    let react = click(&app, Point::new(870.0, 25.0));
    assert!(
        matches!(&react[..], [UiMessage::Message(message::Msg::ReactAt(m, at))] if *m == mid && *at == Point::new(880.0, 39.0)),
        "React opens the picker under its button: {react:?}"
    );
    let reply = click(&app, Point::new(898.0, 25.0));
    assert!(
        matches!(&reply[..], [UiMessage::Commands(c)] if matches!(&c[..], [UiMessage::Media(media_view::Msg::CloseCarousel), UiMessage::Composer(composer::Msg::StartReply(m))] if *m == mid)),
        "Reply closes the carousel and starts a reply"
    );
    let toggle = {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
        let _ = sim.click("🎉").expect("the reaction shows");
        sim.into_messages().collect::<Vec<_>>()
    };
    assert!(
        matches!(&toggle[..], [UiMessage::Message(message::Msg::ToggleReaction(c, m, k))] if *c == chat && *m == mid && k == "🎉")
    );
    let _ = app.update(react.into_iter().next().unwrap());
    assert!(simulator(app.view_main()).find("Search").is_ok(), "the picker shows over the carousel");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(app.pane.emoji.is_none() && app.pane.carousel.is_some(), "Escape closes the picker first");
}

/// Desktop's carousel controls have tooltips with their keys.
#[test]
fn carousel_tooltips_like_desktop() {
    let mut app = demo_app("carousel-tips");
    let (chat, mid) = {
        let st = app.store.read();
        let m = st.messages.iter().find(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap();
        (m.chat_id.clone(), m.id.clone())
    };
    let _ = app.update(UiMessage::SelectChat(chat));
    let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, 0, false)));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
    let close = Point::new(982.0, 25.0);
    sim.point_at(close);
    let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: close })]);
    // The simulator does not find the text of overlays, so the test reads the pixels of the tooltip.
    let dir = std::env::temp_dir().join(format!("beeper-rs-tip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _ = sim.snapshot(&iced::Theme::Light).unwrap().matches_image(dir.join("tip"));
    let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let shot = ::image::open(&png).unwrap().into_rgba8();
    std::fs::remove_dir_all(&dir).unwrap();
    let k = shot.width() / 1000;
    assert_eq!(shot.get_pixel(887 * k, 45 * k).0, [255; 4], "the Close tooltip shows under the button");
}

/// Escape in a chat window leaves its chat and closes the window, which iced can draw again before it closes.
#[test]
fn chat_windows_that_left_their_chat_draw_nothing() {
    let mut app = demo_app("chat-window-left");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().take(2).map(|c| c.id.clone()).collect();
    let _ = app.update(UiMessage::SelectChat(ids[0].clone()));
    let _ = app.update(UiMessage::Window(windows::Msg::OpenChat(ids[1].clone())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    app.cmd_bar_commands_mode = true;
    let id = app.chat_windows[0].0;
    app.chat_windows[0].1.selected_chat_id = None;
    drop(simulator(app.view(id)));
}

/// The chats that a space window of rail space `entry` lists.
fn space_chats(app: &mut BeeperGuiApp, entry: &str) -> Vec<String> {
    let main = std::mem::replace(&mut app.list, chat_list::ListState::space(entry));
    let chats = app.visible_chats(&app.store.read()).iter().map(|c| c.id.clone()).collect();
    app.list = main;
    chats
}

/// Desktop's `openSpaceInNewWindow` from the menu of a space bar account: a 1200 by 800 window titled after the
/// space, with the chat list of the space and no space bar, on the chat of the main window when the space has it.
/// It handles its chats and menus apart from the main window, opens once, and does not open again at launch.
#[test]
fn space_windows_like_desktop() {
    let mut app = demo_app("space-window");
    let main = app.main_window.expect("a main window");
    let accounts = crate::rail_accounts(&app.store.read());
    let spaces = accounts.into_iter().map(|a| (space_chats(&mut app, &a), a));
    let (chats, acc) = spaces.max_by_key(|s| s.0.len()).expect("an account space");
    assert!(chats.len() > 1, "the space has chats");
    // Its chats show as rows with their titles, out of the pinned grid.
    for c in &chats {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(c).expect("a chat");
        st.set_chat_flag(idx, ChatFlag::Pinned, Some(false));
    }
    let _ = app.update(UiMessage::SelectChat(chats[1].clone()));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu(&acc));
        let _ = sim.click("Open in New Window").expect("an Open in New Window item");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(app.space_windows.len(), 1, "a space window opens");
    let (id, entry, list) = &app.space_windows[0];
    let id = *id;
    assert!(*entry == acc && list.locked && list.selected_account_filter.as_deref() == Some(acc.as_str()));
    let pane = &app.chat_windows.iter().find(|w| w.0 == id).expect("a pane for the space window").1;
    assert_eq!((pane.size, pane.selected_chat_id.as_deref()), (Size::new(1200.0, 800.0), Some(chats[1].as_str())));
    let name = crate::space_name(&app.store.read(), &acc);
    assert_eq!(app.title(id), name, "the window is titled after its space");
    assert!(app.list.selected_account_filter.is_none(), "the main window keeps its space");
    {
        let mut sim = simulator(app.view(id));
        assert!(sim.find(iced::widget::Id::new("rail-inbox")).is_err(), "a space window has no space bar");
        assert!(sim.find(iced::widget::Id::new("composer-input")).is_ok(), "and shows its chat");
        let _ = sim.click(name.as_str()).expect("the space name in the header");
        let menu = |m: &UiMessage| matches!(m, UiMessage::ChatList(chat_list::Msg::OpenMenu(_)));
        let menus = sim.into_messages().filter(|m| matches!(m, UiMessage::InWindow(_, m) if menu(m)));
        assert_eq!(menus.count(), 0, "its header opens neither the space sheet nor a menu");
    }
    assert!(simulator(app.view(main)).find(iced::widget::Id::new("rail-inbox")).is_ok(), "the main window has one");
    // A click on a chat of its list opens the chat in the space window only.
    let titles: Vec<(String, String)> = {
        let st = app.store.read();
        let others = st.chats.iter().filter(|c| c.id != chats[1] && chats.contains(&c.id));
        others.map(|c| (c.id.clone(), c.title.clone())).collect()
    };
    let (picked, msgs): (String, Vec<UiMessage>) = {
        let mut sim = simulator(app.view(id));
        let (chat, title) = titles.iter().find(|t| sim.find(t.1.as_str()).is_ok()).expect("a row of the space");
        let _ = sim.click(title.as_str()).expect("a chat of the space");
        (chat.clone(), sim.into_messages().collect())
    };
    for m in msgs {
        let _ = app.update(m);
    }
    let pane =
        |app: &BeeperGuiApp| app.chat_windows.iter().find(|w| w.0 == id).and_then(|w| w.1.selected_chat_id.clone());
    assert_eq!(pane(&app), Some(picked.clone()));
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(chats[1].as_str()), "the main window keeps its chat");
    // The chat menu of the space window shows there, and not in the main window.
    let _ =
        app.update(UiMessage::InWindow(id, Box::new(UiMessage::ChatList(chat_list::Msg::OpenMenu(picked.clone())))));
    assert!(app.list.chat_menu.is_none() && app.space_windows[0].2.chat_menu.is_some());
    assert!(
        simulator(app.view(id)).find("Mark as Unread").is_ok() || simulator(app.view(id)).find("Mark as Read").is_ok()
    );
    let main_menu = |app: &BeeperGuiApp| {
        let mut sim = simulator(app.view(main));
        sim.find("Mark as Unread").is_ok() || sim.find("Mark as Read").is_ok()
    };
    assert!(!main_menu(&app), "the main window shows no chat menu");
    let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::CloseModals)));
    // Desktop's `disableSpaceSwitching`: the account hotkeys keep the space window on its space.
    let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::RunHotkey("Select Next Account"))));
    assert_eq!(app.space_windows[0].2.selected_account_filter.as_deref(), Some(acc.as_str()));
    // It opens once, and Desktop's `zv` keeps no space window for the next launch.
    let _ = app.update(UiMessage::Window(windows::Msg::OpenSpace(acc.clone())));
    let _ = app.update(UiMessage::Window(windows::Msg::OpenChat(chats[1].clone())));
    assert_eq!((app.space_windows.len(), app.chat_windows.len()), (1, 2), "a chat window opens beside it");
    let path = app.store.read().config.data_dir.join("subwindow-state.json");
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let thread = format!("thread-{}", chats[1]);
    assert_eq!(
        saved["openSubwindows"],
        serde_json::json!([{"windowId": thread, "type": "thread", "threadId": chats[1]}])
    );
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
    assert!(app.space_windows.is_empty() && app.chat_windows.iter().all(|w| w.0 != id), "closing it forgets it");
}

/// A double click on a space of the space bar, the menus of its custom spaces and labels, and the command bar open a
/// space window like Desktop. A custom space opens on its first chat when it lacks the chat of the main window.
#[test]
fn space_windows_open_like_desktop() {
    let mut app = demo_app("space-window-open");
    let press = |app: &mut BeeperGuiApp, space: &str| {
        app.rail_hover = Some((space.into(), Point::new(18.0, 18.0)));
        let _ = app.update(UiMessage::Rail(rail::Msg::Press));
        let _ = app.update(UiMessage::Rail(rail::Msg::DragEnd));
    };
    press(&mut app, crate::LABEL_NUDGE);
    press(&mut app, crate::LABEL_NUDGE);
    press(&mut app, "archive");
    assert!(app.space_windows.is_empty(), "Create Label and one press open nothing");
    press(&mut app, "archive");
    assert_eq!(app.space_windows.len(), 1, "a double click opens the space");
    let (id, list) = (app.space_windows[0].0, &app.space_windows[0].2);
    assert!(list.active_tab == InboxTab::Archived && list.selected_account_filter.is_none());
    assert_eq!(app.title(id), InboxTab::Archived.label());
    let first = space_chats(&mut app, "archive").into_iter().next();
    assert_eq!(app.chat_windows.iter().find(|w| w.0 == id).unwrap().1.selected_chat_id, first);
    let has = |menu: El<'_>| simulator(menu).find("Open in New Window").is_ok();
    assert!(has(app.view_account_menu("inbox")) && has(app.view_account_menu("label:lbl_work")));
    assert!(!has(app.view_chat_menu(crate::RAIL_MENU, false, 900.0)), "the space bar background menu has none");
    let cmds = app.commands(&app.store.read(), "");
    let open = cmds.iter().find(|c| c.title == "Open space in new window").expect("a command bar entry");
    assert!(open.category == "Window" && open.keywords == ["pop out", "detach"]);
    let _ = app.update(open.msg.clone().expect("a command"));
    assert_eq!(app.space_windows.iter().map(|w| w.1.as_str()).collect::<Vec<_>>(), ["archive", "inbox"]);
}

#[test]
fn typing_focuses_the_composer_like_desktop() {
    let mut app = demo_app("typefocus");
    let main = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("ab".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Move(
        text_editor::Motion::DocumentStart,
    ))));
    let _ = app.update(UiMessage::WindowText(main, "c".into()));
    assert_eq!(app.pane.composer.text(), "abc", "a key outside a text field types at the end of the composer");
    app.show_command_palette = true;
    let _ = app.update(UiMessage::WindowText(main, "d".into()));
    assert_eq!(app.pane.composer.text(), "abc", "the command bar keeps the keys");
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
            m.links = vec![beeper_core::models::MessageLink { url, title: "Title".into(), ..Default::default() }];
            Some(m.id.clone())
        });
        linked.collect()
    };
    assert!(linked.len() > 1, "the demo has links");
    let _ = app.update(UiMessage::Message(message::Msg::Select(linked[..1].to_vec())));
    let cmds = app.commands(&app.store.read(), "");
    let titles: Vec<&str> = cmds.iter().take_while(|c| c.category == "Message").map(|c| c.title.as_str()).collect();
    let react = "Type any emoji to react with it";
    let all = ["Edit", "Delete…", "Forward", "Copy", "Copy with timestamps", "Open link", "Unselect"];
    assert_eq!(titles, [&[react][..], &all[..], &["Reply", "Send later"]].concat());
    assert!(matches!(&cmds[6].msg, Some(UiMessage::OpenUrl(u)) if u.starts_with("https://")));
    let _ = app.update(cmds[7].msg.clone().unwrap());
    assert!(app.pane.selected_msgs.is_empty(), "Unselect clears the selection");
    let _ = app.update(UiMessage::Message(message::Msg::Select(linked[..2].to_vec())));
    let cmds = app.commands(&app.store.read(), "");
    let titles: Vec<&str> = cmds.iter().take_while(|c| c.category == "Messages").map(|c| c.title.as_str()).collect();
    assert_eq!(titles, [&[react][..], &all[1..]].concat());
    let _ = app.update(UiMessage::Commands(vec![UiMessage::CloseModals, cmds[5].msg.clone().unwrap()]));
    let rows = app.commands(&app.store.read(), "");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.title.contains(" — https://")),
        "{:?}",
        rows.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
}

/// Desktop's `composeSwitchTargets`: with two accounts of a bridge, the chats with the person on both accounts, the
/// "Switch Chat" card, and a pick that opens the other chat with the draft.
#[test]
fn account_switcher_like_desktop() {
    let mut app = demo_app("switcher");
    let b = app.store.read().bridge_accounts.iter().find(|b| b.remote_profile.is_some()).cloned().unwrap();
    let them = |c: &Chat| c.participants.items.iter().any(|u| u.is_self != Some(true));
    let st = app.store.read();
    let chat = st.chats.iter().find(|c| c.account_id == b.account_id && c.chat_type == "single" && them(c)).cloned();
    let chat = chat.unwrap();
    drop(st);
    assert_eq!(switch_targets(&app.store.read(), &chat), None, "one account of the bridge");
    let two = beeper_core::models::BridgeAccountInfo { account_id: format!("{}-2", b.account_id), ..b.clone() };
    app.store.write().bridge_accounts.push(two.clone());
    assert_eq!(switch_targets(&app.store.read(), &chat), Some(vec![chat.id.clone()]), "no chat on the other account");
    let alt = Chat { id: "!alt:x".into(), account_id: two.account_id.clone(), draft: None, ..chat.clone() };
    app.store.write().upsert_chat(alt);
    let both = Some(vec![chat.id.clone(), "!alt:x".to_string()]);
    assert_eq!(switch_targets(&app.store.read(), &chat), both);
    let _ = app.update(UiMessage::SelectChat(chat.id.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("hi".into())));
    let _ = app.update(UiMessage::SwitchCard);
    assert!(simulator(app.view_main()).find("Switch Chat").is_ok());
    let _ = app.update(UiMessage::SwitchChat("!alt:x".into()));
    assert_eq!((app.pane.selected_chat_id.as_deref(), app.list.chat_menu.is_none()), (Some("!alt:x"), true));
    assert_eq!(app.pane.composer.text().trim(), "hi", "the draft goes to the chat without a draft");
    let st = app.store.read();
    let old = st.chats[st.resolve_chat_idx(&chat.id).unwrap()].draft.clone().unwrap_or_default();
    assert!(old.text.unwrap_or_default().is_empty(), "the chat keeps no draft");
}

#[test]
fn composer_placeholder_like_desktop() {
    let app = demo_app("placeholder");
    let st = app.store.read();
    let mut chat = app.visible_chats(&st)[0].clone();
    let net = network_name(&chat.account_id);
    chat.title = "x".repeat(130);
    let place = |chat: &Chat| app.composer_placeholder(&st, chat);
    assert_eq!(place(&chat), format!("Message {}... on {net}", "x".repeat(125)), "128 characters");
    (chat.title, chat.is_request, chat.chat_type) = ("Ann".into(), Some(true), "single".into());
    assert_eq!(place(&chat), format!("Message Ann on {net} to accept the request"));
    chat.chat_type = "group".into();
    assert_eq!(place(&chat), format!("Message Ann on {net} to join the group"));
    // Desktop's `bridgeProtocolDisplayName`: a second account of the bridge adds the user of the account.
    drop(st);
    let b = app.store.read().bridge_accounts.iter().find(|b| b.remote_profile.is_some()).cloned().unwrap();
    let two = beeper_core::models::BridgeAccountInfo { account_id: format!("{}-2", b.account_id), ..b.clone() };
    app.store.write().bridge_accounts.push(two);
    let st = app.store.read();
    (chat.account_id, chat.is_request) = (b.account_id.clone(), None);
    let user = account_user(&st, &b.account_id);
    assert!(!user.is_empty());
    let want = format!("Message Ann on {} ({user})", network_name(&b.account_id));
    assert_eq!(app.composer_placeholder(&st, &chat), want);
}

#[test]
fn emoticons_follow_the_setting() {
    use iced_test::simulator::typewrite;
    let mut app = demo_app("emoticons");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    press(&mut app, true, typewrite("hi :) "));
    assert_eq!(text(&app), "hi 🙂 ");
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    press(&mut app, true, typewrite("hi :) ").chain([key(Named::Backspace, Modifiers::empty())]));
    assert_eq!(text(&app), "hi :) ", "Backspace undoes the replacement");
    press(&mut app, true, [key(Named::Backspace, Modifiers::empty())]);
    assert_eq!(text(&app), "hi :)");
    press(&mut app, true, [key(Named::Home, Modifiers::empty())].into_iter().chain(typewrite(";) ")));
    assert_eq!(text(&app), "😉 hi :)", "the replacement is at the caret");
    press(&mut app, true, typewrite("x").chain([key(Named::Backspace, Modifiers::empty())]));
    assert_eq!(text(&app), "😉 hi :)", "an other edit ends the undo");
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("`a".into())));
    press(&mut app, true, typewrite(" :) "));
    assert_eq!(text(&app), "`a :) ", "no replacement in code");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("keepEmoticons", 1)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    press(&mut app, true, typewrite(":D "));
    assert_eq!(text(&app), ":D ", "the setting keeps emoticons");
}

#[test]
fn typing_notices_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("typing");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let typing = |app: &BeeperGuiApp| app.pane.typing.as_ref().map(|t| t.0.clone());
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("a".into())));
    assert_eq!(typing(&app), Some(note.to_string()), "a key starts the notice");
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    assert_eq!(typing(&app), None, "a send ends the notice");
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Set(Some((now_unix_millis() + 3_600_000, false)))));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("later".into())));
    assert_eq!(typing(&app), None, "a Send Later message has no notice");
    let _ = app.update(UiMessage::Scheduled(scheduled::Msg::Set(None)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("b".into())));
    let _ = app.update(UiMessage::TypingTick);
    assert_eq!(typing(&app), Some(note.to_string()), "the notice stays for 3 seconds");
    app.pane.typing.as_mut().unwrap().1 -= TYPING_STOP;
    let _ = app.update(UiMessage::TypingTick);
    assert_eq!(typing(&app), None, "the notice ends 3 seconds after the last key");
}

#[test]
fn composer_undo_groups_like_prosemirror() {
    let mut app = demo_app("undo");
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(String::new())));
    let text = |app: &BeeperGuiApp| app.pane.composer.text();
    let paste = |app: &mut BeeperGuiApp, s: &str| {
        let edit = text_editor::Edit::Paste(Arc::new(s.into()));
        let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit))));
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
    let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Move(
        text_editor::Motion::DocumentStart,
    ))));
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
        st.timeline_indices(chat).iter().map(|&i| st.messages[i].id.clone()).collect()
    };
    assert!(ids.len() >= 3, "the demo chat has messages");
    let mid = ids[ids.len() - 2].clone();
    let _ = app.update(UiMessage::Message(message::Msg::Click(mid.clone())));
    {
        let mut st = app.store.write();
        let i = st.message_idx(&mid);
        st.messages[i].is_deleted = Some(true);
    }
    let _ = app.update(UiMessage::Message(message::Msg::Step(true, false)));
    assert_eq!(app.pane.selected_msgs.last(), ids.last());
    let _ = app.update(UiMessage::Message(message::Msg::Click(mid)));
    let _ = app.update(UiMessage::Message(message::Msg::Step(false, false)));
    assert_eq!(app.pane.selected_msgs, [ids[ids.len() - 3].clone()]);
}

#[test]
fn forward_selected_messages_like_desktop() {
    let mut app = demo_app("fwdmany");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let texts = |app: &BeeperGuiApp, cid: &str| -> Vec<Option<String>> {
        let st = app.store.read();
        st.timeline_indices(cid).iter().map(|&i| st.messages[i].text.clone()).collect()
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
    let _ = app.update(UiMessage::Forward(forward::Msg::Pick(target.clone())));
    let _ = app.update(UiMessage::Forward(forward::Msg::Send));
    let _ = app.update(UiMessage::Forward(forward::Msg::Send));
    let after = texts(&app, &target);
    assert_eq!(after[before..], source[n - 2..], "the messages arrive in timeline order, once for a double click");
}

/// Desktop's `virtua` lists: the chat list and the timeline build only the rows near the viewport, and build the
/// other rows when they scroll into view or when a jump needs them.
#[test]
fn long_lists_build_only_the_rows_near_the_viewport() {
    let mut app = demo_app("virtual-lists");
    let (chat, msg) = {
        let st = app.store.read();
        let has_msgs = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
        let c = st.chats.iter().filter(|c| c.flags & CHAT_PINNED == 0).find(has_msgs).expect("a chat").clone();
        (c.clone(), st.messages[st.timeline_indices(&c.id)[0]].clone())
    };
    {
        let mut st = app.store.write();
        for i in 0..150 {
            let at = format!("2001-01-01T00:{:02}:{:02}.000Z", i / 60, i % 60);
            let title = format!("Many {i:03}");
            let id = format!("!many{i}:beeper.local");
            st.upsert_chat(Chat { id, title, network_name: None, last_activity: Some(at.clone()), ..chat.clone() });
            let text = Some(format!("Line {i:03}"));
            let sort_key = format!("{:016}", 1000 + i);
            st.upsert_message(Message { id: format!("$many{i:03}"), text, sort_key, timestamp: at, ..msg.clone() });
        }
    }
    fn view(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
        let id = app.main_window.expect("a main window");
        Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id))
    }
    let mut sim = view(&app);
    assert!(sim.find("Many 149").is_ok() && sim.find("Many 000").is_err(), "the list builds the top rows only");
    sim.point_at(Point::new(200.0, 500.0));
    let wheel = mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: 0.0, y: -100_000.0 } };
    let redraw = iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()));
    let _ = sim.simulate([iced::Event::Mouse(wheel), redraw]);
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    assert!(msgs.iter().any(|m| matches!(m, UiMessage::Relist)), "a scroll past the built rows asks for others");
    for m in msgs {
        let _ = app.update(m);
    }
    let mut sim = view(&app);
    assert!(sim.find("Many 000").is_ok() && sim.find("Many 149").is_err(), "the list builds the bottom rows");
    drop(sim);
    app.select_chat(&chat.id);
    let first = iced::widget::Id::new("msg-$many000");
    assert!(view(&app).find(first.clone()).is_err(), "the timeline builds the newest messages only");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), "$many000".into())));
    assert!(view(&app).find(first).is_ok(), "a jump builds the message that it shows");
}

/// Desktop loads the images of the rows that its lists render: a view asks for the media that it shows, and the next
/// frame starts the loads, so an update does not scan every message of the chat.
#[test]
fn views_load_only_the_media_that_they_show() {
    let mut app = demo_app("lazy-media");
    let (chat, msg) = {
        let st = app.store.read();
        let has_msgs = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
        let c = st.chats.iter().filter(|c| c.flags & CHAT_PINNED == 0).find(has_msgs).expect("a chat").clone();
        (c.clone(), st.messages[st.timeline_indices(&c.id)[0]].clone())
    };
    let url = "https://example.com/old.png";
    {
        let mut st = app.store.write();
        let img = Attachment {
            attachment_type: "img".into(),
            src_url: Some(url.into()),
            file_name: Some("old.png".into()),
            ..Default::default()
        };
        let at = "2001-01-01T00:00:00.000Z".to_string();
        let old = Message {
            id: "$old".into(),
            text: None,
            attachments: vec![img],
            sort_key: format!("{:016}", 500),
            ..msg.clone()
        };
        st.upsert_message(Message { timestamp: at.clone(), ..old });
        for i in 0..100 {
            let (id, text, sort_key) =
                (format!("$text{i:03}"), Some(format!("Line {i:03}")), format!("{:016}", 1000 + i));
            st.upsert_message(Message { id, text, sort_key, timestamp: at.clone(), ..msg.clone() });
        }
    }
    app.select_chat(&chat.id);
    let _ = app.update(UiMessage::Noop);
    assert!(!app.media.contains_key(url), "an update does not load the media of a message that no view shows");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), "$old".into())));
    let id = app.main_window.expect("a main window");
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id));
    let _ = sim.simulate([iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    assert!(msgs.iter().any(|m| matches!(m, UiMessage::LoadWanted)), "the frame asks to load the shown media");
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.media.contains_key(url), "the shown media loads");
}

/// A hover and a frame do not filter and sort the chats again: the views keep the chat list and the counts until
/// the store changes.
#[test]
fn views_keep_the_chat_list_until_the_store_changes() {
    let mut app = demo_app("memo");
    let id = app.main_window.expect("a main window");
    let list = |app: &BeeperGuiApp| {
        drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id)));
        app.memo.lists.borrow().values().next().cloned().expect("the view sorted the chat list")
    };
    let first = list(&app);
    let generation = app.store.generation().expect("no write lock");
    let st = app.store.read();
    let ids: Vec<String> =
        app.visible_chats(&st).iter().filter(|c| c.flags & CHAT_PINNED == 0).take(4).map(|c| c.id.clone()).collect();
    drop(st);
    for chat in &ids {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::Hover(chat.clone(), true)));
        let _ = app.update(UiMessage::Relist);
        assert!(Rc::ptr_eq(&first, &list(&app)), "a hover keeps the chat list");
    }
    assert_eq!(app.store.generation(), Some(generation), "a hover does not write to the store");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(ids[0].clone(), ChatFlag::Archived)));
    let after = list(&app);
    assert!(!Rc::ptr_eq(&first, &after), "a change of the store sorts the chat list again");
    assert!(!app.visible_chats(&app.store.read()).iter().any(|c| c.id == ids[0]), "the archived chat leaves");
}

/// The rows of the open chat read all its messages once for each store generation: a hover keeps them, and a new
/// message makes them again.
#[test]
fn timeline_rows_stay_until_the_store_changes() {
    let mut app = demo_app("rows-memo");
    let id = app.main_window.expect("a main window");
    let rows = |app: &BeeperGuiApp| {
        drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id)));
        app.pane.rows.borrow().as_ref().expect("the view made the rows").1.clone()
    };
    let first = rows(&app);
    assert!(!first.msgs.is_empty(), "the note chat has messages");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Hover(NOTE_CHAT_ID.into(), true)));
    assert!(Rc::ptr_eq(&first, &rows(&app)), "a hover keeps the rows");
    let _ = app.store.write().send_message(NOTE_CHAT_ID, Some("new".into()), None, None).unwrap();
    let next = rows(&app);
    assert_eq!(next.msgs.len(), first.msgs.len() + 1, "a new message makes the rows again");
}

/// The chat list reads the unread threads from SQLite once for each store generation.
#[test]
fn unread_threads_stay_until_the_store_changes() {
    let mut app = demo_app("threads-memo");
    let id = app.main_window.expect("a main window");
    let threads = |app: &BeeperGuiApp| {
        drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id)));
        app.memo.threads.borrow().clone().expect("the chat list read the unread threads")
    };
    let first = threads(&app);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Hover(NOTE_CHAT_ID.into(), true)));
    assert!(Rc::ptr_eq(&first, &threads(&app)), "a hover keeps the unread threads");
}

/// The command bar searches once for each query: the other events keep its finds until the store changes, and the
/// finds are the newest messages first.
#[test]
fn command_bar_searches_once_for_each_query() {
    let mut app = demo_app("search-memo");
    let id = app.main_window.expect("a main window");
    let hits = |app: &BeeperGuiApp| {
        drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id)));
        app.memo.hits.borrow().clone().expect("the view searched").1
    };
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("rust".into())));
    let first = hits(&app);
    assert!(first.messages.len() > 1, "the demo has messages with the word");
    let _ = app.update(UiMessage::Relist);
    assert!(Rc::ptr_eq(&first, &hits(&app)), "a frame keeps the finds");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("rus".into())));
    assert!(!Rc::ptr_eq(&first, &hits(&app)), "a key searches again");
    assert!(first.messages.windows(2).all(|w| w[0].sort_key > w[1].sort_key), "newest first");
}

/// The command bar's search view, tall enough to lay out all of its results.
fn search_view(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
    Simulator::with_size(Default::default(), Size::new(1440.0, 2000.0), app.view_command_palette())
}

/// Whether the search view shows each of `texts`, each below the one before.
fn in_order(app: &BeeperGuiApp, texts: &[&str]) -> bool {
    let mut sim = search_view(app);
    let ys: Vec<f32> = texts.iter().map(|t| sim.find(*t).unwrap_or_else(|_| panic!("{t} shows")).bounds().y).collect();
    ys.windows(2).all(|w| w[0] < w[1])
}

/// Desktop's `Jqt` sections of a search in order, with the `J5` limits: the chats by title, the chats by member, the
/// bookmarks, three until "Show more", the media, the 10 newest images and videos without a media filter, and the
/// messages.
#[test]
fn search_sections_like_desktop() {
    let mut app = demo_app("search-sections");
    {
        let mut st = app.store.write();
        let i = st.find_message("msg_1000a").expect("a message of Eric");
        let base = st.messages[i].clone();
        for i in 0..12 {
            let img =
                Attachment { attachment_type: "img".into(), file_name: Some("a.png".into()), ..Default::default() };
            let (id, sort_key) = (format!("$zebra{i}"), format!("{:016}", 9_000_000 + i));
            let text = Some(format!("zebra snapshot {i}"));
            st.upsert_message(Message { id, sort_key, text, attachments: vec![img], ..base.clone() });
        }
        for id in ["msg_1000a", "msg_1002", "msg_1006"] {
            assert!(st.toggle_bookmark(id), "the demo bookmarks msg_1001 only");
        }
    }
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let hits = |app: &BeeperGuiApp, q: &str| app.search_hits(&app.store.read(), q);
    let title = |app: &BeeperGuiApp, i: usize| app.store.read().chats[i].title.clone();
    let eric = hits(&app, "eric");
    let groups: Vec<String> = eric.in_groups.iter().map(|&i| title(&app, i)).collect();
    assert_eq!(eric.chats.iter().map(|&i| title(&app, i)).collect::<Vec<_>>(), ["Eric Migicovsky"]);
    assert!(groups.contains(&"#desktop-release".to_string()), "a group with Eric: {groups:?}");
    // Desktop's command bar index: the words in any order, and the network of a chat.
    let titles = |q: &str| hits(&app, q).chats.iter().map(|&i| title(&app, i)).collect::<Vec<_>>();
    assert_eq!(titles("rust beeper"), ["Beeper Rust Core Team"]);
    let whatsapp = titles("whatsapp");
    assert!(whatsapp.contains(&"Eric Migicovsky".to_string()), "a chat by its network: {whatsapp:?}");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("eric".into())));
    assert!(in_order(&app, &["Chats", "In Groups", "#desktop-release"]));
    let beeper = hits(&app, "beeper");
    assert_eq!(beeper.bookmarks.len(), 4);
    assert!(beeper.bookmarks.iter().all(|m| m.is_bookmarked == Some(true)));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("beeper".into())));
    assert!(in_order(&app, &["Bookmarks", "Messages"]));
    let msgs: Vec<UiMessage> = {
        let mut sim = search_view(&app);
        let _ = sim.click("Show more").expect("four bookmarks show three");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(search_view(&app).find("Show less").is_ok(), "Show more shows all bookmarks");
    let zebra = hits(&app, "zebra");
    assert_eq!(zebra.media.len(), 10, "Desktop shows 10 media without a media filter");
    assert!(zebra.media.windows(2).all(|w| w[0].sort_key > w[1].sort_key), "newest first");
    assert!(zebra.messages.len() == 12 && zebra.bookmarks.is_empty());
    let _ = app.update(UiMessage::CommandBar(commands::Msg::PickSearchFilter(SearchPick::Media(0))));
    assert_eq!(hits(&app, "zebra").media.len(), 12, "a media filter shows all of its media");
    assert!(hits(&app, "zebra").messages.is_empty(), "and no text messages");
    assert!(hits(&app, "z").media.len() > 12, "a one-letter query does not search the text");
    assert!(hits(&app, "eric").chats.is_empty(), "a media filter shows no chats");
}

/// Desktop's `threadSearcher` also finds the chats that All Chats hides, after the others by `computeStatePenalty`:
/// low priority adds 120 and a chat hidden from the inbox (snoozed, or hidden by a label or by the "Show in Inbox" of
/// its account) adds 80. A member of a merged chat is `isHidden` and does not show.
#[test]
fn search_shows_hidden_chats_lower_like_desktop() {
    let mut app = demo_app("search-hidden");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    // Six chats named Zulu, newest first.
    let zulu: Vec<String> = {
        let mut st = app.store.write();
        let mut chats: Vec<&Chat> = st.chats.iter().filter(|c| c.listed() && c.flags == 0).collect();
        chats.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        let ids: Vec<String> = chats.iter().take(6).map(|c| c.id.clone()).collect();
        assert_eq!(ids.len(), 6, "the demo has six plain chats");
        for (n, id) in ids.iter().enumerate() {
            let idx = st.chat_idx(id);
            st.update_chat(idx, |c| c.title = format!("Zulu {n}"));
        }
        ids
    };
    let (low, label, account, snoozed, merged) = (&zulu[0], &zulu[1], &zulu[2], &zulu[3], &zulu[4]);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(low.clone(), ChatFlag::LowPriority)));
    let _ = app.update(UiMessage::Labels(labels::Msg::Switch("lbl_work".into(), label.clone(), true)));
    let _ = app.update(UiMessage::Labels(labels::Msg::SetInbox("lbl_work".into(), Some("none"))));
    {
        let mut st = app.store.write();
        st.features.internal = true;
        st.features.flags["features"]["*"]["snooze"] = serde_json::json!(true);
        let (a, s, m) = (st.chat_idx(account), st.chat_idx(snoozed), st.chat_idx(merged));
        st.update_chat(a, |c| c.is_hidden_from_inbox = Some(true));
        st.update_chat(s, |c| c.snoozed_until_ms = Some(now_unix_millis() + 3_600_000));
        st.update_chat(m, |c| c.merged_into_chat_id = Some(low.clone()));
    }
    let st = app.store.read();
    for id in [label, account] {
        assert_ne!(st.find_chat(id).unwrap().flags & CHAT_HIDDEN, 0, "All Chats hides {id}");
    }
    let found: Vec<&str> = app.search_hits(&st, "zulu").chats.iter().map(|&i| st.chats[i].title.as_str()).collect();
    let order = ["Zulu 5", "Zulu 1", "Zulu 2", "Zulu 3", "Zulu 0"];
    assert_eq!(found, order, "the shown chat, the hidden ones, then low priority, and no merged member");
}

/// Desktop's `Hct` operators in the query and in the filters, and its partial matches toggle.
#[test]
fn search_operators_and_partial_matches_like_desktop() {
    let mut app = demo_app("search-operators");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let ids = |app: &BeeperGuiApp, q: &str| -> HashSet<String> {
        app.search_hits(&app.store.read(), q).messages.iter().map(|m| m.id.clone()).collect()
    };
    let set = |ids: &[&str]| -> HashSet<String> { ids.iter().map(|s| s.to_string()).collect() };
    assert!(ids(&app, "beeper").len() > 2);
    assert_eq!(ids(&app, "beeper from:me"), set(&["msg_1000b", "msg_note_4"]));
    assert_eq!(ids(&app, "voice has:audio"), set(&["msg_1000c", "msg_1003"]));
    assert_eq!(ids(&app, "voice has:file"), set(&["msg_1003"]));
    assert_eq!(ids(&app, "voice in:dm"), set(&["msg_1000c"]));
    assert_eq!(ids(&app, "voice network:acc_telegram"), set(&["msg_1003"]));
    assert_eq!(ids(&app, "from:me").len(), 7, "only operators lists the messages that match them");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::PickSearchFilter(SearchPick::Sent(true))));
    assert_eq!(ids(&app, "beeper"), set(&["msg_1000b", "msg_note_4"]), "a filter searches like its operator");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::PickSearchFilter(SearchPick::Sent(true))));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("rus".into())));
    assert!(!ids(&app, "rus").is_empty(), "a partial match");
    assert!(search_view(&app).find("Hide partial matches").is_ok());
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePartialMatches));
    assert!(ids(&app, "rus").is_empty() && !ids(&app, "rust").is_empty(), "whole words only");
    assert!(search_view(&app).find("Show partial matches").is_ok(), "the heading stays with the chats it found");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("zzqq".into())));
    assert!(in_order(&app, &["No messages matching those terms", "Show partial matches"]));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    assert!(app.search_partial, "a new search shows partial matches");
}

/// Desktop's empty search: the recent chats and the chat that was archived last, and no recent searches, which
/// Desktop 4.3 keeps but `Jqt` leaves out of the general search.
#[test]
fn an_empty_search_shows_the_last_archived_chat_like_desktop() {
    let mut app = demo_app("search-archived");
    let old = {
        let st = app.store.read();
        st.chats.iter().filter(|c| c.flags & CHAT_HIDDEN == 0).min_by_key(|c| c.last_activity.clone()).unwrap().clone()
    };
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(old.id.clone(), ChatFlag::Archived)));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    assert!(in_order(&app, &["Recent chats", "Last archived", &old.title]));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("rust".into())));
    assert!(search_view(&app).find("Last archived").is_err(), "a query hides it");
    let _ = app.update(UiMessage::CommandBar(commands::Msg::PickSearchFilter(SearchPick::Sent(true))));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query(String::new())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    assert!(search_view(&app).find("Recent searches").is_err());
    let _ = app.update(UiMessage::SelectChat(old.id.clone()));
    assert!(search_view(&app).find("Last archived").is_err(), "the open chat is not a result");
}

/// The work of an event does not grow with the account: a hover over a list of 2,000 chats builds as many widgets
/// as a hover over a list of 200 chats.
#[test]
fn a_hover_builds_as_many_widgets_with_2000_chats_as_with_200() {
    struct Count(Arc<std::sync::atomic::AtomicUsize>);
    impl iced_test::selector::Selector for Count {
        type Output = ();
        fn select(&mut self, _: iced_test::selector::Candidate<'_>) -> Option<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            None
        }
        fn description(&self) -> String {
            "every widget".into()
        }
    }
    let widgets = |n: usize| {
        let mut app = demo_app(&format!("scale-{n}"));
        {
            let mut st = app.store.write();
            let chat = st.chats.iter().find(|c| c.flags & CHAT_PINNED == 0).expect("a chat").clone();
            for i in 0..n {
                let at = Some(format!("2001-01-01T00:00:{:02}.000Z", i % 60));
                let (id, title) = (format!("!scale{i}:beeper.local"), format!("Scale {i}"));
                st.upsert_chat(Chat { id, title, last_activity: at, ..chat.clone() });
            }
        }
        let first = app.visible_chats(&app.store.read())[0].id.clone();
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::Hover(first, true)));
        let id = app.main_window.expect("a main window");
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id));
        assert!(sim.find(Count(count.clone())).is_err(), "the count selects no widget");
        count.load(std::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(widgets(2000), widgets(200), "the widgets of a view do not grow with the chats");
}

#[test]
fn forward_rows_open_the_chat_menu_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("fwd-menu");
    // The forward picker lists the recent chats.
    let other = app.visible_chats(&app.store.read())[3].id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowUp), Modifiers::empty()));
    let _ = app.update(UiMessage::KeyPressed(Key::Character("f".into()), Modifiers::CTRL | Modifiers::SHIFT));
    let (id, title) = {
        let st = app.store.read();
        let c = forward_chats(&st, app.forward.as_ref().expect("the forward picker"))[1];
        (c.id.clone(), c.title.clone())
    };
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_forward(app.forward.as_ref().unwrap()));
        let at = sim.find(title.as_str()).expect("a forward row").visible_bounds().expect("a visible row").center();
        sim.point_at(at);
        let _ = sim.simulate(
            [mouse::Event::ButtonPressed(Button::Right), mouse::Event::ButtonReleased(Button::Right)]
                .map(iced::Event::Mouse),
        );
        sim.into_messages().collect()
    };
    assert!(
        matches!(&open[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(c))] if *c == id),
        "the chat menu of the row"
    );
    for m in open {
        let _ = app.update(m);
    }
    let outside = interact(&app, app.main_window.unwrap(), Size::new(1440.0, 900.0), Point::new(4.0, 896.0), {
        let left = [mouse::Event::ButtonPressed(Button::Left), mouse::Event::ButtonReleased(Button::Left)];
        left.map(iced::Event::Mouse).to_vec()
    });
    assert!(
        matches!(&outside[..], [UiMessage::ChatList(chat_list::Msg::CloseMenu)]),
        "a click outside the menu keeps the picker"
    );
    for m in outside {
        let _ = app.update(m);
    }
    assert!(app.list.chat_menu.is_none() && app.forward.is_some());
}

#[test]
fn messages_select_like_desktop() {
    let mut app = demo_app("multi");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let ids: Vec<String> = {
        let st = app.store.read();
        st.timeline_indices(chat).iter().map(|&i| st.messages[i].id.clone()).collect()
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
    let _ = app.update(UiMessage::Message(message::Msg::Click(ids[n - 2].clone())));
    let _ = app.update(UiMessage::Message(message::Msg::Click(ids[0].clone())));
    assert_eq!(app.pane.selected_msgs, [ids[0].as_str()], "Ctrl click toggles a message");
    let _ = app.update(UiMessage::Modifiers(Modifiers::SHIFT));
    let _ = app.update(UiMessage::Message(message::Msg::Click(ids[2].clone())));
    assert_eq!(
        app.pane.selected_msgs,
        [ids[0].as_str(), ids[1].as_str(), ids[2].as_str()],
        "Shift click adds the range"
    );
    let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    let _ = app.update(UiMessage::Message(message::Msg::Click(ids[1].clone())));
    assert_eq!(
        app.pane.selected_msgs,
        [ids[0].as_str(), ids[2].as_str()],
        "a click on a selected message unselects it"
    );
    let _ = app.update(UiMessage::Message(message::Msg::Click(ids[3].clone())));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Devices)));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Take Beeper with you").is_ok());
    let _ = sim.click("Get the app");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    assert!(app.app_qr && simulator(app.view_settings()).find("Get the app").is_err(), "the button shows the QR code");
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::About)));
    assert!(!app.app_qr, "another tab hides the QR code again");
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Devices)));
    let device = |name: &str, model: Option<&str>, kind: Option<&str>| beeper_core::matrix::DeviceInfo {
        id: name.into(),
        name: name.into(),
        model: model.map(Into::into),
        kind: kind.map(Into::into),
        last_seen_ms: None,
        verified: true,
        current: false,
    };
    let _ = app.update(UiMessage::Settings(settings::Msg::DevicesLoaded(Ok(vec![
        device("Beeper Desktop", Some("Linux PC"), Some("computer")),
        device("Beeper iPad", Some("iPad Air"), Some("tablet")),
        device("beeper-rs", None, None),
    ]))));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Linux PC").is_ok() && sim.find("iPad Air").is_ok(), "the device metadata model shows");

    // Desktop's "Logout" of another device: a confirmation, then `deleteDevices` with the auth of the Beeper JWT.
    let _ = sim.click("Logout");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Logout of Beeper Desktop?").is_ok(), "the dialog names the device");
    assert!(sim.find("This will disconnect all chat accounts on this device.").is_ok());
    drop(sim);
    let _ = app.update(UiMessage::Settings(settings::Msg::LogoutDevice("Beeper Desktop".into())));
    assert!(app.confirm.is_none() && app.devices_busy, "the logout runs");
    let mut sim = simulator(app.view_settings());
    let _ = sim.click("Logout");
    assert_eq!(sim.into_messages().count(), 0, "the buttons are disabled while the logout runs");
    let refused = Err(beeper_core::matrix::REAUTH.into());
    let _ = app.update(UiMessage::Settings(settings::Msg::DeviceLoggedOut("Beeper Desktop".into(), refused)));
    let modal = app.reauth.window.expect("a refused JWT asks for a sign-in code");
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(modal)));
    assert!(!app.devices_busy, "a closed modal ends the logout");
    let _ = app.update(UiMessage::Settings(settings::Msg::LogoutDevice("Beeper Desktop".into())));
    let _ = app
        .update(UiMessage::Settings(settings::Msg::DeviceLoggedOut("Beeper Desktop".into(), Err("M_UNKNOWN".into()))));
    assert!(!app.devices_busy);
    assert_eq!(app.status_banner, "Couldn't delete device. Please report a problem if this keeps happening.");
}

#[test]
fn storage_clears_like_desktop() {
    let mut app = demo_app("storage");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Storage)));
    let _ = app.update(UiMessage::Settings(settings::Msg::StorageScanned(beeper_core::storage::scan(&app.store))));
    let mut sim = tall(app.view_settings());
    for label in ["Messages", "Clear storage older than", "Clear All Cache", "Storage by Chat", "Note to self"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    let _ = sim.click("Note to self");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    let _ = app.update(UiMessage::Settings(settings::Msg::StorageAge(1, 0)));
    let _ = app.update(UiMessage::Ask(Some(ASK_CLEAR_CHAT)));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Clear storage for Note to self?").is_ok(), "the dialog names the chat");
    let picked: Vec<_> = app.storage_pick(2).into_iter().cloned().collect();
    assert!(!picked.is_empty() && picked.iter().all(|e| e.chat.as_deref() == Some(NOTE_CHAT_ID)));
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(beeper_core::storage::clear(app.store.clone(), picked)).unwrap();
    let after = beeper_core::storage::scan(&app.store);
    assert!(after.iter().all(|e| e.chat.as_deref() != Some(NOTE_CHAT_ID)), "the chat has no local data left");
    assert!(app.store.read().timeline_indices(NOTE_CHAT_ID).is_empty());
    let counts = [compact(34), compact(1234), compact(15_000), compact(2_500_000), grouped(1234567)];
    assert_eq!(counts, ["34", "1.2K", "15K", "2.5M", "1,234,567"]);
}

#[test]
fn integrations_like_desktop() {
    let mut app = demo_app("integrations");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Integrations)));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::ToggleSetting(|x| x.enable_pas_api ^= true)));
    assert!(*on.borrow() && app.store.read().settings.enable_pas_api, "the toggle starts the Desktop API");
    let _ = app.update(UiMessage::Integrations(integrations::Msg::McpTarget(3)));
    assert!(simulator(app.view_settings()).find("Prefer Streamable HTTP when possible.").is_ok());
    let _ = app.update(UiMessage::Settings(settings::Msg::JoinCommunity));
    assert_eq!(
        app.update(UiMessage::Settings(settings::Msg::JoinCommunity)).units(),
        0,
        "a second click waits for the join"
    );
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
        let _ = app.update(UiMessage::Integrations(integrations::Msg::Request(id.clone(), None)));
        (id, rx)
    };
    let (_, mut rx) = ask(&mut app, Some("1.2"));
    let mut sim = simulator(app.view_main());
    for label in ["Approve connection", "Allow sensitive actions", "Expires in", "Decline", "Approve"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    drop(sim);
    assert_eq!(app.token_ask.as_ref().unwrap().app, "Raycast v1.2");
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Write));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Expiry(Expiry(3600))));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Answer(true)));
    let token = rx.try_recv().unwrap().expect("approved");
    let stored = app.store.read().connect.validate(&token, "", "").expect("the token works");
    assert_eq!(stored.scopes, ["read", "write"], "Allow sensitive actions grants write");
    assert!(stored.expires_at.unwrap() - stored.created_at == 3_600_000 && app.token_ask.is_none());
    let (_, mut rx) = ask(&mut app, None);
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Answer(false)));
    assert_eq!(rx.try_recv().unwrap(), None, "Decline rejects the request");
    let (id, _) = ask(&mut app, None);
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Request(id, Some(Outcome::Timeout))));
    assert!(simulator(app.view_main()).find("The request has timed out.").is_ok());
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Close));
    assert!(app.token_ask.is_none());
}

#[test]
fn approved_connections_like_desktop() {
    let mut app = demo_app("connections");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Integrations)));
    assert!(simulator(app.view_settings()).find("Add a connection to get started").is_ok());
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Add(true)));
    let mut sim = simulator(app.view_settings());
    for label in ["Name (optional)", "Expires In", "Allow sensitive actions", "Create Access Token"] {
        assert!(sim.find(label).is_ok(), "{label} shows");
    }
    drop(sim);
    let _ = app.update(UiMessage::Integrations(integrations::Msg::AddName("Script".into())));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::AddWrite));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::AddExpiry(Expiry(0))));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::Create));
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
    let _ = app.update(UiMessage::Integrations(integrations::Msg::RevokeConfirmed));
    assert!(app.store.read().connect.validate(&token, "", "").is_err() && app.confirm.is_none());
    assert_eq!(crate::distance(50 * 60_000), "about 1 hour");
    assert_eq!(crate::distance(30 * 86_400_000), "about 1 month");
    assert_eq!(crate::distance(400 * 86_400_000), "about 1 year");
}

#[test]
fn remote_access_like_desktop() {
    let mut app = demo_app("remote");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Integrations)));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::RemoteAccess(true)));
    assert!(
        simulator(app.view_settings()).find("Enable Remote Access?").is_ok()
            && !app.store.read().settings.remote_access
    );
    let _ = app.update(UiMessage::Integrations(integrations::Msg::RemoteAccess(true)));
    assert!(app.store.read().settings.remote_access && app.confirm.is_none(), "Allow Remote Access turns it on");
    assert_eq!(app.relaunch_toast, Some("Remote Access will be enabled after relaunch"));
    let _ = app.update(UiMessage::Integrations(integrations::Msg::RemoteAccess(false)));
    assert!(!app.store.read().settings.remote_access && app.confirm.is_none(), "off needs no confirmation");
    app.store.write().config.launch_host = true;
    let mut sim = simulator(app.view_settings());
    let _ = sim.click("Remote Access");
    assert!(
        sim.into_messages().all(|m| !matches!(m, UiMessage::Integrations(integrations::Msg::RemoteAccess(_)))),
        "--pas-hostname locks the toggle"
    );
}

#[test]
fn ai_settings_like_desktop() {
    let mut app = demo_app("ai");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Ai)));
    let mut sim = simulator(app.view_settings());
    for heading in ["General", "Voice Transcription", "AI Mentions"] {
        assert!(sim.find(heading).is_ok(), "the {heading} group shows");
    }
    let mention = "Will send the 20 most recent messages, including sender names and any @mentioned users.";
    assert!(sim.find(mention).is_ok() && sim.find("Requires a paid subscription.").is_ok());
    let _ = sim.click("Whisper");
    let msgs: Vec<_> = sim.into_messages().collect();
    let upsell =
        msgs.iter().all(|m| matches!(m, UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::BeeperPlus))));
    assert!(!msgs.is_empty() && upsell, "a locked Plus toggle opens Beeper Plus");
    let rows = ["Automatically", "Improves accuracy."];
    let shown = |app: &BeeperGuiApp| rows.map(|r| simulator(app.view_settings()).find(r).is_ok());
    assert_eq!(shown(&app), [false; 2], "Desktop shows the transcription rows only while Whisper is on");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("transcribeWithOpenai", 1)));
    assert_eq!(shown(&app), [true; 2]);
}

#[test]
fn a_sent_link_keeps_its_preview() {
    let mut app = demo_app("preview");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let url = "https://example.com/";
    let edit = text_editor::Edit::Paste(Arc::new(format!("see {url}")));
    let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit))));
    let page = serde_json::json!({ "matched_url": url, "og:title": "Example" });
    app.pane.composer_links = vec![(url.into(), LinkState::Loaded(page, None))];
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    let links = &st.messages.last().unwrap().links;
    assert_eq!(links.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), ["Example"]);
}

#[test]
fn undo_send_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("undo-send");
    app.store.write().features.flags["features"]["*"]["undo-send"] = true.into();
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("undoSendDelay", 1)));
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let send = |app: &mut BeeperGuiApp, text: &str| {
        let edit = text_editor::Edit::Paste(Arc::new(text.into()));
        let _ = app.update(UiMessage::Composer(composer::Msg::Edit(text_editor::Action::Edit(edit))));
        let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
        app.store.read().messages.last().unwrap().id.clone()
    };
    let count = |app: &BeeperGuiApp| app.store.read().timeline_indices(NOTE_CHAT_ID).len();
    let before = count(&app);
    send(&mut app, "oops");
    assert_eq!((count(&app), app.queued.len()), (before + 1, 1), "the message waits");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("z".into()), Modifiers::COMMAND));
    assert_eq!(count(&app), before, "Ctrl Z takes the message back");
    assert_eq!(app.pane.composer.text().trim(), "oops", "the composer has the text again");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("undoOnPendingClick", 1)));
    let id = send(&mut app, "");
    let _ = app.update(UiMessage::Message(message::Msg::Click(id)));
    assert_eq!((count(&app), app.pane.composer.text().trim()), (before, "oops"), "a click on the message undoes");
    send(&mut app, "");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("Z".into()), Modifiers::COMMAND | Modifiers::SHIFT));
    assert!(app.queued.is_empty() && count(&app) == before + 1, "Ctrl Shift Z sends now");
    send(&mut app, "later");
    app.queued[0].due = 0;
    let _ = app.update(UiMessage::Composer(composer::Msg::SendQueued(None)));
    assert!(app.queued.is_empty(), "the delay sends");
    app.store.write().features.flags["features"]["*"]["undo-send"] = false.into();
    send(&mut app, "now");
    assert!(app.queued.is_empty(), "the free plan sends at once");
}

/// Turns off the demo, so that own messages show their send status. The seed messages count as messages that the
/// homeserver took.
fn online(app: &BeeperGuiApp) {
    let mut st = app.store.write();
    st.config.demo = false;
    let seeded: Vec<String> = st.messages.iter().map(|m| m.id.clone()).collect();
    st.sent.extend(seeded.into_iter().map(|id| (id.clone(), id)));
}

#[test]
fn failed_sends_show_retry_and_remove_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("failed-send");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    online(&app);
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("hi".into())));
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let _ = app.update(UiMessage::Composer(composer::Msg::SendQueued(Some(note.into()))));
    let id = {
        let mut st = app.store.write();
        let (row, _, out) = st.outbox.remove(0);
        let id = out.message().expect("a send has a local echo").to_string();
        st.pushed(row, Some(&id), Some(("M_FORBIDDEN: refused".into(), true)));
        id
    };
    // Desktop's `sendQueuedMessages` shows the error in a toast that stays, with Copy Error, Show and Retry.
    let failed = app.store.read().loaded(&id).expect("the local echo").clone();
    let _ = app.update(UiMessage::Upserted(Box::new(failed)));
    let toast = app.shown_toast().expect("a toast of the failed send");
    assert!(
        toast.text.starts_with("Something went wrong sending message to ")
            && toast.text.ends_with("\nM_FORBIDDEN: refused")
    );
    let labels: Vec<&str> = toast.buttons.iter().map(|b| b.0).collect();
    assert_eq!(labels, ["Copy Error", "Show", "Retry"]);
    assert!(matches!(&toast.buttons[2].1, UiMessage::Message(message::Msg::RetrySend(m)) if *m == id));
    assert_eq!(app.sticky_banner.as_ref(), Some(&app.status_banner), "the toast stays until it is closed");
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Not sent").is_ok() && sim.find("Remove").is_ok(), "a failed send shows its error and actions");
    let _ = sim.click("Retry");
    assert!(matches!(sim.into_messages().last(), Some(UiMessage::Message(message::Msg::RetrySend(m))) if *m == id));
    let _ = app.update(UiMessage::Toast(toasts::Msg::DismissBanner));
    let _ = app.update(UiMessage::Message(message::Msg::RetrySend(id.clone())));
    assert!(simulator(app.view_main()).find("Retrying ...").is_ok(), "Retry waits for the homeserver");
    let again = {
        let mut st = app.store.write();
        let (row, _, _) = st.outbox.remove(0);
        st.pushed(row, Some(&id), Some(("M_TOO_LARGE: too big".into(), true)));
        st.loaded(&id).expect("the local echo").clone()
    };
    let _ = app.update(UiMessage::Upserted(Box::new(again.clone())));
    assert!(app.status_banner.ends_with("\nM_TOO_LARGE: too big"), "a retry that fails shows its error again");
    let _ = app.update(UiMessage::Toast(toasts::Msg::DismissBanner));
    let _ = app.update(UiMessage::Upserted(Box::new(again)));
    assert!(app.status_banner.is_empty(), "the same failure shows once");
    let _ = app.update(UiMessage::Message(message::Msg::RemoveUnsent(id.clone())));
    let st = app.store.read();
    assert!(st.loaded(&id).is_none() && st.outbox.is_empty(), "Remove drops the echo and its send");
}

#[test]
fn own_messages_show_their_send_status_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("send-status");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    online(&app);
    let send = |app: &mut BeeperGuiApp, text: &str| {
        let _ = app.update(UiMessage::Composer(composer::Msg::Changed(text.into())));
        let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    };
    let status = |app: &BeeperGuiApp| {
        let mut sim = simulator(app.view_main());
        ["Queued", "Sending...", "Still sending...", "Sent", "Delivered"].map(|s| sim.find(s).is_ok())
    };
    send(&mut app, "one");
    if !app.queued.is_empty() {
        assert_eq!(status(&app), [true, false, false, false, false], "undo send queues the message");
        let _ = app.update(UiMessage::Composer(composer::Msg::SendQueued(Some(note.into()))));
    }
    let id = app.store.read().outbox[0].2.message().expect("a local echo").to_string();
    // The views of a debug build are slow, so the test sets the age of the message.
    let age = |app: &BeeperGuiApp, ms: i64| {
        let mut st = app.store.write();
        let i = st.message_idx(&id);
        st.update_message(i, |m| m.timestamp = beeper_core::config::rfc3339_from_millis(now_unix_millis() - ms));
    };
    age(&app, -60_000);
    assert_eq!(status(&app), [false, true, false, false, false], "a pending message is sending");
    age(&app, 5_000);
    assert_eq!(status(&app), [false, false, true, false, false], "after 5 s it is still sending");
    {
        let mut st = app.store.write();
        st.config.demo = true;
        let (row, ..) = st.outbox.remove(0);
        st.pushed(row, Some(&id), None);
        let echo = st.message(&id).clone();
        st.remove_message(note, &id).expect("the local echo");
        st.upsert_message(Message { id: "$sent".into(), ..echo });
    }
    assert_eq!(status(&app), [false, false, false, true, false], "the homeserver took the message");
    {
        let mut st = app.store.write();
        let i = st.message_idx("$sent");
        st.update_message(i, |m| m.is_delivered = true);
    }
    assert_eq!(status(&app), [false, false, false, false, true], "a bridge delivered the message");
}

#[test]
fn scroll_to_bottom_shows_when_you_scroll_up_like_desktop() {
    let mut app = demo_app("scroll-to-bottom");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    {
        let mut st = app.store.write();
        for n in 0..40 {
            st.send_message(NOTE_CHAT_ID, Some(format!("line {n}")), None, None).expect("a demo send");
        }
        // Like the composer, a send reads the chat.
        st.mark_chat_read(NOTE_CHAT_ID, true);
        let i = st.chat_idx(NOTE_CHAT_ID);
        st.update_chat(i, |c| c.unread_count = 2);
    }
    // The sensor at the bottom of the timeline tells when the timeline leaves its bottom and comes back.
    let scroll = |app: &BeeperGuiApp, y: f32| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view_main());
        sim.point_at(Point::new(900.0, 450.0));
        let redraw = || iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()));
        let wheel = iced::Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: 0.0, y } });
        let _ = sim.simulate([redraw(), wheel, redraw()]);
        let ups = sim.into_messages().filter_map(|m| match m {
            UiMessage::Timeline(timeline::Msg::Scrolled(up)) => Some(up),
            _ => None,
        });
        ups.collect::<Vec<bool>>()
    };
    assert_eq!(scroll(&app, 0.0), [false], "the timeline opens at its bottom");
    assert_eq!(scroll(&app, 300.0), [false, true], "a scroll up leaves the bottom");
    let fade = || std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(simulator(app.view_main()).find("New Messages").is_err(), "the bottom of the timeline shows");
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(true)));
    fade();
    let mut sim = simulator(app.view_main());
    assert!(sim.find("2").is_ok() && sim.find("Ctrl").is_ok() && sim.find("↓").is_ok(), "the pill shows the hotkey");
    let _ = sim.click("New Messages");
    assert!(matches!(sim.into_messages().last(), Some(UiMessage::Timeline(timeline::Msg::ScrollToBottom))));
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(false)));
    fade();
    assert!(simulator(app.view_main()).find("New Messages").is_err(), "at the bottom the pill goes away");
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(true)));
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    assert!(app.pane.scrolled_up.is_none(), "another chat opens at its bottom");
}

/// Desktop's timeline effect: a message that comes into the open chat reads it while the window is focused and the
/// timeline is at its bottom; otherwise the chat stays unread until the window is focused or the timeline is back down.
#[test]
fn the_open_chat_reads_new_messages_like_desktop() {
    let mut app = demo_app("open-chat-read");
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID && !c.is_unread()).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let arrive = |app: &mut BeeperGuiApp| {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| c.unread_count = 1);
        let m = st.messages.iter().rfind(|m| m.chat_id == id).unwrap().clone();
        drop(st);
        let _ = app.update(UiMessage::Upserted(Box::new(m)));
    };
    let unread = |app: &BeeperGuiApp| app.store.read().chat(&id).unread_count;
    arrive(&mut app);
    assert_eq!(unread(&app), 1, "an unfocused window keeps the chat unread");
    let _ = app.update(UiMessage::Window(windows::Msg::Focused(true)));
    assert_eq!(unread(&app), 0, "focusing the window reads it");
    arrive(&mut app);
    assert_eq!(unread(&app), 0, "a new message in view is read");
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(true)));
    arrive(&mut app);
    assert_eq!(unread(&app), 1, "scrolled up, it stays unread");
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(false)));
    assert_eq!(unread(&app), 0, "back at the bottom it is read");
}

/// Desktop's `unreadDividerMessageSortKey`: a Matrix room, whose messages keep no unread mark, shows "New" after the
/// read position that the chat had when it opened, and ↓ with nothing selected goes to the first message after it.
#[test]
fn the_unread_divider_follows_the_read_position_like_desktop() {
    let mut app = demo_app("read-position");
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID).unwrap().id.clone();
    {
        let mut st = app.store.write();
        st.load_chat(&id);
        let read = st.messages.iter().rfind(|m| m.chat_id == id).unwrap().clone();
        for i in st.timeline_indices(&id).to_vec() {
            st.update_message(i, |m| m.is_unread = Some(false));
        }
        let after: u64 = read.sort_key.parse().expect("a numeric sort key");
        for n in 0..3 {
            let (text, sort_key) = (format!("fresh {n}"), format!("{:016}", after + 1 + n));
            let m = Message {
                id: format!("$fresh{n}"),
                sort_key,
                text: Some(text),
                is_sender: Some(false),
                ..read.clone()
            };
            st.upsert_message(m);
        }
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| (c.unread_count, c.last_read_message_sort_key) = (3, Some(read.sort_key.clone())));
    }
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    assert_eq!(app.pane.unread_divider.as_deref(), Some("$fresh0"), "after the read position when the chat opened");
    let unread = |app: &BeeperGuiApp| app.store.read().chat(&id).unread_count;
    assert_eq!(unread(&app), 3, "the chat opens at its divider, and reads only what shows");
    assert!(app.pane.scrolled_up.is_some_and(|(up, _)| up), "above the bottom of the timeline");
    assert!(simulator(app.view_main()).find("New").is_ok());
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowDown), Modifiers::empty()));
    assert_eq!(app.pane.selected_msgs, ["$fresh0"], "↓ goes to the first message after the divider");
    let _ = app.update(UiMessage::Window(windows::Msg::Focused(true)));
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(false)));
    assert_eq!(unread(&app), 0, "the bottom of the timeline reads it");
    assert!(simulator(app.view_main()).find("New").is_ok(), "the divider stays while the chat is open");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let _ = app.update(UiMessage::SelectChat(id));
    assert!(app.pane.unread_divider.is_none() && simulator(app.view_main()).find("New").is_err(), "read, no divider");
}

/// Desktop's `onRangeChange`: above its bottom, the timeline reads the open chat through the last message in view
/// once the scroll stops, and a Matrix room, whose messages keep no unread mark, counts the messages left below.
#[test]
fn the_open_chat_reads_through_the_last_message_in_view_like_desktop() {
    let mut app = demo_app("read-in-view");
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID).unwrap().id.clone();
    let _ = app.update(UiMessage::Window(windows::Msg::Focused(true)));
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(true)));
    {
        let mut st = app.store.write();
        let last = st.messages.iter().rfind(|m| m.chat_id == id).unwrap().clone();
        let after: u64 = last.sort_key.parse().expect("a numeric sort key");
        for n in 0..40 {
            let text = format!("incoming {n}");
            let (id, sort_key) = (format!("$in{n}"), format!("{:016}", after + 1 + n));
            st.upsert_message(Message { id, sort_key, text: Some(text), is_sender: Some(false), ..last.clone() });
        }
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| (c.unread_count, c.last_read_message_sort_key) = (40, Some(last.sort_key.clone())));
    }
    let unread = |app: &BeeperGuiApp| app.store.read().chat(&id).unread_count;
    let redraw = || iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view_main());
    sim.point_at(Point::new(900.0, 450.0));
    let wheel = iced::Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: 0.0, y: 600.0 } });
    let _ = sim.simulate([redraw(), wheel, redraw()]);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let _ = sim.simulate([redraw()]);
    let relists: Vec<_> = sim.into_messages().filter(|m| matches!(m, UiMessage::Relist)).collect();
    assert!(!relists.is_empty(), "the stopped scroll tells the app");
    assert_eq!(unread(&app), 40, "nothing is read while the scroll moves");
    for m in relists {
        let _ = app.update(m);
    }
    let left = unread(&app);
    assert!((1..40).contains(&left), "read through the last message in view, {left} left below it");
    let receipt = app.store.read().outbox.iter().rev().find_map(|(_, _, o)| match o {
        Outgoing::Read(event) => Some(event.clone()),
        _ => None,
    });
    assert_eq!(receipt, Some(format!("$in{}", 39 - left)), "the receipt goes to the last message in view");
}

/// Desktop's `getLastParticipantName` and `messageSender`: a group preview and the timeline name the sender by the
/// member's current name, so a rename shows at once.
#[test]
fn previews_name_the_sender_by_the_current_name_like_desktop() {
    let mut app = demo_app("preview-sender");
    let (id, sender) = {
        let st = app.store.read();
        let c = app.visible_chats(&st).into_iter().find(|c| {
            let text = |m: &Message| m.message_type.as_deref() == Some("TEXT") && m.poll.is_none();
            c.chat_type == "group" && c.preview.as_ref().is_some_and(|m| m.is_sender != Some(true) && text(m))
        });
        let c = c.expect("a group previewed by someone else");
        (c.id.clone(), c.preview.as_ref().unwrap().sender_id.clone())
    };
    {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| {
            let u = c.participants.items.iter_mut().find(|u| u.id == sender).expect("the sender is a member");
            (u.full_name, u.custom_nickname) = (Some("Zed Quux".into()), None);
            c.draft = None;
            if let Some(p) = c.preview.as_mut() {
                (p.text, p.html, p.attachments, p.links) = (Some("hi".into()), None, Vec::new(), Vec::new());
            }
        });
    }
    assert!(simulator(app.view_sidebar()).find("Zed: hi").is_ok(), "the new first name");
    let _ = app.update(UiMessage::SelectChat(id));
    assert!(simulator(app.view_conversation()).find("Zed Quux").is_ok(), "the timeline names the sender the same way");
}

/// Desktop's `selfParticipant.hasExited`: a chat that the user was removed from has no composer, and the removal
/// previews it without a sender name, as an action.
#[test]
fn a_chat_the_user_left_like_desktop() {
    let mut app = demo_app("exited");
    let id = {
        let st = app.store.read();
        let row = |c: &&Chat| c.chat_type == "group" && c.is_pinned != Some(true);
        app.visible_chats(&st).into_iter().find(row).expect("a group in the list").id.clone()
    };
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let composer = || iced::widget::Id::new("composer-input");
    assert!(simulator(app.view_conversation()).find(composer()).is_ok(), "a composer before");
    {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| {
            let other = c.participants.items.iter().find(|u| u.is_self != Some(true)).unwrap().id.clone();
            for u in c.participants.items.iter_mut().filter(|u| u.is_self == Some(true)) {
                u.has_exited = Some(true);
            }
            c.draft = None;
            let p = c.preview.as_mut().expect("a preview");
            (p.sender_id, p.is_sender, p.message_type) = (other, Some(false), Some("MEMBERSHIP".into()));
            (p.text, p.html, p.attachments, p.links, p.poll) =
                (Some("Ann left".into()), None, Vec::new(), Vec::new(), None);
        });
    }
    assert!(simulator(app.view_conversation()).find(composer()).is_err(), "no composer after");
    let _ = app.update(UiMessage::Composer(composer::Msg::Type("typed anyway".into())));
    assert!(app.pane.composer.text().is_empty(), "typing goes nowhere without a composer");
    assert!(simulator(app.view_sidebar()).find("Ann left").is_ok(), "no sender name");
}

/// Desktop's `MembershipGrouper`: three member events of a day fold under "3 Membership Events", the group opens while
/// the keyboard selection is in it and closes once the selection leaves, and a group that closes drops its selection.
#[test]
fn membership_groups_follow_the_selection_like_desktop() {
    let mut app = demo_app("member-groups");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("membershipEvents", 1)));
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID && !c.is_unread()).unwrap().id.clone();
    {
        let mut st = app.store.write();
        st.load_chat(&id);
        let last = st.messages.iter().rfind(|m| m.chat_id == id).unwrap().clone();
        let after: u64 = last.sort_key.parse().expect("a numeric sort key");
        let rows =
            [("MEMBERSHIP", "Ann joined"), ("MEMBERSHIP", "Bob joined"), ("MEMBERSHIP", "Cy left"), ("TEXT", "hi")];
        for (n, (kind, text)) in rows.into_iter().enumerate() {
            let m = Message {
                id: format!("$m{n}"),
                sort_key: format!("{:016}", after + 1 + n as u64),
                text: Some(text.into()),
                message_type: Some(kind.into()),
                is_sender: Some(false),
                ..last.clone()
            };
            st.upsert_message(m);
        }
    }
    let _ = app.update(UiMessage::SelectChat(id));
    assert!(simulator(app.view_main()).find("3 Membership Events").is_ok());
    let step = |app: &mut BeeperGuiApp, down: bool| {
        let key = if down { Named::ArrowDown } else { Named::ArrowUp };
        let _ = app.update(UiMessage::KeyPressed(Key::Named(key), Modifiers::empty()));
    };
    step(&mut app, false);
    assert_eq!(app.pane.selected_msgs, ["$m3"]);
    assert!(app.pane.open_members.is_empty(), "the group stays closed");
    step(&mut app, false);
    assert_eq!(app.pane.selected_msgs, ["$m2"]);
    assert!(app.pane.open_members.contains("$m0"), "the selection opens the group");
    assert!(simulator(app.view_main()).find("Cy left").is_ok());
    step(&mut app, true);
    assert!(app.pane.open_members.is_empty(), "and it closes when the selection leaves");
    step(&mut app, false);
    let _ = app.update(UiMessage::Members(members::Msg::Toggle("$m0".into())));
    assert!(app.pane.open_members.is_empty() && app.pane.selected_msgs.is_empty(), "closing drops the selection");
}

/// Desktop's `rWe`: only the last of a sender's messages in a minute shows the time.
#[test]
fn times_show_once_a_minute_like_desktop() {
    let base = demo_app("times").store.read().messages[0].clone();
    let at = |id: &str, sender: &str, ts: &str| Message {
        id: id.into(),
        sender_id: sender.into(),
        timestamp: ts.into(),
        text: Some("hi".into()),
        message_type: Some("TEXT".into()),
        ..base.clone()
    };
    let (a, b) = (at("a", "@ann", "2026-11-16T10:15:05Z"), at("b", "@ann", "2026-11-16T10:15:50Z"));
    use crate::message::time_hidden;
    assert!(time_hidden(&a, Some(&b), false), "the same sender in the same minute");
    assert!(!time_hidden(&b, None, false), "the last message shows its time");
    assert!(!time_hidden(&a, Some(&at("c", "@bob", "2026-11-16T10:15:50Z")), false), "another sender");
    assert!(!time_hidden(&a, Some(&at("d", "@ann", "2026-11-16T10:16:01Z")), false), "the next minute");
    let emoji = Message { text: Some("🎉".into()), ..b.clone() };
    assert!(!time_hidden(&a, Some(&emoji), false), "a jumbo emoji has no time, so the one before keeps it");
}

/// Desktop's `.message-text` clamp: a message over 40 lines shows "Show more", which opens all of it.
#[test]
fn long_messages_clamp_like_desktop() {
    let mut app = demo_app("clamp");
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let mid = {
        let mut st = app.store.write();
        let i = st.messages.iter().rposition(|m| m.chat_id == id && m.message_type.as_deref() == Some("TEXT")).unwrap();
        let long = (1..=60).map(|n| format!("row {n}")).collect::<Vec<_>>().join("\n");
        st.update_message(i, |m| (m.text, m.html) = (Some(long), None)).id
    };
    let more = |app: &BeeperGuiApp, label: &str| simulator(app.view_conversation()).find(label).is_ok();
    assert!(more(&app, "Show more") && !more(&app, "Show less"), "60 lines are clamped");
    let _ = app.update(UiMessage::Message(message::Msg::ToggleClamp(mid.clone())));
    assert!(more(&app, "Show less") && !more(&app, "Show more"), "Show more opens it");
    let _ = app.update(UiMessage::Message(message::Msg::ToggleClamp(mid)));
    assert!(more(&app, "Show more"), "Show less clamps it again");
}

/// Desktop's `kf` media box in a 1000 px window, at Desktop's 0.45 share.
#[test]
fn media_boxes_like_desktop() {
    use crate::message::media_box;
    let at = |w, h| media_box(1000.0, Some((w, h)), 0.45);
    assert_eq!(media_box(1000.0, None, 0.45), (450.0, 400.0, true), "an unknown size is 400 high and crops");
    assert_eq!(at(100.0, 100.0), (100.0, 100.0, false), "never upscaled");
    assert_eq!(at(100.0, 50.0), (100.0, 80.0, true), "Desktop's strip rule takes small wide ones too");
    assert_eq!(at(2000.0, 1000.0), (450.0, 225.0, false), "as wide as the share");
    assert_eq!(at(500.0, 1000.0), (200.0, 400.0, false), "as high as 400");
    assert_eq!(at(300.0, 900.0), (160.0, 400.0, true), "a very tall one is 160 by 400, cropped");
    assert_eq!(at(4000.0, 100.0), (450.0, 80.0, true), "a very wide one is an 80 px strip, cropped");
    assert_eq!(media_box(1000.0, Some((2000.0, 1000.0)), 0.85).0, 800.0, "a hidden sidebar gives 0.85");
}

/// Desktop's `.spoiler`: a click shows the spoilers of a message, and the chat list hides them as blocks.
#[test]
fn spoilers_like_desktop() {
    let mut app = demo_app("spoilers");
    let id = {
        let st = app.store.read();
        let row = |c: &&Chat| c.chat_type == "single" && c.is_pinned != Some(true) && c.id != NOTE_CHAT_ID;
        app.visible_chats(&st).into_iter().find(row).expect("a DM in the list").id.clone()
    };
    let mid = {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&id).unwrap();
        let p = st.chats[idx].preview.clone().unwrap();
        let html = "<span data-mx-spoiler>secret</span>";
        let p = Message { text: Some("secret".into()), html: Some(html.into()), is_sender: Some(true), ..p };
        st.update_chat(idx, |c| (c.preview, c.draft) = (Some(p.clone()), None));
        p.id
    };
    assert!(simulator(app.view_sidebar()).find("██████").is_ok(), "the list hides the spoiler");
    let _ = app.update(UiMessage::OpenUrl(format!("{}{mid}", crate::SPOILER)));
    assert!(app.pane.spoilers.contains(&mid), "a click shows the spoilers of the message");
}

/// Desktop's `P1t`: a reply whose quoted message the timeline does not have shows "Loading…" while it loads, then
/// the quote, or "Message unavailable".
#[test]
fn unloaded_quotes_like_desktop() {
    let mut app = demo_app("quotes");
    let id = app.store.read().chats.iter().find(|c| c.id != NOTE_CHAT_ID).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let quoted = {
        let mut st = app.store.write();
        let i = st.messages.iter().rposition(|m| m.chat_id == id && m.message_type.as_deref() == Some("TEXT")).unwrap();
        let old = Message { id: "$old".into(), text: Some("a quote from long ago".into()), ..st.messages[i].clone() };
        st.update_message(i, |m| m.linked_message_id = Some("$old".into()));
        old
    };
    let shows = |app: &BeeperGuiApp, t: &str| simulator(app.view_conversation()).find(t).is_ok();
    assert!(shows(&app, "Loading…"), "the quote loads");
    assert!(app.wants.quotes.borrow().contains(&(id.clone(), "$old".to_string())), "the view asks for it");
    let _ = app.update(UiMessage::Loaded(loads::Msg::Quote("$old".into(), Err("gone".into()))));
    assert!(shows(&app, "Message unavailable"));
    let _ = app.update(UiMessage::Loaded(loads::Msg::Quote("$old".into(), Ok(Box::new(quoted)))));
    assert!(shows(&app, "a quote from long ago"), "the loaded quote shows");
}

/// Desktop's `bfe`: a quote collapses spaces and cuts its text at 128 characters, names the sender of a group inside
/// the text box, tags a voice note, and shows a deleted message as deleted.
#[test]
fn quote_details_like_desktop() {
    let mut app = demo_app("quote-details");
    let id = "!wa_family:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let base = {
        let mut st = app.store.write();
        let i = st.messages.iter().rposition(|m| m.chat_id == id && m.message_type.as_deref() == Some("TEXT")).unwrap();
        st.update_message(i, |m| m.linked_message_id = Some("$q".into()));
        let other = st.chat(&id).participants.items.iter().find(|u| u.is_self != Some(true)).unwrap().id.clone();
        Message {
            id: "$q".into(),
            sender_id: other,
            is_sender: Some(false),
            linked_message_id: None,
            ..st.messages[i].clone()
        }
    };
    let quote = |app: &mut BeeperGuiApp, m: Message| {
        let _ = app.update(UiMessage::Loaded(loads::Msg::Quote("$q".into(), Ok(Box::new(m)))));
    };
    let name = crate::sender_label(app.store.read().chat(&id), &base);
    let shows = |app: &BeeperGuiApp, t: &str| simulator(app.view_conversation()).find(t).is_ok();
    let long = format!("{}\n\n  tail", "word ".repeat(40));
    quote(&mut app, Message { text: Some(long), attachments: Vec::new(), ..base.clone() });
    let cut = format!("{}...", "word ".repeat(40).trim_end().chars().take(125).collect::<String>());
    assert!(shows(&app, &cut), "cut at 128 characters with the spaces collapsed");
    assert!(shows(&app, &name), "the group names the sender");
    let voice = Attachment {
        attachment_type: "audio".into(),
        is_voice_note: Some(true),
        file_name: Some("voice.ogg".into()),
        ..Attachment::default()
    };
    quote(&mut app, Message { text: None, attachments: vec![voice], ..base.clone() });
    assert!(shows(&app, "🎤 [Voice Note]"), "a media-only quote tags its media");
    quote(&mut app, Message { is_deleted: Some(true), text: Some("secret".into()), ..base });
    assert!(shows(&app, "Message Deleted") && !shows(&app, "secret"), "a deleted quote shows as deleted");
}

/// Desktop's sortable pinned grid (`lY`): a pin dragged over another pin takes its place, and the drop keeps the
/// order in `favourites_order`.
#[test]
fn pins_drag_into_order_like_desktop() {
    let mut app = demo_app("pin-drag");
    let pins = |app: &BeeperGuiApp| -> Vec<String> {
        let st = app.store.read();
        app.pins_and_rows(&st).0.iter().map(|c| c.id.clone()).collect()
    };
    let before = pins(&app);
    assert!(before.len() >= 2, "the demo has pins: {before:?}");
    let (first, last) = (before[0].clone(), before[before.len() - 1].clone());
    let _ = app.update(UiMessage::Pins(pins::Msg::Hover(Some(first.clone()))));
    let _ = app.update(UiMessage::Pins(pins::Msg::Press));
    let _ = app.update(UiMessage::Pins(pins::Msg::DragMoved(Point::new(10.0, 10.0))));
    let _ = app.update(UiMessage::Pins(pins::Msg::DragMoved(Point::new(12.0, 12.0))));
    assert_eq!(pins(&app), before, "a move under 5 px is a click");
    let _ = app.update(UiMessage::Pins(pins::Msg::DragMoved(Point::new(40.0, 10.0))));
    let _ = app.update(UiMessage::Pins(pins::Msg::Hover(Some(last.clone()))));
    let _ = app.update(UiMessage::Pins(pins::Msg::DragEnd));
    let after = pins(&app);
    let want: Vec<String> = before[1..].iter().cloned().chain([first.clone()]).collect();
    assert_eq!(after, want, "the first pin goes to the place of the last");
    let st = app.store.read();
    assert!(st.pin_rank[&first] > st.pin_rank[&last], "the order stays in the pin ranks");
}

/// Desktop's header card (`Amt`): the pointer on it shows the members of the chat, which stay 500 ms after it leaves.
#[test]
fn the_header_shows_the_members_on_hover_like_desktop() {
    let mut app = demo_app("header-members");
    let _ = app.update(UiMessage::SelectChat("!wa_family:beeper.com".into()));
    let chat = app.store.read().chat("!wa_family:beeper.com").clone();
    assert!(app.header_members(&chat).is_some(), "a group has members to list");
    assert!(!app.header_hover.0, "no list before the pointer comes");
    let _ = app.update(UiMessage::HeaderHover(true));
    assert!(app.header_hover.0, "the members show");
    let _ = app.update(UiMessage::HeaderHover(false));
    let left = app.header_hover.1;
    assert!(app.header_hover.0, "and stay while the pointer may come back");
    let _ = app.update(UiMessage::HeaderHover(true));
    let _ = app.update(UiMessage::HeaderHoverEnd(left));
    assert!(app.header_hover.0, "coming back keeps them");
    let _ = app.update(UiMessage::HeaderHover(false));
    let _ = app.update(UiMessage::HeaderHoverEnd(app.header_hover.1));
    assert!(!app.header_hover.0, "then they go");
}

/// Electron's `autoHideMenuBar` of Desktop's menu bar: Alt on its own shows it and hides it, a key with Alt does not,
/// a menu runs its item and hides, and the pointer moves between the open menus.
#[test]
fn the_menu_bar_shows_with_alt_like_desktop() {
    let mut app = demo_app("menu-bar");
    app.focused = true;
    let id = app.main_window.expect("a main window");
    let alt = |app: &mut BeeperGuiApp, keys: &[Option<Key>]| {
        let _ = app.update(UiMessage::Modifiers(Modifiers::ALT));
        for k in keys.iter().flatten() {
            let _ = app.update(UiMessage::WindowKey(id, k.clone(), Modifiers::ALT));
        }
        let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    };
    alt(&mut app, &[]);
    assert_eq!(app.menu_bar, Some(None), "Alt shows the menu bar");
    let titles: Vec<String> = app.menus().into_iter().map(|m| m.0).collect();
    assert_eq!(titles, ["Beeper", "File", "Edit", "View", "Window", "Help"]);
    alt(&mut app, &[]);
    assert_eq!(app.menu_bar, None, "and Alt hides it");
    alt(&mut app, &[Some(Key::Character("f".into()))]);
    assert_eq!(app.menu_bar, None, "Alt with another key is no toggle");
    let _ = app.update(UiMessage::MenuBar(menubar::Msg::Show(Some(Some(0)))));
    let _ = app.update(UiMessage::MenuBar(menubar::Msg::Hover(3)));
    assert_eq!(app.menu_bar, Some(Some(3)), "an open menu follows the pointer");
    assert!(simulator(app.view(id)).find("Hide Sidebar").is_ok(), "the View menu shows its items");
    let hidden = app.sidebar_hidden;
    let _ = app.update(UiMessage::MenuBar(menubar::Msg::Run(Box::new(UiMessage::RunHotkey("Toggle Sidebar")))));
    assert_eq!((app.menu_bar, app.sidebar_hidden), (None, !hidden), "the item runs and the menu bar hides");
}

/// Desktop's `activityTimestamp` and `isArchivedInList`: a draft moves its chat to the top, and out of the archive.
#[test]
fn drafts_move_chats_like_desktop() {
    let mut app = demo_app("draft-order");
    let unpinned = |app: &BeeperGuiApp| -> Vec<String> {
        let st = app.store.read();
        app.visible_chats(&st).iter().filter(|c| c.is_pinned != Some(true)).map(|c| c.id.clone()).collect()
    };
    let id = unpinned(&app).into_iter().rfind(|c| c != NOTE_CHAT_ID).expect("an old chat");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(id.clone(), ChatFlag::Archived)));
    assert!(!unpinned(&app).contains(&id), "archived");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Archived)));
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let _ = app.update(UiMessage::Composer(composer::Msg::Type("not done yet".into())));
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    assert!(!unpinned(&app).contains(&id), "a chat with a draft leaves the archive list");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::AllChats)));
    assert_eq!(unpinned(&app).first(), Some(&id), "and comes back to the top of the inbox");
}

/// Desktop's `showSeenIcon`: a group shows who has seen it on own messages and the last one, not on a received
/// message in the middle.
#[test]
fn seen_lines_like_desktop() {
    let mut app = demo_app("seen-rule");
    let id = "!wa_family:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    // Four received messages from Ann after the one there is.
    let ids: Vec<String> = {
        let mut st = app.store.write();
        let base = st.messages[st.timeline_indices(&id)[0]].clone();
        (1..=4)
            .map(|n| {
                let m = Message {
                    id: format!("$ann{n}"),
                    sender_id: "@ann:x".into(),
                    sender_name: Some("Ann".into()),
                    is_sender: Some(false),
                    text: Some(format!("note {n}")),
                    html: None,
                    timestamp: format!("2030-01-01T10:0{n}:00Z"),
                    sort_key: beeper_core::store::sort_key(9_000_000_000_000 + n),
                    seen: Default::default(),
                    ..base.clone()
                };
                st.upsert_message(m);
                format!("$ann{n}")
            })
            .collect()
    };
    let seen = |app: &mut BeeperGuiApp, mid: &str| {
        let mut st = app.store.write();
        for m in &ids {
            let i = st.message_idx(m);
            let by = if m == mid { vec![("@zed:x".to_string(), "2030-01-01T11:00:00Z".to_string())] } else { vec![] };
            st.update_message(i, |x| x.seen = by.into_iter().collect());
        }
    };
    let shown = |app: &BeeperGuiApp| simulator(app.view_conversation()).find("Seen by @zed:x").is_ok();
    seen(&mut app, "$ann2");
    assert!(!shown(&app), "not under a received message in the middle");
    seen(&mut app, "$ann4");
    assert!(shown(&app), "under the last message");
}

/// Desktop's `accountDisplayName` titles an account space: "<user> · <network>" when the bridge has two accounts.
#[test]
fn account_space_titles_like_desktop() {
    let mut app = demo_app("space-title");
    let b = app.store.read().bridge_accounts.iter().find(|b| b.remote_profile.is_some()).cloned().unwrap();
    let two = beeper_core::models::BridgeAccountInfo { account_id: format!("{}-2", b.account_id), ..b.clone() };
    {
        let mut st = app.store.write();
        let mut all = st.bridge_accounts.clone();
        all.push(two);
        st.set_cloud_accounts(all);
    }
    let title = {
        let st = app.store.read();
        format!("{} · {}", account_user(&st, &b.account_id), network_label(&b.account_id))
    };
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some(b.account_id.clone()))));
    assert!(simulator(app.view_sidebar()).find(title.as_str()).is_ok(), "{title}");
}

/// Desktop's Join a Matrix room view: a bad address shows Desktop's error under the field, and a known room opens.
#[test]
fn join_a_matrix_room_like_desktop() {
    let mut app = demo_app("join-matrix");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleJoinMatrix));
    assert!(simulator(app.view_main()).find("Room address or Matrix link").is_ok());
    for (input, error) in [
        ("#!x:y", "This starts with both # and !, so it isn't a valid Matrix address."),
        ("room", "Enter a valid Matrix room address like #room:server, or paste a Matrix room link."),
    ] {
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query(input.into())));
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::JoinMatrixRoom));
        assert!(app.join_room.1.as_deref().is_some_and(|e| e.starts_with(error)), "{input}");
        let shown = app.join_room.1.clone().unwrap();
        assert!(simulator(app.view_main()).find(shown).is_ok(), "the error shows under the field");
    }
    let known = app.store.read().chats.iter().find(|c| c.id.starts_with('!') && c.id.contains(':')).unwrap().id.clone();
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query(known.clone())));
    assert!(app.join_room.1.is_none(), "typing clears the error");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::JoinMatrixRoom));
    assert!(
        !app.join_matrix_mode && app.pane.selected_chat_id.as_deref() == Some(known.as_str()),
        "a known room opens"
    );
    assert!(app.new_chats.is_empty(), "the opened room drops its draft");
    let join = |app: &mut BeeperGuiApp| {
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleJoinMatrix));
        app.join_room.0 = true;
        app.creating_chat
    };
    let draft = join(&mut app);
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::MatrixRoomJoined(draft, Ok("!new:localhost".into()), false)));
    assert_eq!(
        app.join_room,
        (false, Some("You joined the room, but Beeper couldn't open it yet. Try again.".into())),
        "a room the sync has not brought shows why in the view"
    );
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::MatrixRoomJoined(draft, Ok(known.clone()), false)));
    assert!(app.new_chats.is_empty() && app.pane.selected_chat_id.as_deref() == Some(known.as_str()), "it opens");
    let draft = join(&mut app);
    let _ = app.update(UiMessage::SelectChat(known.clone()));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::MatrixRoomJoined(draft, Err("forbidden".into()), true)));
    assert_eq!(
        app.toast.as_ref().map(|t| t.text.as_str()),
        Some("Couldn't join this Matrix room without routing information. Try a link with routing information, or check your access."),
        "after the user left the view the error is a toast"
    );
}

/// Desktop's `thread-header-status`: "typing" under the name of a 1:1 chat while the other person types.
#[test]
fn the_header_shows_typing_in_a_dm_like_desktop() {
    let mut app = demo_app("header-typing");
    let (dm, group) = {
        let st = app.store.read();
        let dm = st.chats.iter().find(|c| c.chat_type == "single" && c.participants.items.len() == 2).unwrap();
        let group = st.chats.iter().find(|c| c.chat_type == "group").unwrap();
        let peer = |c: &Chat| c.participants.items.iter().find(|u| u.is_self != Some(true)).unwrap().id.clone();
        ((dm.id.clone(), peer(dm)), (group.id.clone(), peer(group)))
    };
    for (chat, user) in [&dm, &group] {
        app.store.write().typing.insert(chat.clone(), vec![user.clone()]);
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let shown = simulator(app.view_main()).find("typing").is_ok();
        assert_eq!(shown, *chat == dm.0, "only a 1:1 chat shows the activity in its header");
    }
}

#[test]
fn deleted_and_encrypted_messages_look_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("deleted-rows");
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let deleted = {
        let mut st = app.store.write();
        let ids: Vec<String> =
            st.timeline_indices(note).iter().rev().take(2).map(|&i| st.messages[i].id.clone()).collect();
        st.delete_message(note, &ids[0]).expect("delete a message");
        let i = st.message_idx(&ids[1]);
        st.update_message(i, |m| (m.text, m.html) = (Some(beeper_core::matrix::UTD_TEXT.into()), None));
        ids[0].clone()
    };
    let mut sim = simulator(app.view_main());
    assert!(sim.find("This message is waiting for decryption.").is_ok(), "an encrypted message waits for its keys");
    assert!(sim.find("Edited").is_err(), "Desktop shows no Edited label on a deleted message");
    let _ = sim.click("This message has been deleted");
    let clicked =
        sim.into_messages().any(|m| matches!(m, UiMessage::Message(message::Msg::Click(id)) if id == deleted));
    assert!(clicked, "a deleted message keeps its row and its click selection");
}

#[test]
fn attachments_show_download_and_time_on_hover_like_desktop() {
    let mut app = demo_app("attachment-hover");
    let (chat, id, time) = {
        let st = app.store.read();
        let m = st.messages.iter().rev().find(|m| {
            m.text.as_deref().is_none_or(str::is_empty)
                && m.attachments.first().is_some_and(|a| a.attachment_type == "img")
        });
        let m = m.expect("a demo image message");
        (m.chat_id.clone(), m.id.clone(), format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1))
    };
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = app.update(UiMessage::Message(message::Msg::HoverEnter(id.clone())));
    let download = icon::DOWNLOAD.to_string();
    assert!(simulator(app.view_main()).find(download.as_str()).is_err(), "a gallery opens its items in the carousel");
    {
        // Captions take the other images of the demo gallery out of it, so that this image shows alone.
        let mut st = app.store.write();
        let others = st.timeline_indices(&chat).iter().copied();
        let others: Vec<usize> =
            others.filter(|&i| st.messages[i].id != id && !st.messages[i].attachments.is_empty()).collect();
        for i in others {
            st.update_message(i, |m| m.text = Some("caption".into()));
        }
    }
    let _ = app.update(UiMessage::Message(message::Msg::HoverExit(id.clone())));
    assert!(simulator(app.view_main()).find(time.as_str()).is_err(), "an image has no time until you hover it");
    let _ = app.update(UiMessage::Message(message::Msg::HoverEnter(id.clone())));
    let mut sim = simulator(app.view_main());
    assert!(sim.find(time.as_str()).is_ok(), "the hover shows the time of an image");
    let _ = sim.click(download.as_str());
    assert!(
        matches!(sim.into_messages().last(), Some(UiMessage::Message(message::Msg::Download(m, Some(0)))) if *m == id)
    );
}

#[test]
fn plus_rows_follow_the_feature_flags() {
    let mut app = demo_app("features");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    let clicks = |app: &BeeperGuiApp, label: &str| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1024.0, 4000.0), app.view_settings());
        assert!(sim.find(label).is_ok(), "{label} shows");
        let _ = sim.click(label);
        sim.into_messages().filter(|m| matches!(m, UiMessage::Settings(settings::Msg::SetPref(..)))).count()
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
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    for (tab, pref) in prefs() {
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(tab)));
        for v in 0..=pref.max_value() {
            let _ = app.update(UiMessage::Settings(settings::Msg::SetPref(pref.key, v)));
            let _ = simulator(app.view_settings());
            let _ = simulator(app.view_main());
        }
        let _ = app.update(UiMessage::Settings(settings::Msg::SetPref(pref.key, pref.default_value())));
    }
}

#[test]
fn random_input_does_not_panic() {
    let var =
        |k, default| std::env::var(k).map_or(default, |n| n.parse().unwrap_or_else(|_| panic!("{k} is a number")));
    let (seed, steps) = (var("FUZZ_SEED", 0), var("FUZZ_STEPS", 100));
    std::thread::scope(|s| {
        for seed in seed..seed + 16 {
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
    // Desktop's Font size sets the root font (`--custom-font-size`), which only the text in rem follows.
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("fontSize", 0)));
    assert_eq!((app.zoom, crate::style::rem(16.0)), (1.0, 12.0), "the font size 9 is a root font of 12 px");
    let _ = app.update(UiMessage::KeyPressed(Key::Character("=".into()), Modifiers::CTRL));
    assert_eq!((app.zoom, crate::style::rem(16.0)), (1.1, 12.0), "the zoom keeps the root font");
}

/// Desktop's windows open at their size in screen pixels whatever the zoom and the font size, and iced opens a
/// window at its size times the scale factor.
#[test]
fn windows_open_at_their_size_at_any_zoom_like_desktop() {
    use iced::futures::StreamExt;
    use iced::keyboard::{Key, Modifiers};
    let opened = |app: &mut BeeperGuiApp| {
        let task = app.open_settings(SettingsTab::General);
        let actions = iced_runtime::task::into_stream(task).expect("a window opens");
        iced::futures::executor::block_on(
            actions
                .filter_map(|a| async move {
                    match a {
                        iced_runtime::Action::Window(iced_runtime::window::Action::Open(_, s, _)) => Some(s.size),
                        _ => None,
                    }
                })
                .boxed()
                .next(),
        )
    };
    let mut app = demo_app("zoomed-windows");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("fontSize", 8)));
    assert_eq!(opened(&mut app), Some(SETTINGS_SIZE), "the font size 17 does not zoom the window");
    app.settings_window = None;
    for _ in 0..10 {
        let _ = app.update(UiMessage::KeyPressed(Key::Character("=".into()), Modifiers::CTRL));
    }
    assert_eq!(app.zoom, 2.0);
    assert_eq!(opened(&mut app), Some(SETTINGS_SIZE / 2.0), "800 by 740 screen pixels at 200%");
}

/// The sizes of the windows are in iced's logical pixels, which the zoom scales like Desktop's CSS pixels: the
/// sidebar hides under 600 of them, and a zoom resizes the viewport.
#[test]
fn the_zoom_resizes_the_viewport_like_desktop() {
    use iced::keyboard::{Key, Modifiers};
    let mut app = demo_app("zoom-viewport");
    let main = app.main_window.expect("the main window");
    let zoom = |app: &mut BeeperGuiApp, key: &str| {
        let _ = app.update(UiMessage::KeyPressed(Key::Character(key.into()), Modifiers::CTRL));
    };
    zoom(&mut app, "=");
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(main, Size::new(640.0, 500.0))));
    assert!(!app.sidebar_hidden, "640 CSS pixels at 110% keep the sidebar");
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(main, Size::new(590.0, 500.0))));
    assert!(app.sidebar_hidden, "590 CSS pixels hide it");
    let _ = app.update(UiMessage::Window(windows::Msg::Resized(main, Size::new(640.0, 500.0))));
    assert!(!app.sidebar_hidden, "and 640 show it again");
    zoom(&mut app, "0");
    assert_eq!((app.pane.size.width.round(), app.sidebar_hidden), (704.0, false), "a reset widens the viewport");
    zoom(&mut app, "=");
    zoom(&mut app, "=");
    assert!(app.pane.size.width < 600.0 && app.sidebar_hidden, "a zoom in narrows it, at {}", app.zoom);
    zoom(&mut app, "0");
    assert_eq!((app.pane.size.width.round(), app.sidebar_hidden), (704.0, false));
}

/// Sets Desktop's Font size (9 to 36) and zoom (in percent) of `app`.
fn font_and_zoom(app: &mut BeeperGuiApp, font: u8, zoom: u16) {
    app.store.write().settings.zoom_percent = zoom;
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("fontSize", font - 9)));
    assert_eq!((app.zoom, crate::style::rem(16.0)), (f32::from(zoom) / 100.0, f32::from(font) + 3.0));
}

/// Desktop's Font size is its root font (`--custom-font-size`), which only the sizes in rem follow: the text, not the
/// space bar, whose buttons are in px. Only the zoom scales those, and the viewport with them.
#[test]
fn the_font_size_scales_the_text_and_not_the_space_bar_like_desktop() {
    let mut app = demo_app("font-size-rail");
    let main = app.main_window.expect("the main window");
    let bounds = |sim: &mut Simulator<UiMessage>, id: &'static str| {
        sim.find(iced::widget::Id::new(id)).unwrap_or_else(|_| panic!("{id} shows")).bounds()
    };
    for zoom in [50, 100, 150, 200] {
        let mut rail = None;
        for font in [13, 9, 10, 12, 14, 16, 18, 20, 22, 24] {
            font_and_zoom(&mut app, font, zoom);
            let size = app.pane.size;
            let viewport = MAIN_SIZE * (100.0 / f32::from(zoom));
            assert_eq!(
                (size.width.round(), size.height.round()),
                (viewport.width.round(), viewport.height.round()),
                "the viewport"
            );
            let mut sim = Simulator::with_size(Default::default(), size, app.view(main));
            let (more, media) = (bounds(&mut sim, "rail-more"), bounds(&mut sim, "rail-media"));
            let rail = *rail.get_or_insert((more, media));
            assert_eq!((more, media), rail, "the space bar at font {font} and {zoom}% is the one of font 13");
            let label = sim.find("Unread threads").expect("the unread threads pill").bounds();
            assert_eq!(label.height, crate::style::rem(16.0), "its text in rem at font {font}");
        }
    }
    // At 24 the Media button still draws its icon, which the spaces above it used to cover.
    font_and_zoom(&mut app, 24, 100);
    let mut sim = Simulator::with_size(Default::default(), app.pane.size, app.view(main));
    let media = bounds(&mut sim, "rail-media");
    let shot = crate::message::tests::pixels(&mut sim);
    let k = shot.width() as f32 / app.pane.size.width;
    let dark = (0..36).flat_map(|y| (0..36).map(move |x| (x, y))).any(|(x, y)| {
        let px = shot.get_pixel(((media.x + x as f32) * k) as u32, ((media.y + y as f32) * k) as u32).0;
        px[..3].iter().all(|&c| c < 100)
    });
    assert!(dark, "the Media icon draws at font 24");
}

/// Desktop's settings row (`.label-wrapper label`) keeps its label, which wraps anywhere (`overflow-wrap: anywhere`)
/// and shrinks (`min-width: 0`), before its select (`flex: 0 0 auto`), at any font size and zoom.
#[test]
fn settings_labels_stay_clear_of_their_selects_at_any_font_size_like_desktop() {
    let mut app = demo_app("font-size-settings");
    let _ = app.open_settings(SettingsTab::Appearance);
    let id = app.settings_window.expect("the settings window");
    for zoom in [50, 100, 150, 200] {
        for font in [9, 13, 16, 20, 24] {
            font_and_zoom(&mut app, font, zoom);
            let size = SETTINGS_SIZE * (100.0 / f32::from(zoom));
            let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
            let label = sim.find("Badge count").expect("the label").bounds();
            let select = sim.find(iced::widget::Id::new("select-badgeCount")).expect("the select").bounds();
            let at = format!("at font {font} and {zoom}%: {label:?} {select:?}");
            assert!(label.x + label.width <= select.x, "the label ends before the select {at}");
            assert!((select.center_y() - label.center_y()).abs() < 1.0, "the select centers on the label {at}");
            let line = crate::style::rem(16.0);
            let whole = crate::style::text_width("Badge count", 13.0, Weight::Normal);
            let room = select.x - 6.0 - label.x;
            // The Font size alone leaves the row its width, so the label keeps its line, as in Desktop.
            assert!(zoom > 100 || room >= whole, "the label has its room {at}");
            if room >= whole {
                assert!(label.width >= whole - 0.5 && label.height == line, "the label on one line {at}");
            } else if room >= crate::style::text_width("Badge", 13.0, Weight::Normal) {
                assert!(label.height == 2.0 * line, "the label wraps between its words {at}");
            }
        }
    }
}

/// Desktop's `.message .timestamp` is 11 px, so the time of a message keeps its size while the text follows the Font
/// size.
#[test]
fn message_times_keep_their_px_size_like_desktop() {
    let mut app = demo_app("font-size-times");
    let main = app.main_window.expect("the main window");
    let (chat, time) = {
        let st = app.store.read();
        let chat = st.chats.iter().find(|c| c.title == "Note to self").expect("the demo note").id.clone();
        let last = st.chat_timeline(&chat).into_iter().last().expect("a message").timestamp.clone();
        (chat, crate::time::format_short_ts(&last, false))
    };
    let _ = app.update(UiMessage::SelectChat(chat));
    for font in [9, 13, 24] {
        font_and_zoom(&mut app, font, 100);
        let mut sim = Simulator::with_size(Default::default(), app.pane.size, app.view(main));
        let at = sim.find(time.as_str()).expect("the time of the last message").bounds();
        let px = crate::style::fixed_width(&time, 11.0, Weight::Normal);
        assert_eq!(at.width, px, "11 px text at font {font}: {time} {at:?}");
        // The read receipt is `--semantic-typo-size-label-sm` text, in rem.
        let sent = sim.find("Sent").expect("the receipt").bounds();
        assert_eq!(sent.height, crate::style::rem(15.0), "rem text at font {font}");
    }
}

/// Text that Desktop sets in px keeps its size at any Font size: the filter count (`.FilterButton-module__count`,
/// 11 px), the empty command bar (`.command-bar .empty`, 14 px), the title and the empty list of the Keyboard
/// Shortcuts (`.modal h3`, 24 px, and `.hotkey-search-empty`, 16 px) and the tiles of the chat theme
/// (`.ChatAppearancePicker-module__tileNone`, 11 px).
#[test]
fn px_text_keeps_its_size_like_desktop() {
    use crate::style::fixed_width;
    let mut app = demo_app("font-size-px");
    let main = app.main_window.expect("the main window");
    let note = app.store.read().chats.iter().find(|c| c.title == "Note to self").expect("the demo note").id.clone();
    let width =
        |sim: &mut Simulator<'_, UiMessage>, s: &str| sim.find(s).unwrap_or_else(|_| panic!("{s}")).bounds().width;
    // The widths of each text at each font size.
    let mut widths: Vec<[f32; 5]> = Vec::new();
    for font in [9, 13, 24] {
        font_and_zoom(&mut app, font, 100);
        app.set_filters(0b11);
        let count = width(&mut Simulator::with_size(Default::default(), app.pane.size, app.view(main)), "2");
        app.set_filters(0);
        let _ = app.update(UiMessage::RunHotkey("Command Bar"));
        let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("zzqqxx".into())));
        let empty = width(&mut Simulator::with_size(Default::default(), app.pane.size, app.view(main)), "No results");
        let _ = app.update(UiMessage::CloseModals);
        let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Modal(Some("zzqqxx".into()))));
        let mut sim = Simulator::with_size(Default::default(), app.pane.size, app.view(main));
        let (title, none) = (width(&mut sim, "Keyboard Shortcuts"), width(&mut sim, "No results"));
        drop(sim);
        let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Modal(None)));
        let mut sim = Simulator::with_size(Default::default(), Size::new(400.0, 800.0), app.view_chat_theme(&note));
        widths.push([count, empty, title, none, width(&mut sim, "None")]);
    }
    assert!(widths.iter().all(|w| *w == widths[0]), "the same widths at fonts 9, 13 and 24: {widths:?}");
    let px = [
        fixed_width("2", 11.0, Weight::Normal),
        fixed_width("No results", 14.0, Weight::Normal),
        fixed_width("Keyboard Shortcuts", 24.0, Weight::Bold),
        fixed_width("No results", 16.0, Weight::Normal),
        fixed_width("None", 11.0, Weight::Normal),
    ];
    for (w, px) in widths[0].iter().zip(px) {
        assert!((w - px).abs() < 2.0, "{w} is the width at its px size {px}");
    }
}

#[test]
fn search_finds_custom_items_like_desktop() {
    let mut app = demo_app("search-custom");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::Pick(hit("wallpaper").expect("a hit"))));
    assert_eq!(app.active_settings_tab, SettingsTab::Appearance);
    assert_eq!(app.pref_flash.map(|f| f.0), Some("Default chat theme"));
    let _ = simulator(app.view_settings());
}

/// Desktop's Settings is a new window each time it opens, so a search typed before it closed does not hide the
/// sections of the next one.
#[test]
fn settings_search_starts_empty_like_desktop() {
    let mut app = demo_app("search-reopen");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    let _ = app.update(UiMessage::Settings(settings::Msg::Query("time formatminimized".into())));
    assert!(simulator(app.view_settings()).find("Notifications").is_err(), "a search without hits hides the sections");
    let id = app.settings_window.expect("the Settings window");
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    assert_eq!(app.settings_query, "");
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("time formatminimized").is_err() && sim.find("Notifications").is_ok(), "the sections show again");
}

#[test]
fn settings_nav_follows_the_plan() {
    let mut app = demo_app("settings-nav");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Beeper Plus").is_ok() && sim.find("Free").is_ok(), "a free account can subscribe");
    let _ = sim.click("Incognito").expect("the incognito row");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    assert_eq!(app.active_settings_tab, SettingsTab::BeeperPlus, "a locked row opens the subscription prompt");
    let plus = Profile { plan: Plan::PlusPlus, can_subscribe: false, ..Profile::default() };
    app.store.write().whoami = plus;
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::General)));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Beeper Plus").is_err(), "a subscriber does not see Beeper Plus");
    assert!(sim.find("PlusPlus").is_ok() && sim.find("++").is_ok());
    assert!(SettingsTab::listed(&app.store.read()).all(|t| t != SettingsTab::BeeperPlus));
}

#[test]
fn hotkey_recorder_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut app = demo_app("hotkey-recorder");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Hotkeys)));
    let ch = |c: &str| Key::Character(c.into());
    let keys = |app: &BeeperGuiApp| app.store.read().settings.hotkey_keys("Search").to_string();
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Record("Search")));
    let _ = app.settings_key(ch("k"), Modifiers::empty());
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Recorder(true)));
    assert_eq!(app.status_banner, "You must also use a modifier key like Ctrl, Shift or Option/Alt");
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Recorder(false)));
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Record("Search")));
    let _ = app.settings_key(ch("j"), Modifiers::CTRL);
    assert!(simulator(app.view_settings()).find("Ctrl J is already used by Command Bar").is_ok());
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Recorder(true)));
    assert_eq!(keys(&app), "ctrl+k", "a used combination is not saved");
    let _ = app.settings_key(Key::Named(Named::Shift), Modifiers::SHIFT);
    let _ = app.settings_key(ch("k"), Modifiers::SHIFT | Modifiers::CTRL);
    let _ = app.settings_key(ch("q"), Modifiers::SHIFT | Modifiers::CTRL);
    let _ = app.settings_key(Key::Named(Named::Escape), Modifiers::empty());
    let _ = app.settings_key(Key::Named(Named::Enter), Modifiers::empty());
    assert_eq!(keys(&app), "ctrl+shift+k", "a new key starts again, and a second key is ignored");
    assert!(app.hotkey_rec.is_none() && app.relaunch_toast.is_some());
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Ctrl Shift K").is_ok());
    drop(sim);
    assert!(simulator(app.view_main())
        .find("Beeper has to be relaunched for all hotkey changes to take effect")
        .is_ok());
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Revert("Search")));
    assert_eq!(keys(&app), "ctrl+k");
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Record("Switch First 9 Chats Modifier")));
    let _ = app.settings_key(Key::Named(Named::Control), Modifiers::empty());
    let _ = app.settings_key(ch("x"), Modifiers::CTRL);
    let _ = app.settings_key(Key::Named(Named::Shift), Modifiers::CTRL);
    let _ = app.update(UiMessage::Hotkeys(hotkeys::Msg::Recorder(true)));
    let modifiers = app.store.read().settings.hotkey_keys("Switch First 9 Chats Modifier").to_string();
    assert_eq!(modifiers, "ctrl+shift", "a modifier hotkey records only modifiers");
}

#[test]
fn profile_page_like_desktop() {
    use crate::profile::Msg;
    let mut app = demo_app("profile-page");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Profile)));
    let me = app.store.read().me().clone();
    let username = me.username.clone().expect("a username");
    let shows = |app: &BeeperGuiApp, text: &str| simulator(app.view_settings()).find(text).is_ok();
    assert!(
        shows(&app, "Beeper Profile") && shows(&app, "Your username is how other people can contact you on Beeper.")
    );
    assert!(!shows(&app, "Save"), "Save shows only after a change");
    let _ = app.update(UiMessage::Profile(Msg::Name("New Name".into())));
    assert!(shows(&app, "Save"));
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::General)));
    assert_eq!(app.active_settings_tab, SettingsTab::Profile, "unsaved changes keep the page");
    assert!(shows(&app, "You have unsaved changes"));
    let _ = app.update(UiMessage::Profile(Msg::Leave(false)));
    assert_eq!(app.active_settings_tab, SettingsTab::General);
    assert!(app.profile.name.is_none(), "Discard drops the edit");
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Profile)));
    let _ = app.update(UiMessage::Profile(Msg::Menu));
    assert!(shows(&app, "Show Recovery Key") && shows(&app, "Logout") && shows(&app, "Delete account"));
    let _ = app.update(UiMessage::Profile(Msg::Delete(Some("nobody".into()))));
    assert!(!app.profile.menu, "a menu item closes the menu");
    assert!(shows(&app, &format!("To confirm, type your username {username} below.")));
    let _ = app.update(UiMessage::Profile(Msg::ConfirmDelete));
    assert_eq!(app.profile.delete.as_ref().map(|d| d.1), Some(false), "a wrong username does not delete");
    let _ = app.update(UiMessage::Profile(Msg::Delete(Some(username))));
    let _ = app.update(UiMessage::Profile(Msg::ConfirmDelete));
    assert_eq!(app.profile.delete.as_ref().map(|d| d.1), Some(true));
    let _ = app.update(UiMessage::Profile(Msg::Deleted(Err("offline".into()))));
    assert!(shows(&app, "Error deleting your account: offline"));
    let _ = app.update(UiMessage::Profile(Msg::Delete(None)));
    let _ = app.update(UiMessage::Profile(Msg::Email("new@example.com".into())));
    let _ = app.update(UiMessage::Profile(Msg::Save));
    assert!(shows(&app, "Change Email") && shows(&app, "Confirm New Email"));
    let _ = app.update(UiMessage::Profile(Msg::EmailField(0, "wrong@example.com".into())));
    let _ = app.update(UiMessage::Profile(Msg::EmailField(2, "new@example.com".into())));
    let _ = app.update(UiMessage::Profile(Msg::SaveEmail));
    assert_eq!(app.profile.change_email.as_ref().map(|c| c.1), Some(false), "the current email must match");
    let current = me.email.clone().expect("an email");
    let _ = app.update(UiMessage::Profile(Msg::EmailField(0, current.to_uppercase())));
    let _ = app.update(UiMessage::Profile(Msg::SaveEmail));
    assert_eq!(app.profile.change_email.as_ref().map(|c| c.1), Some(true), "the email saves");
    let hits = search_settings(&app.store.read(), "recovery code");
    assert_eq!(hits.first(), Some(&SettingsHit::Section(SettingsTab::Profile, true)));
}

/// Desktop's backup banner and `ShowRecoveryKeyModal`: a key this device made over a day ago shows "If you lose your
/// device..." over the chat list, and the key typed again after its copy marks it backed up.
#[test]
fn recovery_key_backup_like_desktop() {
    use crate::profile::{KeyStep, Msg};
    let mut app = demo_app("recovery-backup");
    let key = "EsTc LW2K PGiF wKEo 9bCn nYwJ 3bR7 jAxZ hLoh 6Ycm Nkx5 YTqh";
    {
        let mut st = app.store.write();
        st.features.flags["features"]["*"]["deferred-recovery-code-setup"] = true.into();
        let s = &mut st.beeper_state;
        (s.secrets.recovery_code, s.has_backed_up_code) = (key.into(), false);
        s.recovery_code_generated_at = beeper_core::config::now_unix_millis() - 2 * 24 * 60 * 60 * 1000;
    }
    let banner = |app: &BeeperGuiApp| simulator(app.view_sidebar()).find("If you lose your device...").is_ok();
    assert!(banner(&app), "an unsaved key a day old shows the banner");
    let _ = app.update(UiMessage::Profile(Msg::SaveKeyNow));
    assert!(!banner(&app) && app.settings_window.is_some(), "Save key now opens the key and closes the banner");
    let shows = |app: &BeeperGuiApp, text: &str| simulator(app.view_settings()).find(text).is_ok();
    assert!(shows(&app, "Copy to Clipboard"));
    let _ = app.update(UiMessage::Profile(Msg::CopyRecoveryKey));
    assert!(shows(&app, "Next"), "an unsaved key goes on to its check");
    let _ = app.update(UiMessage::Profile(Msg::KeyNext));
    assert!(shows(&app, "Verify recovery key"));
    let wrong = key.replace("EsTc", "AAAA");
    let _ = app.update(UiMessage::Profile(Msg::KeyPasted(Some(wrong))));
    assert!(shows(&app, "Entered key does not match your recovery key"));
    let _ = app.update(UiMessage::Profile(Msg::KeyPasted(Some(key.into()))));
    assert!(matches!(app.profile.recovery_key, Some(KeyStep::Verify(_, true, None))), "the right key verifies");
    let _ = app.update(UiMessage::Profile(Msg::KeyVerified(Ok(()))));
    assert!(shows(&app, "Verified") && app.store.read().beeper_state.has_backed_up_code);
}

#[test]
fn downloads_location_like_desktop() {
    let mut app = demo_app("downloads-location");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    let os = downloads_dir(&app.store.read().settings);
    assert!(os.ends_with("Downloads"), "the downloads folder of the OS is the default");
    let _ = app.update(UiMessage::Settings(settings::Msg::DownloadsPicked(Ok(vec!["/tmp/picked".into()]))));
    assert_eq!(downloads_dir(&app.store.read().settings), std::path::PathBuf::from("/tmp/picked"));
    let _ = app.update(UiMessage::Settings(settings::Msg::DownloadsPicked(Ok(vec![]))));
    assert_eq!(app.store.read().settings.downloads_dir, "/tmp/picked", "a cancel keeps the folder");
    let hits = search_settings(&app.store.read(), "downloads");
    let labels: Vec<String> = hits.into_iter().map(SettingsHit::label).collect();
    assert!(labels.contains(&"Downloads Preferences".to_string()), "{labels:?}");
}

#[test]
fn quick_reaction_settings_like_desktop() {
    use crate::emoji::EmojiMsg;
    let mut app = demo_app("quick-reaction-settings");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    assert!(simulator(app.view_settings()).find("Quick reaction emoji").is_err(), "only for React with...");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("messageDoubleClick", 5)));
    assert!(simulator(app.view_settings()).find("Quick reaction emoji").is_ok());
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Appearance)));
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("quickReactionsCustomized", 1)));
    assert_eq!(
        crate::emoji::quick_reactions(&app.store.read(), &[], true),
        ["👍", "❤️", "😂"],
        "Desktop's custom defaults"
    );
    let _ = app.update(UiMessage::QuickEmoji(Some(2)));
    let _ = simulator(app.view_settings());
    let _ = app.update(UiMessage::Emoji(EmojiMsg::Pick(0)));
    let custom = crate::emoji::quick_reactions(&app.store.read(), &[], true);
    assert!(custom[1] != "❤️" && app.pane.emoji.is_none(), "a pick sets the custom reaction: {custom:?}");
    let _ = app.update(UiMessage::QuickEmoji(Some(0)));
    let _ = app.update(UiMessage::Emoji(EmojiMsg::Pick(0)));
    assert_eq!(app.store.read().settings.quick_reaction, custom[1], "any emoji is the Quick React emoji");
}

/// Desktop's `TimeSuggestionsPreference`: Add with its checks, Enter and Escape, remove, drag to reorder, and Reset.
#[test]
fn time_suggestions_editor_like_desktop() {
    use crate::time_suggestions::{suggestion_date, Msg, INPUT};
    use iced::keyboard::{key::Named, Key, Modifiers};
    use iced::mouse::Event as M;
    let mut app = demo_app("time-suggestions");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
    fn tall(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
        Simulator::with_size(Default::default(), Size::new(1024.0, 4000.0), app.view_settings())
    }
    let list = |app: &BeeperGuiApp| -> Vec<String> {
        app.store.read().settings.time_suggestions().into_iter().map(String::from).collect()
    };
    let defaults: Vec<String> = prefs::TIME_SUGGESTIONS.map(String::from).to_vec();
    let about = "Customize time suggestions for quick reminders in the command bar and context menus. Try \
        \"7am Tomorrow\", \"In 3 weeks\", \"Next Monday\".";
    {
        let mut sim = tall(&app);
        assert!(sim.find("Time Suggestions").is_ok(), "the section heading");
        assert!(sim.find(format!("{about} You can add up to 8.")).is_ok());
        let at = parse_when_millis("Tomorrow", now_unix_millis()).unwrap();
        assert!(sim.find("Tomorrow").is_ok() && sim.find(suggestion_date(at)).is_ok(), "the phrase and its time");
        assert!(sim.find("Reset").is_ok());
        let _ = sim.click("Add").expect("the Add button");
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(&mut app, msgs);
    }
    assert_eq!(app.time_edit.input.as_deref(), Some(""), "Add opens the input");
    assert!(tall(&app).find("Add").is_err(), "the input replaces Add and Reset");
    let type_in = |app: &mut BeeperGuiApp, text: &str| {
        let mut sim = tall(app);
        let _ = sim.click(iced::widget::Id::new(INPUT)).expect("the input");
        let _ = sim.typewrite(text);
        let _ = sim.tap_key(Key::Named(Named::Enter));
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(app, msgs);
    };
    for (text, why) in [("Tomorrow", "already listed"), ("yesterday", "a past time"), ("banana", "not a time")] {
        type_in(&mut app, text);
        assert_eq!(list(&app), defaults, "{why}");
        assert_eq!(app.time_edit.input.as_deref(), Some(text), "the input stays open");
        let _ = app.update(UiMessage::TimeSuggestions(Msg::Input(String::new())));
    }
    type_in(&mut app, "In 3w");
    let at = parse_when_millis("In 3w", now_unix_millis()).unwrap();
    assert!(tall(&app).find(suggestion_date(at)).is_ok(), "the preview of the new phrase");
    assert_eq!(list(&app), [&defaults[..], &["In 3w".to_string()]].concat(), "Enter saves a new future time");
    assert!(app.time_edit.input.is_none(), "the save closes the input");
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Add));
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Input("7am Tomorrow".into())));
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Save));
    assert_eq!(list(&app).len(), 8);
    assert!(tall(&app).find(format!("{about} You have reached the limit of 8.")).is_ok());
    {
        let mut sim = tall(&app);
        let _ = sim.click("Add");
        assert_eq!(sim.into_messages().count(), 0, "Add is disabled at the limit");
    }
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Add));
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Input("in 5 days".into())));
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Save));
    assert_eq!(list(&app).len(), 8, "no ninth phrase");
    let settings = app.settings_window.expect("the settings window");
    let _ = app.update(UiMessage::WindowKey(settings, Key::Named(Named::Escape), Modifiers::empty()));
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Cancel));
    assert!(app.time_edit.input.is_none() && app.settings_window.is_some(), "Cancel closes the input");

    // The remove button of a row, the close icon on the line of its phrase.
    let remove = |app: &mut BeeperGuiApp, phrase: &str| {
        let mut sim = tall(app);
        let row = sim.find(phrase).expect("the phrase").bounds().center_y();
        let close = icon::CLOSE.to_string();
        let found = sim.find(move |c: iced_test::selector::Candidate<'_>| match c {
            iced_test::selector::Candidate::Text { content, bounds, .. }
                if content == close && (bounds.center_y() - row).abs() < 10.0 =>
            {
                Some(bounds.center())
            }
            _ => None,
        });
        sim.point_at(found.expect("the remove button"));
        let _ = sim.simulate(iced_test::simulator::click());
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(app, msgs);
    };
    remove(&mut app, "In Two Hours");
    assert!(!list(&app).contains(&"In Two Hours".to_string()) && list(&app).len() == 7, "{:?}", list(&app));

    // A press on a row and a move over another row, then the drop.
    let first = list(&app)[0].clone();
    {
        let mut sim = tall(&app);
        let from = sim.find(first.as_str()).expect("the first row").bounds().center();
        sim.point_at(from);
        let _ = sim
            .simulate([Event::Mouse(M::CursorMoved { position: from }), Event::Mouse(M::ButtonPressed(Button::Left))]);
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(&mut app, msgs);
    }
    assert_eq!(app.time_edit.drag.as_ref().map(|d| d.0), Some(0), "the press grabs the row");
    let third = list(&app)[2].clone();
    {
        let mut sim = tall(&app);
        let to = sim.find(third.as_str()).expect("the third row").bounds().center();
        sim.point_at(to);
        let _ = sim.simulate([Event::Mouse(M::CursorMoved { position: to })]);
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(&mut app, msgs);
    }
    assert_eq!(list(&app)[0], first, "the order saves at the drop");
    let _ = app.update(UiMessage::TimeSuggestions(Msg::Grab(None)));
    let order = list(&app);
    assert_eq!((order[1].as_str(), order[2].as_str()), (third.as_str(), first.as_str()), "{order:?}");

    while list(&app).len() > 1 {
        let phrase = list(&app)[0].clone();
        remove(&mut app, &phrase);
    }
    let last = list(&app);
    remove(&mut app, &last[0].clone());
    assert_eq!(list(&app), last, "the last phrase stays");
    {
        let mut sim = tall(&app);
        let _ = sim.click("Reset").expect("the Reset button");
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(&mut app, msgs);
    }
    assert!(app.store.read().settings.time_suggestions.is_empty(), "Reset stores no list");
    assert_eq!(list(&app), defaults, "which means Desktop's defaults");
    let hits = search_settings(&app.store.read(), "time sugg");
    assert!(hits.contains(&SettingsHit::Custom(SettingsTab::General, "Time suggestions")), "{hits:?}");
}

/// Desktop's `Cze` and `cy`: the Remind Me menus and the command bar offer the custom time suggestions.
#[test]
fn menus_use_the_time_suggestions_like_desktop() {
    let mut app = demo_app("time-suggestion-menus");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    app.store.write().update_settings(|s| s.time_suggestions = vec!["In 3w".into(), "7am Tomorrow".into()]);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(1))));
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("In 3w").is_ok() && sim.find("7am Tomorrow").is_ok(), "the chat menu has the custom phrases");
        assert!(sim.find("In Two Hours").is_err(), "and not the defaults");
        let _ = sim.click("In 3w").expect("the phrase");
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        apply_all(&mut app, msgs);
    }
    let at = parse_when_millis("In 3w", now_unix_millis()).unwrap();
    let reminder = app.store.read().chat(&id).reminder.clone().map(|r| r.remind_at_ms);
    assert!(reminder.is_some_and(|r| (r - at).abs() < 60_000), "the phrase sets the reminder");
    assert_eq!(app.store.read().settings.last_time, "In 3w", "and is kept for Last Used");
    let _ = app.update(UiMessage::CloseModals);
    app.selection = app.visible_chats(&app.store.read()).iter().take(2).map(|c| c.id.clone()).collect();
    let (menu, _) = app.view_selection_menu(0);
    let mut sim = simulator(menu);
    assert!(sim.find("7am Tomorrow").is_ok() && sim.find("In an Hour").is_err(), "the selection bar menu too");
    drop(sim);
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(id))));
    let labels: Vec<String> =
        app.commands(&app.store.read(), "").into_iter().filter_map(|c| c.render.map(|r| r.0)).collect();
    assert!(labels.contains(&"in 3w".into()) && labels.contains(&"7am tomorrow".into()), "{labels:?}");
    assert!(!labels.contains(&"in two hours".into()), "the command bar drops the defaults: {labels:?}");
}

fn apply_all(app: &mut BeeperGuiApp, msgs: Vec<UiMessage>) {
    for m in msgs {
        let _ = app.update(m);
    }
}

#[test]
fn polls_like_desktop() {
    let mut app = demo_app("polls");
    let (chat, mid) = ("!tg_rust:beeper.com", "msg_1004");
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let me = app.store.read().me().id.clone();
    let poll = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.message(mid).poll.clone().expect("a poll")
    };
    let click = |app: &BeeperGuiApp, label: &str| -> Vec<UiMessage> {
        let mut sim = simulator(app.view_main());
        let _ = sim.click(label).expect("the option");
        sim.into_messages().filter(|m| matches!(m, UiMessage::Message(message::Msg::VotePoll(..)))).collect()
    };
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Select one option").is_ok() && sim.find("2 votes").is_ok());
        let st = app.store.read();
        let m = st.message(mid);
        assert_eq!(
            (crate::chat_list::message_tag(m), crate::chat_list::preview_body(m)),
            (Some((crate::chat_list::POLL_TAG, "Poll", None)), None)
        );
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
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
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
    let i = app.store.read().chat_idx(chat);
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
    assert!(hidden_actions(&app.store.read()).contains(&6), "Create Poll is behind the plus button");
    let i = app.store.read().chat_idx(&chat);
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
        let m = &st.messages[*st.timeline_indices(&chat).last().expect("the sent poll")];
        let poll = m.poll.as_ref().expect("a poll message");
        let labels: Vec<&str> = poll.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!((poll.question.as_str(), labels, poll.max_selections), ("Best editor?", vec!["Vim", "Emacs"], 2));
        assert_eq!(m.message_type.as_deref(), Some("POLL"));
    }
    let sent = {
        let st = app.store.read();
        st.messages[*st.timeline_indices(&chat).last().expect("the sent poll")].id.clone()
    };
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(sent.clone())));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("End Poll").expect("the sender can end the poll");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    {
        let st = app.store.read();
        assert!(st.message(&sent).poll.as_ref().expect("a poll").ended);
    }
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(sent.clone())));
    assert!(simulator(app.view_main()).find("End Poll").is_err(), "an ended poll has no End Poll");
    assert!(simulator(app.view_main()).find("Edit…").is_err(), "a poll is not edited");
    let _ = app.update(UiMessage::OpenPollCreator(chat.clone()));
    let id = app.poll_windows[0].id;
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Poll(id, e)));
    edit(&mut app, PollEdit::Question("Lunch?".into()));
    let _ = app.update(UiMessage::Window(windows::Msg::CloseRequested(id)));
    assert!(app.poll_windows[0].discarding && found(&app, "Discard poll?") && found(&app, "Keep editing"));
    edit(&mut app, PollEdit::Discard(false));
    edit(&mut app, PollEdit::Question(String::new()));
    edit(&mut app, PollEdit::Close);
    assert!(!app.poll_windows[0].discarding, "an empty draft closes without a question");
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
    assert!(app.poll_windows.is_empty());
}

#[test]
fn chat_theme_like_desktop() {
    let mut app = demo_app("chat-theme");
    let chat = app.store.read().chats[0].id.clone();
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Appearance)));
    {
        let mut sim = simulator(app.view_settings());
        assert!(
            sim.find("Pick a wallpaper and bubble color").is_ok() && sim.find("Only you see your chat theme").is_ok()
        );
    }
    let swatch = |id: &str| Some(Bubble::Swatch { id: id.into() });
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Bubble(None, swatch("blue"))));
    let _ =
        app.update(UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(None, Wallpaper::Gradient { id: "linen".into() })));
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(Some(chat.clone()), Wallpaper::None)));
    let stored = serde_json::to_value(&app.store.read().settings.chat_appearance).expect("json");
    let want = serde_json::json!({
        "global": { "wallpaper": { "type": "gradient", "id": "linen" }, "bubble": { "type": "swatch", "id": "blue" } },
        "perChat": { chat.as_str(): { "wallpaper": { "type": "none" } } },
    });
    assert_eq!(stored, want, "the chat theme is stored like Desktop's chat_appearance");
    let a = app.store.read().settings.chat_appearance.clone();
    let dir = app.store.read().config.data_dir.clone();
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
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(
        Some(chat.clone()),
        Wallpaper::Color { id: "mint".into() },
    )));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Bubble(None, None)));
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(None, Wallpaper::None)));
    let a = app.store.read().settings.chat_appearance.clone();
    assert!(a.theme(None).bubble.is_none(), "the default bubble color clears");
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Open(chat.clone())));
    assert_eq!(app.theme_windows.len(), 1);
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Open(chat.clone())));
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
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::WallpaperPicked(Some(chat.clone()), Ok(vec![picked]))));
    let themed = app.store.read().settings.chat_appearance.clone();
    let Some(Wallpaper::Photo { file_name }) = themed.theme(Some(&chat)).wallpaper else { panic!("a photo wallpaper") };
    assert!(file_name.ends_with(".png") && crate::chat_theme::photo_dir(&dir).join(&file_name).exists());
    let (wallpaper, _) = crate::chat_theme::resolve(&themed, Some(&chat), true, &dir);
    assert!(matches!(wallpaper, Some((crate::chat_theme::Paint::Photo(..), Color::WHITE))));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(Some(chat.clone()), Wallpaper::None)));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("incognito", 1)));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("incognitoIndicator", 0)));
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
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("label:lbl_work".into()))));
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
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())));
    let edit = |app: &mut BeeperGuiApp, e| drop(app.update(UiMessage::Labels(labels::Msg::Change(e))));
    let found = |app: &BeeperGuiApp, s: &str| {
        simulator(app.view_create_label(app.label_window.as_ref().expect("the label window"))).find(s).is_ok()
    };
    let step = |app: &BeeperGuiApp| app.label_window.as_ref().expect("the label window").step;
    assert!(found(&app, "Organize chats in custom spaces.") && found(&app, "Get Started"), "the intro shows first");
    assert!(app.store.read().settings.labels_intro_seen);
    edit(&mut app, LabelEdit::Advance);
    assert!(found(&app, "Chats") && !found(&app, "Selected"));
    edit(&mut app, LabelEdit::Advance);
    assert_eq!(step(&app), LabelStep::Pick, "Next needs a chat");
    edit(&mut app, LabelEdit::Toggle(chats[0].clone()));
    edit(&mut app, LabelEdit::Toggle(chats[1].clone()));
    assert!(found(&app, "Selected"));
    edit(&mut app, LabelEdit::Search("zzzz".into()));
    assert!(found(&app, "Clear search to show all selected chats") && !found(&app, "Selected"));
    edit(&mut app, LabelEdit::Advance);
    assert!(step(&app) == LabelStep::Details && found(&app, "Back"));
    edit(&mut app, LabelEdit::Advance);
    assert!(app.label_window.is_some(), "a label needs a name");
    edit(&mut app, LabelEdit::Color(3));
    edit(&mut app, LabelEdit::Color(11));
    edit(&mut app, LabelEdit::Icon(DEFAULT_LABEL_ICON));
    edit(&mut app, LabelEdit::Icon("3257-5950"));
    edit(&mut app, LabelEdit::Icon("3257-5950"));
    edit(&mut app, LabelEdit::Name(" Friends ".into()));
    edit(&mut app, LabelEdit::Advance);
    assert_eq!(app.label_settings.len(), 1, "a new label opens its Label Settings");
    let st = app.store.read();
    let label = st.labels.iter().find(|l| l.name == "Friends").expect("the new label");
    assert_eq!(label.rooms, chats);
    assert_eq!((label.color, label.icon.as_deref()), (Some(11), None), "a second pick clears the icon");
    assert!(chats.iter().all(|c| st.chat(c).label_ids.contains(&label.id)));
    assert_eq!(app.list.selected_account_filter, Some(format!("label:{}", label.id)), "the new label opens");
    assert_eq!(app.status_banner, "Created Friends");
    drop(st);
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert!(app.store.read().labels.iter().all(|l| l.name != "Friends") && app.list.selected_account_filter.is_none());
}

#[test]
fn edit_and_delete_labels_like_desktop() {
    let mut app = demo_app("label-menu");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("account:label:lbl_work".into())));
    let menu = |app: &BeeperGuiApp| simulator(app.view_account_menu("label:lbl_work")).find("Label Settings").is_ok();
    assert!(menu(&app));
    let _ = app.update(UiMessage::Labels(labels::Msg::Edit("lbl_work".into())));
    let ready = |app: &BeeperGuiApp| app.label_window.as_ref().expect("the label window").ready();
    assert_eq!(app.label_window.as_ref().map(|d| d.step), Some(LabelStep::Details), "Edit opens the Edit Label sheet");
    simulator(app.view_create_label(app.label_window.as_ref().expect("open"))).find("Done").expect("a Done button");
    let _ = app.update(UiMessage::Labels(labels::Msg::Change(LabelEdit::Name(" ".into()))));
    assert!(!ready(&app), "a label needs a name");
    let _ = app.update(UiMessage::Labels(labels::Msg::Change(LabelEdit::Name("Work Team".into()))));
    assert!(ready(&app));
    let _ = app.update(UiMessage::Labels(labels::Msg::Change(LabelEdit::Advance)));
    let rooms = {
        let st = app.store.read();
        let work = st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label");
        assert_eq!(work.name, "Work Team");
        work.rooms.clone()
    };
    assert_eq!(app.status_banner, "Updated Work Team");
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.name == "Work" && l.rooms == rooms));
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("label:lbl_work".into()))));
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenSettings("lbl_work".into())));
    let _ = app.update(UiMessage::Labels(labels::Msg::Pane(app.label_settings[0].id, LabelPane::Delete)));
    simulator(app.view_confirm(ASK_DELETE_LABEL)).find("Delete").expect("a Delete button");
    app.store.write().set_label_muted("lbl_work", true);
    app.patch_settings(|s| s.spaces_order = vec![SpaceOrder { id: "label:lbl_work".into(), visible: true }]);
    let _ = app.update(UiMessage::Labels(labels::Msg::Delete("lbl_work".into())));
    {
        let st = app.store.read();
        assert!(st.labels.iter().all(|l| l.id != "lbl_work"));
        assert!(rooms.iter().all(|r| !st.chat(r).label_ids.iter().any(|l| l == "lbl_work")));
    }
    assert_eq!(app.list.selected_account_filter, None, "the inbox opens when its label is deleted");
    assert_eq!(app.status_banner, "Deleted \"Work\"");
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    let st = app.store.read();
    let back = st.labels.iter().find(|l| l.name == "Work").expect("Undo makes the label again");
    assert!(back.muted && back.rooms == rooms);
    assert!(rooms.iter().all(|r| st.chat(r).label_ids.contains(&back.id)));
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
    assert_eq!(app.list.selected_account_filter.as_deref(), Some("label:lbl_work"));
    let _ = app.run_hotkey("Switch First 9 Spaces Modifier", 0);
    assert_eq!((app.list.selected_account_filter.as_deref(), app.list.active_tab), (None, InboxTab::Archived));
}

#[test]
fn edit_spaces_like_desktop() {
    let mut app = demo_app("edit-spaces");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("account:label:lbl_work".into())));
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
            sim.find("Label Settings").is_ok()
                && sim.find("Delete Label").is_err()
                && sim.find("Edit Spaces…").is_err()
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
    let _ = app.update(UiMessage::Labels(labels::Msg::Delete("lbl_work".into())));
    assert!(!app.store.read().labels.iter().any(|l| l.id == "lbl_work"));
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
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenCreate(vec!["!note_self:beeper.com".into()])));
    for e in [LabelEdit::Advance, LabelEdit::Advance, LabelEdit::Name("Friends".into()), LabelEdit::Advance] {
        let _ = app.update(UiMessage::Labels(labels::Msg::Change(e)));
    }
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(ids[0].clone(), ChatFlag::Archived)));
    assert_eq!(app.status_banner, "Archived");
    undo(&mut app);
    assert!(!flag(&app, &ids[0], ChatFlag::Archived) && app.status_banner.is_empty());
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(ids[0].as_str()), "Undo selects the chat");
    app.selection = ids.clone();
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectionSet(ChatFlag::LowPriority, true)));
    assert_eq!(app.status_banner, "Moved 2 chats to Low Priority");
    assert!(ids.iter().all(|id| flag(&app, id, ChatFlag::LowPriority)));
    undo(&mut app);
    assert!(ids.iter().all(|id| !flag(&app, id, ChatFlag::LowPriority)));
    assert_eq!(app.selection, ids, "Undo selects the chats again");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectionSet(ChatFlag::Archived, true)));
    assert!(app.status_banner == "Archived 2 chats" && app.toast.as_ref().unwrap().icon.is_none());
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(ids[0].clone(), ChatFlag::Archived)));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(ids[1].clone(), ChatFlag::LowPriority)));
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(ids[0].clone(), ChatFlag::Archived)));
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert!(!archived(&app) && app.older_toasts.len() == 1, "Mod-z closes the newest toast and keeps the others");
    let _ = app.update(UiMessage::Toast(toasts::Msg::Hover(true)));
    assert!(!app.toast_running(), "the toasts wait while the pointer is on them");
    let _ = app.update(UiMessage::Toast(toasts::Msg::Hover(false)));
    for _ in 0..TOAST_TIMEOUT.as_millis() / TOAST_TICK.as_millis() {
        let _ = app.update(UiMessage::Toast(toasts::Msg::Tick));
    }
    assert!(app.older_toasts.is_empty() && app.status_banner.is_empty(), "each toast closes after its timeout");
    let _ = app.update(UiMessage::Toast(toasts::Msg::DismissOlder(0)));
    assert!(app.older_toasts.is_empty(), "the second click of a double click on a close button closes nothing");
}

#[test]
fn sidebar_spaces_drag_like_desktop() {
    let mut app = demo_app("rail-drag");
    let hide = SpaceOrder { id: "requests".into(), visible: false };
    app.patch_settings(|s| s.spaces_order = vec![hide.clone()]);
    let press = |app: &mut BeeperGuiApp, moves: &[f32]| {
        let _ = app.update(UiMessage::Rail(rail::Msg::Hover("inbox".into(), Point::new(18.0, 18.0))));
        let _ = app.update(UiMessage::Rail(rail::Msg::Press));
        for &y in moves {
            let _ = app.update(UiMessage::Rail(rail::Msg::DragMoved(Point::new(34.0, y))));
        }
    };
    press(&mut app, &[32.0, 34.0]);
    let _ = app.update(UiMessage::Rail(rail::Msg::DragEnd));
    assert_eq!(app.store.read().settings.spaces_order, std::slice::from_ref(&hide), "a click does not sort");
    press(&mut app, &[32.0, 60.0]);
    let _ = app.update(UiMessage::Rail(rail::Msg::Hover("archive".into(), Point::new(18.0, 4.0))));
    let _ = app.view_rail();
    let _ = app.update(UiMessage::Rail(rail::Msg::DragEnd));
    let st = app.store.read();
    assert_eq!(rail_spaces(&st)[..2], ["archive", "inbox"]);
    assert_eq!(st.settings.spaces_order.last(), Some(&hide), "the hidden spaces stay hidden");
}

#[test]
fn create_label_shows_a_draft_space_like_desktop() {
    let mut app = demo_app("label-draft");
    let rail = |app: &mut BeeperGuiApp, name: &str| {
        let _ = app.update(UiMessage::Labels(labels::Msg::Change(LabelEdit::Name(name.into()))));
        let _ = app.view_rail();
        let d = app.label_window.as_ref().expect("Create Label is open");
        draft_rail_spaces(&app.store.read(), &d.name)
    };
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())));
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
fn accounts_settings_rows_have_the_account_menu_like_desktop() {
    let mut app = demo_app("prefs-account-menu");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("prefs:{account}"))));
    let ask: Vec<UiMessage> = {
        let mut sim = simulator(app.view_settings());
        for item in ["Show in Inbox", "Mute Space", "Edit Spaces…"] {
            assert!(sim.find(item).is_ok(), "{item}");
        }
        assert!(sim.find("Account Settings…").is_err() && sim.find("Add/Remove Accounts…").is_err());
        let _ = sim.click("Remove Account…").expect("a Remove Account item");
        sim.into_messages().collect()
    };
    assert!(matches!(&ask[..], [UiMessage::Accounts(accounts::Msg::AskRemove(a))] if *a == account));
    let _ = app.update(ask.into_iter().next().unwrap());
    assert!(app.list.chat_menu.is_none() && app.confirm == Some(ASK_REMOVE_ACCOUNT));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Disconnect and remove the data from this account on Beeper").is_ok());
    let _ = sim.click("Remove from All Devices").expect("the confirm button");
    assert!(
        matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::Accounts(accounts::Msg::Remove(a))] if *a == account)
    );
    let _ = app.update(UiMessage::Accounts(accounts::Msg::Removed(account.clone(), Err("offline".into()))));
    let text = format!("There was an error disconnecting account {account}. offline");
    assert!(simulator(app.view_main()).find(text.as_str()).is_ok());
    let _ = app.update(UiMessage::Ask(None));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("prefs:hungryserv".into())));
    let mut sim = simulator(app.view_settings());
    let _ = sim.click("Remove Account…").expect("Desktop's Remove Account item of the Beeper account");
    assert!(matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::ChatList(chat_list::Msg::CloseMenu)]));
}

/// The demo has no Beeper Cloud: Remove Account takes the account out of the demo without a request to the Beeper
/// API or the homeserver.
#[tokio::test(flavor = "multi_thread")]
async fn the_demo_removes_an_account_without_the_cloud() {
    let mut app = demo_app("demo-remove-account");
    // A closed port for the Beeper API and the homeserver, so that a request fails at once.
    {
        let mut st = app.store.write();
        (st.config.api_url, st.beeper_state.homeserver) = ("http://127.0.0.1:9".into(), "http://127.0.0.1:9".into());
    }
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    let task = app.update(UiMessage::Accounts(accounts::Msg::Remove(account.clone())));
    let sent =
        task_messages(task).await.into_iter().any(|m| matches!(m, UiMessage::Accounts(accounts::Msg::Removed(..))));
    assert!(!sent, "no request");
    assert!(app.store.read().bridge_accounts.iter().all(|b| b.account_id != account), "the account is gone");
}

/// Desktop's `AccountActions` of an account page: the chevron in the corner of the account card opens `rj` without
/// Account Settings…, with Edit Account Nickname… and Add/Remove Accounts….
#[test]
fn account_pages_have_the_account_actions_like_desktop() {
    let mut app = demo_app("account-page-actions");
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
    let _ = app.update(UiMessage::Accounts(accounts::Msg::Open(Some(account.clone()))));
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_settings());
        let at = sim.find(iced::widget::Id::new("account-actions")).expect("the actions of the page").bounds().center();
        sim.point_at(at);
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    };
    let want = format!("account-actions:{account}");
    assert!(matches!(&open[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(m))] if *m == want), "{:?}", open.len());
    let _ = app.update(open.into_iter().next().unwrap());
    let mut sim = simulator(app.view_settings());
    for item in ["Show in Inbox", "Mute Space", "Edit Spaces…", "Edit Account Nickname…", "Add/Remove Accounts…"]
    {
        assert!(sim.find(item).is_ok(), "{item}");
    }
    assert!(sim.find("Account Settings…").is_err() && sim.find("Remove Account…").is_err());
    let _ = sim.click("Edit Account Nickname…").expect("the nickname item");
    let nick = sim.into_messages().any(|m| matches!(m, UiMessage::Nickname(nickname::Msg::Open(a)) if a == account));
    assert!(nick, "the item edits the nickname of the account");
}

#[test]
fn sidebar_menus_open_accounts_like_desktop() {
    let mut app = demo_app("rail-menus");
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    for entry in [account.as_str(), "label:lbl_work"] {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("account:{entry}"))));
        let mut sim = simulator(app.view_account_menu(entry));
        assert!(entry.starts_with("label:") || sim.find("Mute Space").is_ok());
        let _ = sim.click("Add/Remove Accounts…").expect("an Add/Remove Accounts item");
        let open =
            sim.into_messages().find(|m| matches!(m, UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts))));
        let _ = app.update(open.expect("the Accounts settings"));
        assert!(app.list.chat_menu.is_none() && app.settings_window.is_some());
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("account:{account}"))));
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu(&account));
        let _ = sim.click("Account Settings…").expect("an Account Settings item");
        sim.into_messages().collect()
    };
    for m in open {
        let _ = app.update(m);
    }
    assert_eq!(app.account_page.as_deref(), Some(account.as_str()));
    assert!(app.list.chat_menu.is_none() && matches!(app.active_settings_tab, SettingsTab::Accounts));
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
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
    let _ = simulator(app.view_settings());
    let _ = simulator(app.view_rail());
    app.store.write().bridge_accounts[0].state = "CONNECTED".into();
    app.store.write().bridge_accounts[0].remote_error = Some("Token expired".into());
    assert_eq!(status_text(&app.store.read(), &account), Some("Attention Required"), "another state with an error");
    let _ = app.update(UiMessage::Accounts(accounts::Msg::Open(Some(account.clone()))));
    let mut sim = simulator(app.view_settings());
    assert!(sim.find("Account Disconnected").is_ok() && sim.find("Token expired").is_ok());
    assert!(sim.find("Reconnect").is_err(), "a demo account cannot log in again");
    let _ = sim.click("Remove Account").expect("the Remove Account button of the account page");
    for m in sim.into_messages() {
        let _ = app.update(m);
    }
    assert!(simulator(app.view_settings()).find("Remove from All Devices").is_ok(), "the button asks first");
    let _ = app.update(UiMessage::Ask(None));
    let _ = app.update(UiMessage::Accounts(accounts::Msg::Open(Some("hungryserv".into()))));
    assert!(simulator(app.view_settings()).find("Remove Account").is_err(), "the Beeper account has no button");
}

/// Desktop's `AppStatus`: a disconnected account shows its banner in the inbox too, with a dismiss button there, and
/// in its own space without one.
#[test]
fn disconnected_banners_like_desktop() {
    let mut app = demo_app("status-banners");
    let (account, name) = {
        let mut st = app.store.write();
        let b = &mut st.bridge_accounts[0];
        b.remote_error = Some("Token expired".into());
        (b.account_id.clone(), beeper_core::config::account_network(&b.bridge_id).display_name)
    };
    let banner = format!("{name} disconnected");
    assert!(simulator(app.view_sidebar()).find(banner.as_str()).is_ok(), "the inbox shows it");
    // `.app-status > div`: 8 px below the header, 40 px tall, its text in the middle.
    let text = {
        let st = app.store.read();
        simulator(app.view_network_status(&st).expect("a banner")).find(banner.as_str()).expect("the text").bounds()
    };
    assert!((text.center_y() - 28.0).abs() < 1.0, "the text is centered in the banner: {text:?}");
    let _ = app.update(UiMessage::Accounts(accounts::Msg::DismissError(account.clone())));
    assert!(simulator(app.view_sidebar()).find(banner.as_str()).is_err(), "dismissed for the session");
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some(account))));
    assert!(simulator(app.view_sidebar()).find(banner.as_str()).is_err(), "a dismissed account stays dismissed");
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
    assert_eq!(app.list.chat_menu.as_ref().map(|m| m.0.as_str()), Some("account:inbox"));
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
    assert!(!app.store.read().settings.on("showSidebar") && app.list.chat_menu.is_none());
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
        app.list.chat_menu.as_ref().map(|m| m.0.clone())
    };
    assert_eq!(right_click(&mut app, Point::new(62.0, 300.0)).as_deref(), Some(RAIL_MENU), "the space bar");
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
    assert!(app.creating_chat.is_some() && app.list.chat_menu.is_none());
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
    assert!(app.select_mode && app.selection.is_empty() && app.list.chat_menu.is_none());
    let first = app.list_ids(&app.store.read())[0].clone();
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleSelected(first.clone())));
    assert!(!app.select_mode && app.selection == [first.clone()]);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleSelected(first)));
    assert!(app.selection.is_empty() && !app.select_mode, "the mode ends with the last chat");
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("label:lbl_work".into()))));
    let has = |app: &BeeperGuiApp, key: &str, items: &[&str]| {
        let mut sim = simulator(app.view_chat_menu(key, false, 900.0));
        items.iter().all(|i| sim.find(*i).is_ok())
    };
    assert!(has(&app, "head:label:lbl_work", &["Work", "Mute", "Label Settings", "Select Chats"]));
    assert!(
        !has(&app, "head:label:lbl_work", &["Show in Inbox"]) && !has(&app, "head:label:lbl_work", &["Delete Label"])
    );
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    assert!(has(&app, &format!("head:{account}"), &["Select Chats", "Show in Inbox", "Account Settings..."]));
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(None)));

    let chat = app.store.read().chats[0].id.clone();
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Click(chat.clone())));
    let at = Point::new(68.0 + SIDEBAR_WIDTH + 40.0, 20.0);
    assert_eq!(right_click(&mut app, at), Some(chat.clone()), "the chat header has the chat menu");
    let _ = app.update(UiMessage::CloseModals);
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(chat)));
    assert!(simulator(app.view_main()).find("Begin Selecting").is_ok(), "the chat menu shows over the command bar");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::CloseMenu));
    assert!(app.show_command_palette && app.list.chat_menu.is_none());
}

#[test]
fn space_sheet_like_desktop() {
    let mut app = demo_app("space-sheet");
    let main = app.main_window.expect("a main window");
    let clicks = [mouse::Event::ButtonPressed(Button::Left), mouse::Event::ButtonReleased(Button::Left)];
    let title = Point::new(100.0, 26.0);
    for m in interact(&app, main, Size::new(1440.0, 900.0), title, clicks.map(iced::Event::Mouse).to_vec()) {
        let _ = app.update(m);
    }
    assert_eq!(
        app.list.chat_menu.as_ref().map(|m| m.0.as_str()),
        Some(SPACE_SHEET),
        "the header title opens the sheet"
    );
    let pick = |app: &mut BeeperGuiApp, item: &str, has: &[&str], lacks: &[&str]| {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(SPACE_SHEET.into())));
        let msgs: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            for i in has {
                assert!(sim.find(*i).is_ok(), "{i}");
            }
            for i in lacks {
                assert!(sim.find(*i).is_err(), "{i}");
            }
            let _ = sim.click(item).expect(item);
            sim.into_messages().collect()
        };
        for m in msgs {
            let _ = app.update(m);
        }
    };
    let actions = ["Select Chats", "New Label", "Edit Spaces...", "Add/Remove Accounts..."];
    pick(&mut app, "Work", &[&["Inbox", "Archive"][..], &actions].concat(), &["Space Options", "Label Settings"]);
    assert_eq!(app.list.selected_account_filter.as_deref(), Some("label:lbl_work"));
    assert!(app.list.chat_menu.is_none());
    pick(&mut app, "Label Settings", &["Mute"], &["Space Options", "Delete Label"]);
    assert!(app.label_settings.iter().any(|w| w.label == "lbl_work"));
    let _ = app.update(UiMessage::CloseModals);
    let account = app.store.read().bridge_accounts[0].account_id.clone();
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some(account))));
    pick(&mut app, "Space Options", &actions, &["Label Settings", "Account Settings..."]);
    assert_eq!(app.chat_sub, Some(1), "Space Options opens its submenu");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(SPACE_SHEET.into())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(1))));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Show in Inbox").is_ok() && sim.find("Account Settings...").is_ok());
}

#[test]
fn label_settings_like_desktop() {
    let mut app = demo_app("label-settings");
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("label:lbl_work".into()))));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(SPACE_SHEET.into())));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("Label Settings").expect("Label Settings");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    assert_eq!(app.label_settings.len(), 1, "the sheet opens Label Settings");
    let w = app.label_settings[0].id;
    let run = |app: &mut BeeperGuiApp, item: &str| {
        let picked: Vec<UiMessage> = {
            let mut sim = simulator(app.view_window(w));
            let _ = sim.click(item).expect(item);
            sim.into_messages().collect()
        };
        for m in picked {
            let _ = app.update(m);
        }
    };
    {
        let mut sim = simulator(app.view_window(w));
        for item in ["Work", "Edit", "Chats", "Show in Inbox", "Mute", "Delete Label", "Done"] {
            assert!(sim.find(item).is_ok(), "{item}");
        }
    }
    run(&mut app, "All Messages");
    assert!(app.label_settings[0].menu, "the pill opens the Show in Inbox menu");
    run(&mut app, "Only DMs");
    let inbox = |app: &BeeperGuiApp| app.store.read().labels.iter().find(|l| l.id == "lbl_work").map(|l| l.inbox);
    assert_eq!(inbox(&app), Some(Some("dmOnly")));
    assert!(!app.label_settings[0].menu);
    run(&mut app, "Delete Label");
    assert!(app.label_asking(w), "the confirm shows in the Label Settings window");
    let ask = "Are you sure you want to delete \"Work\"? This will remove the label from all chats.";
    assert!(simulator(app.view_main()).find(ask).is_err());
    run(&mut app, "Delete");
    assert_eq!(inbox(&app), None, "Delete deletes the label");
}

#[test]
fn chat_list_mutes_receipts_and_errors_like_desktop() {
    let mut app = demo_app("list-states");
    let (id, account) = {
        let st = app.store.read();
        let free = |c: &&Chat| {
            let bridged = st.bridge_accounts.iter().any(|b| b.account_id == c.account_id);
            bridged && c.is_muted != Some(true) && c.label_ids.is_empty() && c.flags & CHAT_ARCHIVED == 0
        };
        let c = st.chats.iter().find(free).expect("an unmuted chat");
        (c.id.clone(), c.account_id.clone())
    };
    let state = |app: &BeeperGuiApp| {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(&id).expect("the chat")];
        (app.list_muted(&st, c), app.list_quiet(&st, c))
    };
    assert_eq!(state(&app), (false, false));
    let props = app.store.read().settings.account(&account);
    let _ = app.update(UiMessage::Accounts(accounts::Msg::SetProps(
        account.clone(),
        beeper_core::models::AccountProps { notifications: false, ..props },
    )));
    assert_eq!(state(&app), (true, true), "a muted account mutes its rows");
    let idx = app.store.read().resolve_chat_idx(&id).expect("the chat");
    app.store.write().update_chat(idx, |c| c.unread_mentions = 1);
    assert_eq!(state(&app), (true, false), "a mention keeps the count blue");
    let _ = app.update(UiMessage::Accounts(accounts::Msg::SetProps(account.clone(), props)));
    app.store.write().update_chat(idx, |c| c.is_archived = Some(true));
    assert_eq!(state(&app), (false, true), "archived rows are grey");

    let st = app.store.read();
    let has_messages = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
    let mut c = st.chats.iter().filter(has_messages).find(|c| c.chat_type == "group").expect("a group").clone();
    let extra = User { id: "@extra:beeper.local".into(), ..c.participants.items[0].clone() };
    c.participants.items.push(extra);
    let me = st.me().id.clone();
    let others: Vec<String> =
        c.participants.items.iter().filter(|u| !st.is_me(&u.id) && u.id != me).map(|u| u.id.clone()).collect();
    let mut m = st.messages[st.timeline_indices(&c.id)[0]].clone();
    (m.id, m.sender_id, m.seen) = ("$event".into(), me, Default::default());
    assert_eq!(receipt_tip(&st, &c, &m).as_deref(), Some("Sent"));
    let own = Message { is_sender: Some(true), ..m.clone() };
    assert!(chat_list::read_icon(&own), "an own message shows its read icon");
    let left = Message { message_type: Some("MEMBERSHIP".into()), text: Some("You left the chat".into()), ..own };
    assert!(!chat_list::read_icon(&left), "Desktop's `getReadIcon` shows none for an action");
    m.seen.insert(others[0].clone(), String::new());
    assert_eq!(receipt_tip(&st, &c, &m), Some(format!("Seen by 1 of {}", others.len())));
    m.id = "msg_1_1".into();
    assert!(!pending(&st, &m), "the demo has no homeserver");
    drop(st);

    assert_eq!(space_error(&app.store.read(), &account), None);
    app.store
        .write()
        .bridge_accounts
        .iter_mut()
        .filter(|b| b.account_id == account)
        .for_each(|b| b.state = "UNKNOWN_ERROR".into());
    assert_eq!(space_error(&app.store.read(), &account), Some("Disconnected"), "the rail shows the error badge");
    assert_eq!(space_error(&app.store.read(), "label:lbl_work"), None);
    assert_eq!([unread_label(999), unread_label(1000)], ["999", "999+"]);
}

#[test]
fn another_chat_drops_the_pending_reveal() {
    let mut app = demo_app("pending-reveal");
    let chats: Vec<String> = {
        let st = app.store.read();
        let has_messages = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
        st.chats.iter().filter(has_messages).take(2).map(|c| c.id.clone()).collect()
    };
    let _ = app.update(UiMessage::SelectChat(chats[0].clone()));
    let _ = app.update(UiMessage::Message(message::Msg::Step(false, false)));
    assert!(!app.pane.selected_msgs.is_empty(), "a step selects and reveals a message");
    let _ = app.update(UiMessage::SelectChat(chats[1].clone()));
    drop(simulator(app.view_conversation()));
}

#[test]
fn pinned_grid_and_range_selection_like_desktop() {
    let cols = [(232.0, 1), (500.0, 3), (232.0, 4), (1000.0, 7), (232.0, 0)].map(|(w, n)| pin_columns(w, n));
    assert_eq!(cols, [1, 3, 2, 4, 1], "at most 6 columns of 80px, balanced over the rows");
    let mut app = demo_app("pins-range");
    let pinned = app.store.read().chats.iter().find(|c| c.flags & CHAT_PINNED != 0).expect("a pin").id.clone();
    {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&pinned).expect("the pin");
        st.set_chat_flag(idx, ChatFlag::LowPriority, Some(true));
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::LowPriority)));
    assert!(app.pins_and_rows(&app.store.read()).0.iter().any(|c| c.id == pinned), "Low Priority has the grid");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Reminders)));
    assert!(app.pins_and_rows(&app.store.read()).0.is_empty(), "Reminders has no grid");
    {
        let mut sim = simulator(app.view_filter_menu());
        assert!(sim.find("Requests").is_ok());
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Archived)));
    {
        let mut sim = simulator(app.view_filter_menu());
        assert!(sim.find("Low Priority").is_ok() && sim.find("Requests").is_err() && sim.find("Archived").is_err());
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::AllChats)));
    let list = app.list_ids(&app.store.read());
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Click(list[2].clone())));
    app.mods = keyboard::Modifiers::SHIFT;
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Click(list[5].clone())));
    assert_eq!(app.selection, list[2..=5], "Shift-click selects the range from the open chat");
    app.mods = keyboard::Modifiers::CTRL;
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Click(list[7].clone())));
    assert!(app.selection.len() == 5 && app.selection.contains(&list[7]), "Ctrl-click adds a chat");
}

#[test]
fn hidden_space_bar_floats_like_desktop() {
    let mut app = demo_app("float-rail");
    let main = app.main_window.expect("a main window");
    assert_eq!(app.list_width(&app.store.read()), SIDEBAR_WIDTH);
    let _ = app.update(UiMessage::RunHotkey("Toggle Space Bar"));
    assert!(!app.store.read().settings.on("showSidebar"));
    assert_eq!(app.list_width(&app.store.read()), SIDEBAR_WIDTH + 68.0, "the list takes the width of the space bar");
    let hover = |app: &BeeperGuiApp, xs: &[f32]| {
        let moved =
            xs.iter().map(|&x| iced::Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, 300.0) }));
        interact(app, main, Size::new(1440.0, 900.0), Point::new(xs[0], 300.0), moved.collect())
    };
    assert!(!hover(&app, &[100.0]).iter().any(|m| matches!(m, UiMessage::Rail(rail::Msg::Float(true)))));
    let msgs = hover(&app, &[5.0]);
    assert!(
        msgs.iter().any(|m| matches!(m, UiMessage::Rail(rail::Msg::Float(true)))),
        "the left edge slides the space bar in"
    );
    let _ = app.update(UiMessage::Rail(rail::Msg::Float(true)));
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(main));
    for x in [30.0, 200.0] {
        sim.point_at(Point::new(x, 300.0));
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, 300.0) })]);
    }
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    assert!(
        msgs.iter().any(|m| matches!(m, UiMessage::Rail(rail::Msg::Float(false)))),
        "leaving the space bar hides it"
    );
    let account = {
        let mut st = app.store.write();
        let kind = |a: &str| account_network(a).kind;
        let other =
            |b: &&BridgeAccountInfo| !matches!(kind(&b.account_id), NetworkKind::WhatsApp | NetworkKind::Telegram);
        let account = st.bridge_accounts.iter().find(other).expect("a network with spaces").account_id.clone();
        st.network_spaces.push(NetworkSpace {
            id: "!acme:beeper.local".into(),
            account_id: account.clone(),
            name: "Acme".into(),
            avatar: Some("mxc://beeper.local/acme".into()),
        });
        account
    };
    // Desktop's `JO`: a network space with its own picture shows it in the space bar.
    drop(simulator(app.view_rail()));
    assert!(app.wants.avatars.borrow().contains("mxc://beeper.local/acme"), "the space bar loads the space's picture");
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("!acme:beeper.local".into()))));
    let title = format!("Acme ({})", network_label(&account));
    assert!(
        simulator(app.view_sidebar()).find(title.as_str()).is_ok(),
        "the header names the network space and its network"
    );
}

#[test]
fn space_view_account_filter_like_desktop() {
    let mut app = demo_app("space-view-account");
    let pick = |account: &str| {
        let st = app.store.read();
        let free = |c: &&Chat| c.id != NOTE_CHAT_ID && c.is_pinned != Some(true) && c.account_id == account;
        st.chats.iter().find(free).expect("a chat of the account").id.clone()
    };
    let (signal, whatsapp) = (pick("acc_signal"), pick("acc_whatsapp"));
    for id in [&signal, &whatsapp] {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(id).expect("a chat");
        st.update_chat(idx, |c| (c.is_request, c.flags) = (Some(true), c.flags | CHAT_REQUEST));
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Requests)));
    {
        // Requests lists the accounts whose bridge takes requests.
        let mut sim = simulator(app.view_filter_menu());
        assert!(sim.find("Signal").is_ok() && sim.find("WhatsApp").is_err() && sim.find("Low Priority").is_err());
    }
    let ids =
        |app: &BeeperGuiApp| app.visible_chats(&app.store.read()).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
    assert!(ids(&app).contains(&signal) && ids(&app).contains(&whatsapp));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::PickAccount("acc_signal".into())));
    assert_eq!(ids(&app), std::slice::from_ref(&signal), "the account shows only its requests");
    {
        let mut sim = simulator(app.view_sidebar());
        assert!(sim.find("Signal").is_ok(), "the header names the account under the space");
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::PickAccount("acc_signal".into())));
    assert!(app.list.space_view_account.is_none(), "picking the account again shows every account");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::PickAccount("acc_signal".into())));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::FilterButton));
    assert!(
        app.list.space_view_account.is_none() && app.list.filters == 0,
        "the active filter button clears the filters"
    );
    // Archive lists every account, and a space change drops the account.
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Archived)));
    {
        let mut sim = simulator(app.view_filter_menu());
        assert!(sim.find("Signal").is_ok() && sim.find("WhatsApp").is_ok() && sim.find("Low Priority").is_ok());
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::PickAccount("acc_whatsapp".into())));
    assert!(app.visible_chats(&app.store.read()).iter().all(|c| c.account_id == "acc_whatsapp"));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::LowPriority)));
    assert!(app.list.space_view_account.is_none(), "another space drops the account");
    // An account space without requests has no Requests filter in its menu.
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("acc_whatsapp".into()))));
    {
        let mut sim = simulator(app.view_filter_menu());
        assert!(sim.find("Requests").is_err() && sim.find("Archived").is_ok() && sim.find("Signal").is_err());
    }
    let _ = app.update(UiMessage::Rail(rail::Msg::Select(Some("acc_signal".into()))));
    assert!(simulator(app.view_filter_menu()).find("Requests").is_ok());
}

#[test]
fn request_rows_like_desktop() {
    let mut app = demo_app("request-rows");
    let (dm, group) = {
        let st = app.store.read();
        let free = |c: &&Chat| c.id != NOTE_CHAT_ID && c.is_pinned != Some(true) && c.flags & CHAT_ARCHIVED == 0;
        let dm = st.chats.iter().filter(free).find(|c| c.chat_type == "single").expect("a DM").id.clone();
        let group = st.chats.iter().filter(free).find(|c| c.chat_type == "group").expect("a group").id.clone();
        (dm, group)
    };
    for id in [&dm, &group] {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(id).expect("a chat");
        st.update_chat(idx, |c| (c.is_request, c.flags) = (Some(true), c.flags | CHAT_REQUEST));
    }
    app.patch_settings(|s| {
        s.prefs.insert("showMessagePreviews".into(), 0);
    });
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Requests)));
    let mut sim = simulator(app.view_sidebar());
    assert!(sim.find("Message Request").is_ok() && sim.find("Group Invitation").is_ok());
    // Desktop names the inviter of a group invite: the member who added the own pending member.
    let inviter = {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&group).expect("the group");
        let inviter =
            st.chats[idx].participants.items.iter().find(|u| u.is_self != Some(true)).expect("a member").clone();
        let me =
            User { id: "@me:beeper.local".into(), is_self: Some(true), is_pending: Some(true), ..Default::default() };
        let me = User { added_by: Some(inviter.id.clone()), ..me };
        st.update_chat(idx, |c| {
            c.participants.items.retain(|u| u.is_self != Some(true));
            c.participants.items.push(me);
        });
        inviter.full_name.expect("a name")
    };
    let invited = format!("Invited by {inviter}");
    assert!(simulator(app.view_sidebar()).find(invited.as_str()).is_ok(), "{invited}");
}

/// Desktop's `InviteDetailsPane`: an invite to a Matrix room shows who invited the user to which room, with Decline
/// and Accept, in place of the chat.
#[test]
fn matrix_invites_show_their_details_like_desktop() {
    let mut app = demo_app("invite-details");
    let (group, inviter) = {
        let mut st = app.store.write();
        let free = |c: &&Chat| c.id != NOTE_CHAT_ID && c.chat_type == "group" && c.flags & CHAT_ARCHIVED == 0;
        let group = st.chats.iter().find(free).expect("a group").id.clone();
        let idx = st.resolve_chat_idx(&group).expect("the group");
        let inviter = st.chats[idx].participants.items.iter().find(|u| u.is_self != Some(true)).unwrap().clone();
        let me = User {
            id: "@me:beeper.local".into(),
            is_self: Some(true),
            added_by: Some(inviter.id.clone()),
            ..Default::default()
        };
        st.update_chat(idx, |c| {
            (c.is_request, c.can_accept_request, c.account_id) = (Some(true), true, "hungryserv".into());
            c.participants.items.retain(|u| u.is_self != Some(true));
            c.participants.items.push(me);
        });
        (group, inviter)
    };
    let _ = app.update(UiMessage::SelectChat(group.clone()));
    let title = app.store.read().chat(&group).title.clone();
    let mut sim = simulator(app.view_conversation());
    for text in
        [inviter.full_name.clone().unwrap(), format!("({})", inviter.id), "invited you to the group:".into(), title]
    {
        assert!(sim.find(text.as_str()).is_ok(), "{text}");
    }
    let _ = sim.click("Accept").expect("Accept");
    assert!(sim
        .into_messages()
        .any(|m| matches!(m, UiMessage::ChatList(chat_list::Msg::AcceptRequest(id)) if id == group)));
}

#[test]
fn label_menu_labels_the_selection_like_desktop() {
    let mut app = demo_app("label-selection");
    let ids: Vec<String> = {
        let st = app.store.read();
        let free = |c: &&Chat| c.label_ids.is_empty() && c.id != NOTE_CHAT_ID && c.flags & CHAT_ARCHIVED == 0;
        st.chats.iter().filter(free).take(2).map(|c| c.id.clone()).collect()
    };
    for id in &ids {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleSelected(id.clone())));
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(ids[0].clone())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(2))));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(&ids[0], false, 900.0));
        let _ = sim.click("Work").expect("the Work label");
        sim.into_messages().collect()
    };
    assert!(matches!(&picks[..], [UiMessage::Labels(labels::Msg::Toggle(l, c, true))] if l == "lbl_work" && *c == ids));
    for m in picks {
        let _ = app.update(m);
    }
    let labeled = |app: &BeeperGuiApp| {
        let st = app.store.read();
        ids.iter().filter(|id| st.chat(id).label_ids.iter().any(|l| l == "lbl_work")).count()
    };
    assert_eq!(labeled(&app), 2, "the label goes on every selected chat");
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert_eq!(labeled(&app), 0);
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
        assert!(c.is_request == Some(false) && c.flags & CHAT_REQUEST == 0 && app.list.chat_menu.is_none());
    }
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::Click(second.clone())));
    let heading = {
        let st = app.store.read();
        let c = &st.chats[st.resolve_chat_idx(second).expect("a request")];
        let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).expect("the other person");
        let id = handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone());
        id.map_or(member_name(u), |id| format!("{} ({id})", member_name(u)))
    };
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_conversation());
        // Desktop's `RequestNotice` names the other person in a card trigger inside the heading.
        assert!(sim.find("Accept message request from ").is_ok() && sim.find(heading.as_str()).is_ok(), "{heading}");
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
fn request_head_like_desktop() {
    let mut app = demo_app("request-head");
    let (id, url, username) = {
        let mut st = app.store.write();
        let i = st.chats.iter().position(|c| profile_link(c).is_some()).expect("a chat with a profile");
        st.update_chat(i, |c| (c.is_request, c.can_accept_request) = (Some(true), true));
        let c = &st.chats[i];
        let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).expect("the other person");
        (c.id.clone(), profile_link(c).expect("a profile"), handle(u).expect("a username"))
    };
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_conversation());
        assert!(sim.find(username.as_str()).is_ok(), "{username}");
        let _ = sim.click("View Profile").expect("View Profile");
        sim.into_messages().collect()
    };
    assert!(matches!(&picked[..], [UiMessage::OpenUrl(u)] if *u == url));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::AcceptRequest(id)));
    assert!(simulator(app.view_conversation()).find("View Profile").is_err(), "only requests have the header");
}

#[test]
fn blocking_like_desktop() {
    let mut app = demo_app("blocking");
    let (id, title) = {
        let mut st = app.store.write();
        st.features.flags["features"]["*"]["block-users"] = true.into();
        let dm = |c: &Chat| c.chat_type == "single" && c.id != NOTE_CHAT_ID && c.is_pinned != Some(true);
        let i = st.chats.iter().position(dm).expect("a DM");
        st.update_chat(i, |c| {
            (c.is_request, c.can_accept_request, c.block) = (Some(true), true, BLOCK_USER | REPORT_SPAM)
        });
        (st.chats[i].id.clone(), st.chats[i].title.clone())
    };
    let clicks = |view: El<'_>, labels: &[&str]| -> Vec<UiMessage> {
        let mut sim = simulator(view);
        for label in labels {
            let _ = sim.click(*label).unwrap_or_else(|_| panic!("{label}"));
        }
        sim.into_messages().collect()
    };
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let block = app.commands(&app.store.read(), "spam").into_iter().find(|c| c.title == "Block").expect("Block");
    assert!(matches!(block.msg, Some(UiMessage::Blocking(blocking::Msg::Ask(c, true))) if c == id));
    let picked = clicks(app.view_chat_menu(&id, false, 900.0), &["Block…"]);
    assert!(matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::Ask(c, true))] if *c == id));
    let picked = clicks(app.view_conversation(), &["Block"]);
    assert!(
        matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::Ask(c, true))] if *c == id),
        "the request notice has Block"
    );
    let _ = app.update(UiMessage::Blocking(blocking::Msg::Ask(id.clone(), true)));
    {
        let mut sim = simulator(app.view_confirm(ASK_BLOCK));
        assert!(sim.find(format!("Block {title}?").as_str()).is_ok());
        let _ = sim.click("Report spam").expect("Report spam");
        for m in sim.into_messages().collect::<Vec<_>>() {
            let _ = app.update(m);
        }
    }
    for m in clicks(app.view_confirm(ASK_BLOCK), &["Block"]) {
        let _ = app.update(m);
    }
    let blocked = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.user_blocked(st.chat(&id))
    };
    assert!(blocked(&app) && app.confirm.is_none());
    assert_eq!(app.status_banner, format!("{title} blocked"));
    assert!(app.toast.as_ref().is_some_and(|t| t.right.is_none()), "a spam report cannot be undone");
    assert!(app.pane.selected_chat_id.is_none(), "a blocked request closes");
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    {
        let mut conversation = simulator(app.view_conversation());
        assert!(conversation.find(format!("You blocked {title}. ").as_str()).is_ok());
        assert!(conversation.find("Block").is_err(), "the request notice drops Block");
    }
    let picked = clicks(app.view_conversation(), &["Unblock"]);
    assert!(
        matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::Ask(c, false))] if *c == id),
        "the composer link unblocks"
    );
    let picked = {
        let mut sim = tall(app.view_info());
        let _ = sim.click("Unblock").expect("Unblock");
        sim.into_messages().collect::<Vec<_>>()
    };
    assert!(matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::Ask(c, false))] if *c == id));
    let _ = app.update(UiMessage::Blocking(blocking::Msg::Ask(id.clone(), false)));
    for m in clicks(app.view_confirm(ASK_UNBLOCK), &["Unblock"]) {
        let _ = app.update(m);
    }
    assert!(!blocked(&app) && app.status_banner == format!("{title} unblocked"));
    let _ = app.update(UiMessage::UndoAction);
    assert!(blocked(&app), "Undo blocks again");
    let network = {
        let mut st = app.store.write();
        let i = st.chat_idx(&id);
        st.update_chat(i, |c| c.block |= NO_UNBLOCK);
        network_name(&st.chats[i].account_id)
    };
    let notice = format!("You blocked {title}. Unblock them in {network}.");
    assert!(simulator(app.view_conversation()).find(notice.as_str()).is_ok());
    assert!(simulator(app.view_chat_menu(&id, false, 900.0)).find("Unblock…").is_err());
}

/// Desktop shows "Delete Chat" in the chat menu and "Delete" in the command bar only with the delete-chats flag.
#[test]
fn delete_chat_follows_the_flag_like_desktop() {
    let mut app = demo_app("delete-chats");
    let id = {
        let mut st = app.store.write();
        let i = st.chats.iter().position(|c| c.id != NOTE_CHAT_ID).expect("a chat");
        st.update_chat(i, |c| c.delete_modes = DELETE_FOR_ME);
        st.chats[i].id.clone()
    };
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let shown = |app: &BeeperGuiApp| {
        let menu = simulator(app.view_chat_menu(&id, false, 900.0)).find("Delete Chat").is_ok();
        (menu, app.commands(&app.store.read(), "delete").iter().any(|c| c.title == "Delete"))
    };
    assert_eq!(shown(&app), (true, true));
    app.store.write().features.flags["features"]["*"]["delete-chats"] = false.into();
    assert_eq!(shown(&app), (false, false));
}

/// Desktop's `confirmDeleteThread` with chats selected: "Delete Chats" with only the delete action that all of them
/// take, which deletes them all, or a toast when they share none.
#[test]
fn deleting_selected_chats_like_desktop() {
    let mut app = demo_app("delete-selected");
    let ids: Vec<String> = {
        let mut st = app.store.write();
        let others = st.chats.iter().enumerate().filter(|(_, c)| c.id != NOTE_CHAT_ID).map(|(i, _)| i);
        let picked: Vec<usize> = others.take(4).collect();
        let modes = [DELETE_FOR_ME | DELETE_FOR_EVERYONE, DELETE_FOR_EVERYONE, DELETE_FOR_ME, DELETE_FOR_EVERYONE];
        for (&i, m) in picked.iter().zip(modes) {
            st.update_chat(i, |c| c.delete_modes = m);
        }
        picked.iter().map(|&i| st.chats[i].id.clone()).collect()
    };
    let select = |app: &mut BeeperGuiApp, a: &str, b: &str| {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::BeginSelecting(a.into())));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleSelected(b.into())));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::AskDelete(a.into())));
    };
    select(&mut app, &ids[0], &ids[1]);
    assert_eq!(app.confirm, Some(ASK_DELETE_CHAT));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_confirm(ASK_DELETE_CHAT));
        assert!(sim.find("Delete Chats").is_ok());
        assert!(sim.find("This action will permanently delete these chats for all participants.").is_ok());
        assert!(sim.find("Delete for Me").is_err(), "only the action that both chats take");
        let _ = sim.click("Delete for Everyone").expect("Delete for Everyone");
        sim.into_messages().collect()
    };
    for m in picked {
        let _ = app.update(m);
    }
    let gone = |app: &BeeperGuiApp, id: &str| app.store.read().resolve_chat_idx(id).is_none();
    assert!(gone(&app, &ids[0]) && gone(&app, &ids[1]) && app.selection.is_empty(), "both chats are deleted");
    select(&mut app, &ids[2], &ids[3]);
    assert_eq!(app.status_banner, "Selected chats do not support the same delete action");
    assert!(!gone(&app, &ids[2]) && !gone(&app, &ids[3]));
}

/// Desktop's thread header (`ij`) returns nothing in `threadSelectionMode`, which selecting chats turns on.
#[test]
fn selecting_chats_hides_the_thread_header_like_desktop() {
    let mut app = demo_app("selection-header");
    let ids: Vec<String> = app.visible_chats(&app.store.read()).iter().take(2).map(|c| c.id.clone()).collect();
    let _ = app.update(UiMessage::SelectChat(ids[0].clone()));
    let header = |app: &BeeperGuiApp| simulator(app.view_conversation()).find("Feedback?").is_ok();
    assert!(header(&app), "the header of the open chat");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::BeginSelecting(ids[0].clone())));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleSelected(ids[1].clone())));
    assert!(!header(&app), "no header while chats are selected");
}

#[test]
fn blocked_users_like_desktop() {
    let mut app = demo_app("blocked-users");
    let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::General)));
    assert!(simulator(app.view_settings()).find("Blocked users").is_err(), "the block-users flag is off");
    app.store.write().features.flags["features"]["*"]["block-users"] = true.into();
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_settings());
        let _ = sim.click("Blocked users").expect("the Blocked users row");
        sim.into_messages().collect()
    };
    assert!(matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::OpenList)]));
    let _ = app.update(UiMessage::Blocking(blocking::Msg::OpenList));
    assert!(app.blocked_window.is_some());
    assert!(simulator(app.view_blocked_users()).find("No Blocked Users").is_ok());
    let (id, title) = {
        let mut st = app.store.write();
        let i = st.chats.iter().position(|c| c.chat_type == "single" && c.id != NOTE_CHAT_ID).expect("a DM");
        st.update_chat(i, |c| c.block = BLOCK_USER | USER_BLOCKED);
        (st.chats[i].id.clone(), st.chats[i].title.clone())
    };
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_blocked_users());
        let _ = sim.click(title.as_str()).expect("the blocked person");
        sim.into_messages().collect()
    };
    assert!(matches!(&picked[..], [UiMessage::Blocking(blocking::Msg::OpenUser(c))] if *c == id));
    let _ = app.update(UiMessage::Blocking(blocking::Msg::OpenUser(id.clone())));
    assert!(app.pane.selected_chat_id.as_deref() == Some(id.as_str()) && app.show_right_panel);
    assert!(simulator(app.view_settings()).find("1").is_ok(), "the row counts the blocked people");
}

#[test]
fn sidebar_overflow_menu_lists_hidden_spaces_like_desktop() {
    let mut app = demo_app("overflow-menu");
    let hide = |id: &str| SpaceOrder { id: id.into(), visible: false };
    app.patch_settings(|s| s.spaces_order = vec![hide("requests"), hide("label:lbl_work")]);
    let unread = app.store.read().chats.iter().any(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(RAIL_MORE.into())));
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
    assert_eq!(app.list.selected_account_filter.as_deref(), Some("label:lbl_work"));
    assert!(app.list.chat_menu.is_none());
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(read.clone())));
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
    assert!(app.list.chat_menu.is_none() && app.selection.is_empty());
}

/// Desktop's menus and tooltips show the `humanized` keys of a hotkey that you reassigned or turned off.
#[test]
fn menus_show_the_reassigned_hotkeys_like_desktop() {
    let mut app = demo_app("menu-hotkeys");
    let id = app.visible_chats(&app.store.read()).iter().find(|c| c.is_pinned != Some(true)).unwrap().id.clone();
    app.patch_settings(|s| {
        s.keybindings.insert("Chat: Toggle Archive".into(), "ctrl+alt+e, e".into());
        s.keybindings.insert("Chat: Toggle Pin".into(), "disabled".into());
    });
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
    let mut sim = simulator(app.view_chat_menu(&id, false, 900.0));
    assert!(sim.find("Ctrl Alt E").is_ok() && sim.find("Ctrl E").is_err(), "the reassigned Archive keys");
    assert!(sim.find("Ctrl P").is_err(), "no keys for the disabled Pin");
    assert!(sim.find("Ctrl Shift M").is_ok(), "the default Mute keys");
    drop(sim);
    assert_eq!(hotkey_caps("ctrl+shift+]"), ["Ctrl", "Shift", "]"]);
    assert_eq!(hotkey_caps("ctrl+alt+3"), ["Ctrl", "Alt", "3"]);
    assert!(hotkey_caps("disabled").is_empty());
    // The texts of the settings name the keys of the hotkeys, like Desktop's `MTt` with `TOGGLE_THREAD_READ`.
    app.patch_settings(|s| drop(s.keybindings.insert("Chat: Toggle Mark As Read".into(), "ctrl+alt+u".into())));
    let hits = search_settings(&app.store.read(), "hit Ctrl Alt U to mark");
    assert!(hits.iter().any(|h| matches!(h, SettingsHit::Pref(_, r) if r.key == "incognito")), "{hits:?}");
    assert!(search_settings(&app.store.read(), "hit Ctrl Shift U").iter().all(|h| !matches!(h, SettingsHit::Pref(..))));
}

#[test]
fn label_inbox_rules_like_desktop() {
    let mut app = demo_app("label-inbox");
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenSettings("lbl_work".into())));
    let w = app.label_settings[0].id;
    let _ = app.update(UiMessage::Labels(labels::Msg::Pane(w, LabelPane::Menu(true))));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_window(w));
        let _ = sim.click("Only DMs").expect("the Only DMs option");
        sim.into_messages().collect()
    };
    assert!(
        matches!(&picks[..], [UiMessage::Commands(m)] if matches!(&m[1], UiMessage::Labels(labels::Msg::SetInbox(id, Some("dmOnly"))) if id == "lbl_work"))
    );
    let hidden = |app: &BeeperGuiApp| {
        let st = app.store.read();
        let work = st.labels.iter().find(|l| l.id == "lbl_work").expect("the work label");
        let chats = work.rooms.iter().map(|r| st.chat(r));
        chats.map(|c| (c.inbox_group, c.flags & CHAT_HIDDEN != 0)).collect::<Vec<_>>()
    };
    let shown = hidden(&app);
    assert!(!shown.is_empty() && shown.iter().all(|&(_, h)| !h));
    for m in picks {
        let _ = app.update(m);
    }
    assert!(hidden(&app).iter().all(|&(group, h)| group == h), "Only DMs hides group chats");
    assert!(app.list.chat_menu.is_none());
    let _ = app.update(UiMessage::Labels(labels::Msg::SetInbox("lbl_work".into(), Some("none"))));
    assert!(hidden(&app).iter().all(|&(_, h)| h), "None hides all label chats");
    let _ = app.update(UiMessage::Labels(labels::Msg::SetInbox("lbl_work".into(), None)));
    assert_eq!(hidden(&app), shown);
}

#[test]
fn label_chats_from_the_chat_menu_like_desktop() {
    let mut app = demo_app("chat-labels");
    let chat = "!note_self:beeper.com";
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(chat.into())));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(2))));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_chat_menu(chat, false, 900.0));
        assert!(sim.find("Label").is_ok() && sim.find("New Label…").is_ok());
        let _ = sim.click("Work").expect("the Work label");
        sim.into_messages().collect()
    };
    assert!(
        matches!(&picks[..], [UiMessage::Labels(labels::Msg::Toggle(l, c, true))] if l == "lbl_work" && c == &[chat])
    );
    for m in picks {
        let _ = app.update(m);
    }
    let labels = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chat(chat).label_ids.clone()
    };
    assert_eq!(labels(&app), ["lbl_work"]);
    assert!(app.store.read().labels.iter().any(|l| l.id == "lbl_work" && l.rooms.iter().any(|r| r == chat)));
    assert_eq!(app.status_banner, "Added to Work");
    let _ = app.update(app.toast.as_ref().and_then(|t| t.open.clone()).expect("the toast opens the label"));
    assert_eq!(app.list.selected_account_filter.as_deref(), Some("label:lbl_work"));
    let _ = app.update(UiMessage::Composer(composer::Msg::Undo(false)));
    assert!(labels(&app).is_empty());
    assert_eq!(app.status_banner, "Undo: Add to Work");
    assert!(app.toast.is_none() && app.undo_stack.is_empty(), "the undo is not an action");
    app.focused = true;
    for _ in 0..(UNDO_NOTE_TIMEOUT.as_millis() / TOAST_TICK.as_millis()) {
        let _ = app.update(UiMessage::Toast(toasts::Msg::Tick));
    }
    assert!(app.status_banner.is_empty(), "the Undo: toast closes after a second");
    let _ = app.update(UiMessage::Labels(labels::Msg::Toggle("lbl_work".into(), vec![chat.into()], true)));
    let _ = app.update(UiMessage::Labels(labels::Msg::Toggle("lbl_work".into(), vec![chat.into()], false)));
    assert!(labels(&app).is_empty());
    assert!(app.status_banner == "Removed from Work" && app.toast.as_ref().is_some_and(|t| t.open.is_none()));
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenCreate(vec![chat.into()])));
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
    assert!(matches!(&msgs[..], [UiMessage::Labels(labels::Msg::OpenEdit(c))] if *c == chat));
    for m in msgs {
        let _ = app.update(m);
    }
    let _ = app.update(UiMessage::Labels(labels::Msg::OpenEdit(chat.clone())));
    assert_eq!(app.labels_windows.len(), 1, "one window for each chat");
    {
        let mut sim = simulator(app.view_edit_labels(&chat));
        assert!(sim.find("Edit Labels").is_ok() && sim.find("Work").is_ok());
        let _ = sim.click("New Label").expect("a New Label button");
        assert!(
            matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::Labels(labels::Msg::OpenCreate(c))] if c.is_empty())
        );
    }
    let _ = app.update(UiMessage::Labels(labels::Msg::Switch("lbl_work".into(), chat.clone(), true)));
    let st = app.store.read();
    assert_eq!(st.chat(&chat).label_ids, ["lbl_work"]);
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
    let _ = app.update(UiMessage::Info(info::Msg::SetDisappearing(id.clone(), 604_800_000)));
    let disappearing = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chat(&id).disappearing
    };
    assert_eq!(disappearing(&app), Some(604_800_000));
    let _ = app.update(UiMessage::Info(info::Msg::SetDisappearing(id.clone(), 0)));
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
        let last = st.timeline_indices(&id).last().map(|&i| st.messages[i].clone()).expect("a message");
        let me = st.chat(&id).participants.items.iter().find(|u| u.is_self == Some(true));
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
fn messages_disappear_like_desktop() {
    let mut app = demo_app("disappearing-messages");
    let id = "!tg_rust:beeper.com".to_string();
    let now = beeper_core::config::now_unix_millis();
    let timer = |kind: &str, timer: u64, read_at: Option<i64>| {
        Some(MessageTimer { kind: Some(kind.into()), timer, order: 0, read_at })
    };
    let group = {
        let mut st = app.store.write();
        let last = st.timeline_indices(&id).last().map(|&i| st.messages[i].clone()).expect("a message");
        let me = st.chat(&id).participants.items.iter().find(|u| u.is_self == Some(true));
        let me = me.expect("the user").id.clone();
        let messages = [
            ("@tulir:beeper.com", timer("after_send", 1_000, None), now - 10_000),
            ("@tulir:beeper.com", timer("after_read", 60_000, Some(now - 120_000)), now - 10_000),
            ("@tulir:beeper.com", timer("after_send", 3_600_000, None), now),
            (me.as_str(), timer("after_read_by_recipient", 60_000, None), now),
            ("@tulir:beeper.com", timer("after_read", 3_600_000, None), now),
        ];
        for (n, (sender, disappearing, at)) in messages.into_iter().enumerate() {
            st.upsert_message(Message {
                id: format!("timed_{n}"),
                sender_id: sender.into(),
                is_sender: Some(sender == me),
                sort_key: beeper_core::store::sort_key(
                    last.sort_key.parse::<i64>().expect("a sort key") + 1 + n as i64,
                ),
                timestamp: beeper_core::config::rfc3339_from_millis(at),
                disappearing,
                ..last.clone()
            });
        }
        st.chat(&id).chat_type == "group"
    };
    let _ = app.update(UiMessage::SelectChat(id));
    assert!(tall(app.view_conversation()).find("2 messages disappeared").is_ok(), "one note for the timed-out ones");
    let menu = |app: &BeeperGuiApp, mid: &str, label: &str| simulator(app.view_message_menu(mid).0).find(label).is_ok();
    assert!(menu(&app, "timed_2", "Disappears in about 1 hour"), "the line after Sent at");
    let recipient = match group {
        true => "Disappears 1 minute after someone in the group reads it",
        false => "Disappears 1 minute after the recipient reads it",
    };
    assert!(menu(&app, "timed_3", recipient), "your message waits for a read of the recipient");
    assert!(menu(&app, "timed_4", "Disappears 1 hour after you mark it read"));
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
    let ask = |a| UiMessage::Members(members::Msg::Ask(a, id.clone(), "@tulir:beeper.com".into()));
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
    let _ = app.update(UiMessage::Members(members::Msg::Remove(id.clone(), "@tulir:beeper.com".into())));
    assert!(tall(app.view_info()).find("Tulir Asokan").is_err());
}

/// Desktop's member tags: an invited member says "Invited" and one who left says "Exited".
#[test]
fn member_tags_like_desktop() {
    let mut app = demo_app("member-tags");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    {
        let mut st = app.store.write();
        let idx = st.resolve_chat_idx(&id).unwrap();
        st.update_chat(idx, |c| {
            let others = c.participants.items.iter_mut().filter(|u| u.is_self != Some(true));
            for (u, (pending, exited)) in others.zip([(Some(true), None), (None, Some(true))]) {
                (u.is_pending, u.has_exited) = (pending, exited);
            }
        });
    }
    let mut sim = tall(app.view_info());
    assert!(sim.find("Invited").is_ok() && sim.find("Exited").is_ok());
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
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(SubMenu::Members(MEMBERS_ADMIN))));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("tulir".into())));
    assert!(simulator(app.view_main()).find("Tulir Asokan").is_ok());
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    let _ = app.update(UiMessage::RunHotkey("Command Bar"));
    let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(SubMenu::Members(MEMBERS_DISMISS))));
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
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query("found".into())));
    app.new_chat_search = Some(("found".into(), 0, vec![(account, found)]));
    let cmds = app.commands(&app.store.read(), "found");
    assert_eq!(cmds.iter().map(|c| (c.category, c.title.as_str())).collect::<Vec<_>>(), [("Connections", "Found")]);
    let _ = app.update(UiMessage::CommandBar(commands::Msg::Query(pick.full_name.clone().expect("a name"))));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(chat(&app).participants.items.iter().any(|u| u.id == pick.id));
    assert!(!app.show_command_palette);
    let total = chat(&app).participants.total;
    let _ = app.update(UiMessage::Members(members::Msg::Add(id.clone(), Box::new(pick))));
    assert_eq!(chat(&app).participants.total, total, "the second click of a double click adds nobody");
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
fn user_hover_cards_like_desktop() {
    let mut app = demo_app("user-hover-card");
    let id = "!tg_rust:beeper.com";
    let _ = app.update(UiMessage::SelectChat(id.into()));
    let key = format!("{id} @batuhan:beeper.com");
    let shows = |app: &BeeperGuiApp| simulator(app.view_main()).find("@batuhan").is_ok();
    let due = |app: &mut BeeperGuiApp| {
        let n = app.user_hover.r#gen;
        let _ = app.update(UiMessage::UserCard(user_card::Msg::Due(n)));
    };
    // The card waits 300 ms on the trigger, and a pass over the trigger shows nothing.
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Hover(Some(key.clone()))));
    let stale = app.user_hover.r#gen;
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Hover(None)));
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Due(stale)));
    assert!(!shows(&app));
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Hover(Some(key.clone()))));
    due(&mut app);
    assert!(shows(&app), "the card shows after the wait");
    // The pointer crosses from the trigger to the card, which keeps it; leaving the card hides it.
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Hover(None)));
    let _ = app.update(UiMessage::UserCard(user_card::Msg::HoverCard(true)));
    due(&mut app);
    assert!(shows(&app));
    let _ = app.update(UiMessage::UserCard(user_card::Msg::HoverCard(false)));
    due(&mut app);
    assert!(!shows(&app));
    // Only the chat of the card shows it.
    let _ = app.update(UiMessage::UserCard(user_card::Msg::Hover(Some(key))));
    due(&mut app);
    let _ = app.update(UiMessage::SelectChat("!wa_family:beeper.com".into()));
    assert!(!shows(&app));
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
    assert_eq!(app.list.chat_menu.as_ref().map(|m| m.0.as_str()), Some("card:!tg_rust:beeper.com @batuhan:beeper.com"));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("@batuhan").is_ok());
        let _ = sim.click("Send Message").expect("the Send Message button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.list.chat_menu.is_none());
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
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
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
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account.clone(), Box::new(people[0].clone()))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::GroupTitle("Rustaceans".into())));
    assert_eq!(app.new_chats[0].title, "Rustaceans");
    assert!(found(&app, "Rustaceans"), "the name bar shows when the bridge allows a name");
    click(&mut app, "Start group chat");
    assert!(app.sticky_banner.is_none(), "the demo bridge needs two people");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account, Box::new(people[1].clone()))));
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
        let from = st.chat(sarah);
        let user = from.participants.items[other(from)].clone();
        let to = st.chat_idx("!sg_lena:beeper.com");
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
    let _ = app.update(UiMessage::Info(info::Msg::Media(Some(0))));
    assert!(tall(app.view_info()).find("Nothing here").is_err());
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        let _ = sim.click("Audio").expect("the Audio tab");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::Info(info::Msg::Media(Some(2)))]));
    let _ = app.update(UiMessage::Info(info::Msg::Media(Some(2))));
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
        let mut old = st.messages[st.timeline_indices(&id)[0]].clone();
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
    let _ = app.update(UiMessage::Info(info::Msg::RemoveBookmark("$gone".into())));
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
        let m = &st.messages[st.timeline_indices(&id)[0]];
        (m.id.clone(), list_ts(&m.timestamp, st.settings.pref("timeFormat") == 1))
    };
    let _ = app.update(UiMessage::Message(message::Msg::ToggleBookmark(mid.clone())));
    let msgs: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        assert!(sim.find("No Bookmarks").is_err());
        let _ = sim.click(date.as_str()).expect("the bookmark row");
        sim.into_messages().collect()
    };
    assert!(matches!(&msgs[..], [UiMessage::CommandBar(commands::Msg::JumpTo(c, m))] if *c == id && *m == mid));
    let menu = info_menu(&app.store.read(), &format!("bookmark:{id} {mid}"), |_| true);
    let labels: Vec<_> = menu.iter().map(|i| i.as_ref().map(|i| i.0.as_str())).collect();
    assert_eq!(labels, [Some("Jump to Message"), None, Some("Remove Bookmark")]);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("bookmark:{id} {mid}"))));
    let _ = app.update(menu[2].clone().and_then(|i| i.1).expect("Remove Bookmark"));
    assert!(app.list.chat_menu.is_none());
    assert!(tall(app.view_info()).find("No Bookmarks").is_ok());
}

#[test]
fn timer_labels_like_date_fns() {
    let labels: Vec<String> = DEFAULT_TIMERS.iter().map(|&ms| crate::time::timer_label(ms)).collect();
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
    assert_eq!(crate::time::timer_label(7_776_000_000), "3 months");
    assert_eq!(crate::time::timer_label(0), "Off");
}

#[test]
fn mute_labels_like_desktop() {
    let mut app = demo_app("label-mute");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("account:label:lbl_work".into())));
    let picks: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu("label:lbl_work"));
        assert!(sim.find("Edit Spaces…").is_ok() && sim.find("Delete Label").is_err());
        let _ = sim.click("Mute").expect("a Mute item");
        sim.into_messages().collect()
    };
    assert!(matches!(&picks[..], [UiMessage::Labels(labels::Msg::Mute(id, true))] if id == "lbl_work"));
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
    simulator(app.view_account_menu("label:lbl_work")).find("Unmute").expect("an Unmute item");
    let muted = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.notifications_muted(st.chat(&only))
    };
    assert!(muted(&app), "a chat is muted when all its labels are muted");
    {
        let mut sim = simulator(app.view_chat_menu(&only, false, 900.0));
        sim.find("Muted by Label Work").expect("the mute reason");
        let _ = sim.click("Unmute").expect("an Unmute item");
        let toggles = sim.into_messages().filter(|m| matches!(m, UiMessage::ChatList(chat_list::Msg::ToggleFlag(..))));
        assert_eq!(toggles.count(), 0, "a label mute is not undone from the chat");
    }
    let i = app.store.read().chat_idx(&only);
    app.store.write().update_chat(i, |c| c.label_ids.push(both.clone()));
    assert!(!muted(&app), "an unmuted label keeps the chat unmuted");
    let _ = app.update(UiMessage::Labels(labels::Msg::Mute("lbl_work".into(), false)));
    app.store.write().update_chat(i, |c| c.label_ids.retain(|l| l != &both));
    assert!(!muted(&app));
}

#[test]
fn toasts_time_out_like_desktop() {
    let mut app = demo_app("toast-timeout");
    app.focused = true;
    let _ = app.update(UiMessage::Message(message::Msg::CopyLink("https://example.com".into())));
    let shown = app.status_banner.clone();
    assert!(!shown.is_empty() && app.toast_running());
    let _ = app.update(UiMessage::Toast(toasts::Msg::Hover(true)));
    assert!(!app.toast_running(), "hover pauses the toast");
    let _ = app.update(UiMessage::Toast(toasts::Msg::Hover(false)));
    let ticks = (TOAST_TIMEOUT.as_millis() / TOAST_TICK.as_millis()) as usize;
    for _ in 1..ticks {
        let _ = app.update(UiMessage::Toast(toasts::Msg::Tick));
    }
    assert_eq!(app.status_banner, shown);
    let _ = app.update(UiMessage::Toast(toasts::Msg::Tick));
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
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let n = app.creating_chat.expect("a draft");
    assert!(found(&app, "Sam Example") && found(&app, "Bo Example") && !found(&app, &label));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.creating_chat, Some(n), "Start chat needs a person");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("sam".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(sam.id.clone())));
    assert!(found(&app, &label), "an account with any_phone reaches the phone number");
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let user = nets.iter().find(|(a, _)| *a == account).and_then(|(_, u)| u.clone().flatten()).expect("a user");
    assert_eq!((user.id.as_str(), user.phone_number.as_deref()), ("+15550100", Some("+15550100")));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account.clone(), Box::new(user.clone()))));
    assert!(app.new_chat_query.is_empty() && app.new_chats[0].users.len() == 1);
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Backspace), Modifiers::empty()));
    assert!(app.new_chats[0].users.is_empty(), "Backspace removes the pill");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account.clone(), Box::new(user))));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.new_chats.is_empty() && app.creating_chat.is_none(), "the draft goes");
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(dm.as_str()), "the DM with the same phone opens");

    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "lookup_phone": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let n = app.creating_chat.expect("a draft");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(bo.id.clone())));
    assert!(!found(&app, &label) && !found(&app, "No networks found"), "the lookup loads");
    let slot = |app: &BeeperGuiApp| {
        app.contact_networks.as_ref().expect("the networks").1.iter().position(|(a, _)| *a == account).expect("a slot")
    };
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ContactNetwork(bo.id.clone(), slot(&app), Ok(None))));
    assert!(!found(&app, &label) && !found(&app, "No networks found"), "the other accounts reach the phone number");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Account(Some(account.clone()))));
    assert!(found(&app, "No networks found"), "the other accounts are not picked");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Account(None)));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(bo.id.clone())));
    assert!(app.contact_networks.is_none() && !found(&app, "No networks found"), "a second click collapses");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(bo.id.clone())));
    let ghost = serde_json::json!({ "id": "199", "mxid": "@bridge_199:beeper.local", "name": "Bo Example" });
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ContactNetwork(bo.id.clone(), slot(&app), Ok(Some(ghost)))));
    assert!(found(&app, &label));
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let user = nets.iter().find(|(a, _)| *a == account).and_then(|(_, u)| u.clone().flatten()).expect("a user");
    assert_eq!(user.id, "@bridge_199:beeper.local", "a resolved user has its Matrix ID");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account, Box::new(user))));
    assert!(found(&app, "Bo Example"), "the draft is titled with the person");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.status_banner, "Composing chat...");
    assert!(!app.toast_running(), "the composing toast stays");
    let _ =
        app.update(UiMessage::NewChat(new_chat::Msg::DmCreated(Some(n), String::new(), None, Err("timeout".into()))));
    assert!(
        app.status_banner.starts_with("Something went wrong creating thread (")
            && app.status_banner.ends_with(")\ntimeout")
    );
    assert!(
        matches!(&app.shown_toast().expect("a toast").buttons[..], [("Copy Error", UiMessage::Message(message::Msg::CopyText(e)))] if e == "timeout")
    );
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::DmCreated(
        Some(n),
        String::new(),
        None,
        Ok("!new:beeper.local".into()),
    )));
    assert!(app.new_chats.is_empty() && app.creating_chat.is_none() && app.status_banner.is_empty());
    assert_eq!(app.pending_dm, Some(("!new:beeper.local".into(), None)), "the room opens when it syncs");
}

#[test]
fn contacts_without_a_phone_number_resolve_like_desktop() {
    let mut app = demo_app("contact-username");
    let account = rail_accounts(&app.store.read()).into_iter().find(|a| a.contains("telegram")).expect("telegram");
    let caps =
        serde_json::json!({ "resolve_identifier": { "create_dm": true, "any_phone": true, "lookup_username": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let lee: User =
        serde_json::from_value(serde_json::json!({ "id": "contact:t:lee", "fullName": "Lee", "username": "lee" }))
            .expect("a user");
    app.store.write().set_contacts(vec![lee.clone()]);
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(lee.id.clone())));
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let asked: Vec<&str> = nets.iter().filter(|(_, n)| n.is_none()).map(|(a, _)| a.as_str()).collect();
    assert_eq!(
        (asked, nets.len()),
        (vec!["hungryserv", account.as_str()], 2),
        "Desktop looks up the username, also as a Matrix ID, and any_phone needs a phone number"
    );
}

#[test]
fn new_chat_accounts_filter_like_desktop() {
    let mut app = demo_app("new-chat-accounts");
    let account = rail_accounts(&app.store.read()).into_iter().find(|a| a.contains("telegram")).expect("telegram");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleAccounts));
    assert!(found(&app, "Telegram") && found(&app, "WhatsApp"));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Account(Some(account.clone()))));
    assert!(!app.new_chat_accounts_open && app.new_chat_account.as_deref() == Some(account.as_str()));
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(serde_json::json!({ "resolve_identifier": {} }))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert!(app.new_chat_account.is_none(), "a new draft shows all accounts");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleAccounts));
    assert!(!found(&app, "Telegram") && found(&app, "WhatsApp"), "a bridge without create_dm is not listed");
    let (n, drafts) = (app.creating_chat.expect("a draft"), app.new_chats.len());
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Delete(n)));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Delete(n)));
    assert_eq!(app.new_chats.len(), drafts - 1, "the second click of a double click removes nothing");
}

/// Desktop shows the previews of all links with "Load external embeds automatically" off, which only gates the
/// embedded frames. A YouTube link shows its video card, and a click on the card opens the video.
#[test]
fn link_previews_like_desktop() {
    let note = "!note_self:beeper.com";
    let mut app = demo_app("link-previews");
    let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("loadEmbeds", 0)));
    let _ = app.update(UiMessage::SelectChat(note.into()));
    let tube = "https://youtu.be/dQw4w9WgXcQ";
    {
        let mut st = app.store.write();
        let i = *st.timeline_indices(note).last().expect("a message");
        let link = |url: &str, title: &str| MessageLink { url: url.into(), title: title.into(), ..Default::default() };
        st.messages[i].links = vec![link("https://example.com/a", "First page"), link(tube, "A video")];
    }
    let msgs: Vec<UiMessage> = {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 900.0), app.view_main());
        for label in ["Scaling Laws for Neural Language Models", "First page", "YouTube", "A video"] {
            assert!(sim.find(label).is_ok(), "{label} shows");
        }
        let _ = sim.click("A video").expect("the video card");
        let _ = sim.click("First page").expect("the link card");
        sim.into_messages().collect()
    };
    assert!(msgs.iter().any(|m| matches!(m, UiMessage::OpenUrl(u) if u == tube)), "the card opens the video");
    let page = |m: &UiMessage| matches!(m, UiMessage::OpenUrl(u) if u == "https://example.com/a");
    assert!(msgs.iter().any(page), "the card opens the link");
    let i = app.store.write().find_message("msg_note_0").expect("the seed message with a link");
    app.store.write().messages[i].links.clear();
    let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 900.0), app.view_main());
    assert!(sim.find("Scaling Laws for Neural Language Models").is_err(), "a link without a preview shows none");
}

#[test]
fn email_links_and_media_items_match_desktop() {
    let address =
        |t: &str| crate::linkify::links(t).into_iter().map(|(r, h)| (t[r].to_string(), h)).collect::<Vec<_>>();
    assert_eq!(
        address("a.b+c@mail.example.com)."),
        [("a.b+c@mail.example.com".to_string(), "mailto:a.b+c@mail.example.com".to_string())]
    );
    assert!(["a@b", "a@b.c", "a@b.c1", "ab"].iter().all(|t| address(t).is_empty()));
    let mut app = demo_app("email-menu");
    let mid = {
        let mut st = app.store.write();
        let chat = app.pane.selected_chat_id.clone().expect("a chat");
        let i = *st.timeline_indices(&chat).last().expect("a message");
        let m = &mut st.messages[i];
        (m.text, m.html, m.links, m.unresolved_media) =
            (Some("Mail a.b@example.com.".into()), None, Vec::new(), Some("{}".into()));
        m.id.clone()
    };
    // Desktop's address items are the ones of a right click on the address's `mailto:` link.
    let click = |app: &mut BeeperGuiApp, label: &str| {
        let open = match label.contains('@') {
            true => message::Msg::LinkMenu(mid.clone(), "mailto:a.b@example.com".into()),
            false => message::Msg::OpenMenu(mid.clone()),
        };
        let _ = app.update(UiMessage::Message(open));
        let mut sim = simulator(app.view_main());
        let _ = sim.click(label).unwrap_or_else(|_| panic!("{label}"));
        sim.into_messages().collect::<Vec<_>>()
    };
    let copy = click(&mut app, "Copy a.b@example.com");
    assert!(
        matches!(&copy[..], [UiMessage::Message(message::Msg::Menu(m))] if matches!(&**m, UiMessage::Message(message::Msg::CopyText(e)) if e == "a.b@example.com"))
    );
    let mail = click(&mut app, "Email a.b@example.com…");
    assert!(
        matches!(&mail[..], [UiMessage::Message(message::Msg::Menu(m))] if matches!(&**m, UiMessage::OpenUrl(u) if u == "mailto:a.b@example.com"))
    );
    assert!(
        matches!(&click(&mut app, "Load Media")[..], [UiMessage::Message(message::Msg::Menu(m))] if matches!(**m, UiMessage::Loaded(loads::Msg::ResolveMedia(..))))
    );
    app.resolving_media.insert(mid.clone(), true);
    let off = click(&mut app, "Load Media");
    assert!(
        !off.iter().any(|m| matches!(m, UiMessage::Message(message::Msg::Menu(_)))),
        "a resolving media item is disabled"
    );
    app.resolving_media.insert(mid.clone(), false);
    assert_eq!(click(&mut app, "Try Loading Media Again").len(), 1);
}

#[test]
fn errors_toast_like_desktop() {
    let mut app = demo_app("error-toast");
    let failed =
        UiMessage::Loaded(loads::Msg::MediaResolved("acc".into(), "$ev".into(), "{}".into(), Err("timeout".into())));
    let _ = app.update(failed);
    let t = app.shown_toast().expect("an error toast");
    assert!(t.error && app.status_banner == "Could not load this media.");
    let [(label, again)] = &t.buttons[..] else { panic!("one button") };
    assert!(
        *label == "Try again"
            && matches!(again, UiMessage::Loaded(loads::Msg::ResolveMedia(a, e, _)) if a == "acc" && e == "$ev")
    );
    let _ = app.update(again.clone());
    assert_eq!(app.resolving_media.get("$ev"), Some(&true), "Try again resolves the media again");
    let _ = app.update(UiMessage::Message(message::Msg::CopyLink("https://example.com".into())));
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
    assert!(
        matches!(&header[..], [UiMessage::Window(windows::Msg::ToggleMaximize(None))]),
        "the chat header maximizes: {header:?}"
    );
    // The chat header hides the link card under it from the cursor, and the card opens its link below the header.
    let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
    let (size, card) = (Size::new(1000.0, 700.0), Point::new(900.0, 40.0));
    let over = interact(&app, main, size, card, twice.clone());
    assert!(
        matches!(&over[..], [UiMessage::Window(windows::Msg::ToggleMaximize(None))]),
        "the header over a card maximizes"
    );
    let below = interact(&app, main, size, Point::new(900.0, 90.0), twice[..2].to_vec());
    assert!(
        matches!(&below[..], [UiMessage::Message(message::Msg::HoverEnter(_)), UiMessage::OpenUrl(_)]),
        "the card opens its link"
    );
    let edge = interact(&app, main, Size::new(1440.0, 900.0), Point::new(68.0 + SIDEBAR_WIDTH + 1.0, 400.0), twice);
    assert!(
        edge.iter().any(|m| matches!(m, UiMessage::Rail(rail::Msg::ResetWidth))),
        "the sidebar edge resets: {edge:?}"
    );
    let _ = app.update(UiMessage::Rail(rail::Msg::ResizeStart));
    let _ = app.update(UiMessage::Rail(rail::Msg::ResizeMoved(Point::new(500.0, 10.0))));
    let _ = app.update(UiMessage::Rail(rail::Msg::ResizeEnd));
    assert!(app.sidebar_width > SIDEBAR_WIDTH);
    let _ = app.update(UiMessage::Rail(rail::Msg::ResizeStart));
    let _ = app.update(UiMessage::Rail(rail::Msg::ResetWidth));
    let _ = app.update(UiMessage::Rail(rail::Msg::ResizeEnd));
    assert_eq!(app.sidebar_width, SIDEBAR_WIDTH);
    let saved =
        std::fs::read_to_string(app.store.read().config.data_dir.join("sidebar-width")).expect("the saved width");
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(chat.into(), ChatFlag::Muted)));
    let _ = app.update(UiMessage::Labels(labels::Msg::SetInbox("lbl_work".into(), Some("none"))));
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
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert!(!found(&app, "Chats"), "Desktop lists chat people only for a query");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("ERIC".into())));
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
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account, Box::new(eric))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.pane.selected_chat_id.as_deref(), Some(dm.as_str()), "the DM with the Matrix ID opens");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("zzzz".into())));
    assert!(found(&app, "No results found") && !found(&app, "Chats"));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("  ".into())));
    assert!(!found(&app, "No results found"), "only a query has no results");
}

#[test]
fn new_chat_looks_up_identifiers_like_desktop() {
    let mut app = demo_app("new-chat-lookup");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    assert!(typed_phone("+1 (415) 555-0177") && !typed_phone("555") && !typed_phone("call 4155550177"));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("+1 415 555 0177".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("+1 415".into())));
    assert!(app.new_chat_lookup.is_none(), "a lookup of an old query does nothing");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("+1 415 555 0177".into())));
    let nets = &app.new_chat_lookup.as_ref().expect("a lookup").1;
    assert!(nets.len() > 1 && nets.iter().all(|(_, n)| n.as_ref().is_some_and(Option::is_some)), "any_phone");
    assert!(found(&app, &fmt_phone("+14155550177")) && !found(&app, "No results found"));
    let account = rail_accounts(&app.store.read()).into_iter().next().expect("an account");
    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "lookup_username": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("@samx".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("@samx".into())));
    let nets = &app.new_chat_lookup.as_ref().expect("a lookup").1;
    assert!(matches!(&nets[..], [(a, None)] if *a == account), "only the account with lookup_username asks");
    assert!(found(&app, "No results found"), "rows show when all lookups end");
    let user = serde_json::json!({ "id": "x", "mxid": "@x_samx:beeper.local", "name": "Sam X", "identifiers": ["mailto:sam@example.com"] });
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ContactNetwork("lookup:@samx".into(), 0, Ok(Some(user)))));
    assert!(found(&app, "Sam X") && found(&app, "sam@example.com"));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("@sam".into())));
    assert!(app.new_chat_lookup.is_none() && !found(&app, "Sam X"), "typing drops the lookup");
}

#[test]
fn new_chat_searches_connections_like_desktop() {
    let mut app = demo_app("new-chat-connections");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let account = rail_accounts(&app.store.read()).into_iter().find(|a| a != "hungryserv").expect("a bridge");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("dana".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("dana".into())));
    assert!(app.new_chat_search.is_none(), "no bridge can search");
    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "search": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("dana".into())));
    assert!(found(&app, "Connections") && found(&app, "Searching..."));
    let user = |name: &str, phone: &str| serde_json::json!({ "id": name, "mxid": format!("@x_{name}:beeper.local"), "name": name, "identifiers": [format!("tel:{phone}")] });
    let results = [user("Dana One", "+15550111"), user("Dana Two", "+15550111"), user("Ann Dana", "+15550112")];
    let results: Vec<User> = results.iter().map(beeper_core::matrix::bridge_user).collect();
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::SearchedUsers(
        "dan".into(),
        account.clone(),
        Ok(results.clone()),
    )));
    assert!(found(&app, "Searching..."), "results of an old query do nothing");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::SearchedUsers("dana".into(), account, Ok(results))));
    assert!(!found(&app, "Searching...") && found(&app, "Dana One") && found(&app, "Ann Dana"));
    assert!(!found(&app, "Dana Two"), "one row per bridge and phone number");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("danae".into())));
    assert!(app.new_chat_search.is_none() && !found(&app, "Connections"));
}

/// Desktop's Matrix account in New Chat: the capabilities of the SDK's `matrix` pseudo-bridge, the profile lookup
/// of a typed Matrix ID, the user directory only with "Include Matrix results in Start New Chat", and direct chats
/// and groups that the Matrix client creates.
#[test]
fn new_chat_reaches_matrix_users_like_desktop() {
    let mut app = demo_app("new-chat-matrix");
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let matrix = "hungryserv".to_string();
    let caps = ["create_dm", "lookup_username", "search"].map(|c| app.can(&matrix, c));
    assert!(caps.iter().all(|&c| c) && !app.can(&matrix, "lookup_phone") && app.can_group(&matrix));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Account(Some(matrix.clone()))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("@vic:beeper.com".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("@vic:beeper.com".into())));
    assert!(app.new_chat_search.is_none(), "the Matrix account does not search by default");
    let nets = &app.new_chat_lookup.as_ref().expect("a lookup").1;
    assert!(matches!(&nets[..], [(a, None)] if *a == matrix), "the Matrix account looks up the Matrix ID");
    let vic = serde_json::json!({ "id": "@vic:beeper.com", "name": "Vic", "identifiers": ["matrix:u/vic:beeper.com"],
        "mxid": "@vic:beeper.com" });
    let key = "lookup:@vic:beeper.com".to_string();
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ContactNetwork(key, 0, Ok(Some(vic.clone())))));
    assert!(found(&app, "Vic"));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(
        matrix.clone(),
        Box::new(beeper_core::matrix::bridge_user(&vic)),
    )));
    let n = app.creating_chat.expect("a draft");
    // A toast that stays, like a verification request, does not stop the chat from being made.
    app.sticky_banner = Some("Verification request from Pixel".into());
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.status_banner, "Composing chat...", "the Matrix client creates the direct chat");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::DmCreated(Some(n), matrix.clone(), None, Ok("!dm:x".into()))));
    assert_eq!(app.pending_dm, Some(("!dm:x".into(), None)), "the room opens when it syncs");

    let prefs = serde_json::json!({ beeper_core::prefs::INCLUDE_MATRIX_ACCOUNT_SEARCH: true });
    app.store.write().synced_prefs.insert(beeper_core::prefs::DESKTOP_PREFS, prefs);
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Account(Some(matrix.clone()))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::NewGroup));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Query("una".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::LookupIdentifier("una".into())));
    assert!(found(&app, "Connections") && found(&app, "Searching..."), "the internal setting adds the directory");
    let user = |id: &str, name: Option<&str>| User {
        id: id.into(),
        username: Some(id.into()),
        full_name: name.map(Into::into),
        ..Default::default()
    };
    let users = vec![user("@una:beeper.com", Some("Una")), user("@ula:beeper.com", None)];
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::SearchedUsers("una".into(), matrix.clone(), Ok(users))));
    assert!(found(&app, "Una") && found(&app, "@una:beeper.com") && found(&app, "@ula:beeper.com"));
    let n = app.creating_chat.expect("a draft");
    for u in [user("@una:beeper.com", Some("Una")), user("@ula:beeper.com", None)] {
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(matrix.clone(), Box::new(u))));
    }
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::GroupTitle("Team".into())));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.confirm, Some(ASK_ROOM_ENCRYPTION), "Desktop asks about encryption first");
    assert!(found(&app, "Room encryption") && found(&app, "People invited later cannot see earlier messages."));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("Encrypted").expect("the toggle row");
        sim.into_messages().collect()
    };
    let off = msgs.iter().find(|m| matches!(m, UiMessage::NewChat(new_chat::Msg::Encrypted(false))));
    let _ = app.update(off.expect("a click turns encryption off").clone());
    assert!(found(&app, "People invited later can see earlier messages."));
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Escape), keyboard::Modifiers::empty()));
    assert!(app.confirm.is_none() && app.creating_chat == Some(n), "Escape closes the step and keeps the draft");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), keyboard::Modifiers::empty()));
    assert!(found(&app, "People invited later can see earlier messages."), "the toggle stays for the draft");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), keyboard::Modifiers::empty()));
    assert!(app.confirm.is_none(), "Enter in the step starts the group");
    assert_eq!(app.status_banner, "Creating group chat...", "the Matrix client creates the group");
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Enter), keyboard::Modifiers::empty()));
    assert!(app.confirm.is_none(), "Desktop's `isCreatingThread`: Enter does not make a second group");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::GroupCreated(n, matrix, Ok("!group:x".into()))));
    assert!(app.new_chats.iter().all(|d| d.id != n) && app.pending_dm == Some(("!group:x".into(), None)));
}

#[test]
fn new_chat_groups_follow_bridge_capabilities_like_desktop() {
    let mut app = demo_app("new-chat-groups");
    let accounts: Vec<String> = rail_accounts(&app.store.read()).into_iter().filter(|a| app.can_group(a)).collect();
    let found = |app: &BeeperGuiApp, s: &str| simulator(app.view_main()).find(s).is_ok();
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert!(accounts.len() > 1 && found(&app, "New Group"));
    let user = |id: &str| Box::new(User { id: id.into(), full_name: Some(id.into()), ..Default::default() });
    let (a, b) = (accounts[0].clone(), accounts[1].clone());
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(a.clone(), user("@one:x"))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(a.clone(), user("@two:x"))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(a.clone(), user("@two:x"))));
    let ids = |app: &BeeperGuiApp| app.new_chats[0].users.iter().map(|(_, u)| u.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&app), ["@one:x", "@two:x"], "a group bridge adds people once");
    let single = serde_json::json!({ "resolve_identifier": { "create_dm": true } });
    for acc in &accounts {
        let _ = app.update(UiMessage::Capabilities(acc.clone(), Ok(single.clone())));
    }
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(b, user("@three:x"))));
    assert_eq!(ids(&app), ["@three:x"], "a bridge without groups replaces the person");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert!(!found(&app, "New Group"), "no bridge can create groups");
}

/// Desktop's `GroupTitleBar` avatar and `T$t`: a bridge whose groups need an avatar starts one only with a picked
/// image, and its `max_length` stops the picks with Desktop's toast.
#[test]
fn new_group_avatar_and_size_like_desktop() {
    let mut app = demo_app("new-group-avatar");
    let accounts = rail_accounts(&app.store.read());
    let account = accounts.into_iter().find(|a| app.can_group(a) && a != "hungryserv").expect("a group bridge");
    let group = serde_json::json!({ "name": { "allowed": true }, "avatar": { "required": true },
        "participants": { "max_length": 2 } });
    let _ = app.update(UiMessage::Capabilities(
        account.clone(),
        Ok(serde_json::json!({ "group_creation": { "group": group } })),
    ));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::NewGroup));
    let user = |id: &str| Box::new(User { id: id.into(), full_name: Some(id.into()), ..Default::default() });
    for id in ["@one:x", "@two:x", "@three:x"] {
        let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account.clone(), user(id))));
    }
    assert_eq!(app.new_chats[0].users.len(), 2, "the bridge takes two people");
    let full = format!("{} groups can have up to 2 people", network_label(&account));
    assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some(full.as_str()));
    let start = |app: &BeeperGuiApp| {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("Start group chat").expect("the button");
        sim.into_messages().count()
    };
    assert_eq!(start(&app), 0, "the bridge needs an avatar");
    let path = beeper_core::testing::temp_dir("new-group-avatar").join("a.png");
    ::image::RgbaImage::new(4, 4).save_with_format(&path, ::image::ImageFormat::Png).expect("write an image");
    let n = app.new_chats[0].id;
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::AvatarLoaded(n, portal::load_file(&path))));
    assert!(app.new_chats[0].photo.as_ref().is_some_and(|(_, mime, _)| mime == "image/png"));
    assert_eq!(start(&app), 1, "the picked avatar enables Start");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Start));
    assert_eq!(app.sticky_banner.as_deref(), Some("Creating group chat..."));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::NewGroup));
    let m = n + 1;
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::AvatarLoaded(m, portal::load_file(&path))));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Select(n)));
    assert!(app.new_chats.iter().any(|d| d.id == m), "Desktop's `isDraftEmpty` keeps a draft with an avatar");
}

#[test]
fn reply_privately_like_desktop() {
    let mut app = demo_app("reply-privately");
    let (theirs, mine, account) = {
        let st = app.store.read();
        let c = st.chats.iter().find(|c| c.id == "!tg_rust:beeper.com").expect("the group");
        let msgs = st.timeline(&c.id);
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
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ReplyPrivately(theirs.id.clone())));
    assert_eq!(app.status_banner, "Composing chat...");
    let quote = format!("> {}\n", theirs.text.clone().expect("text").lines().next().expect("a line"));
    let room = "!sg_security:beeper.com".to_string();
    let _ =
        app.update(UiMessage::NewChat(new_chat::Msg::DmCreated(None, account, Some(quote.clone()), Ok(room.clone()))));
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
        network_name(&st.chat(&id).account_id)
    };
    let set = |app: &mut BeeperGuiApp, rules: MessageRules| {
        let mut st = app.store.write();
        let i = st.chat_idx(&id);
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
    assert!(clicked(&app, "Edit…").is_empty(), "disabled like Desktop");
    assert!(simulator(app.view_message_menu(&mine).0).find("React…").is_err(), "the network has no reactions");
    let ask = clicked(&app, "Delete…");
    assert!(
        matches!(&ask[..], [UiMessage::Message(message::Msg::Menu(m))] if matches!(&**m, UiMessage::Message(message::Msg::AskDelete(v)) if v == std::slice::from_ref(&mine)))
    );
    assert!(app.last_own_message().is_none_or(|m| m.id != mine), "Up Arrow skips a message past edit_max_age");
    app.pane.selected_msgs = vec![mine.clone()];
    let titles = |app: &BeeperGuiApp| -> Vec<String> {
        app.commands(&app.store.read(), "").into_iter().filter(|c| c.category == "Message").map(|c| c.title).collect()
    };
    let t = titles(&app);
    assert!(t.contains(&"Delete…".into()) && t.contains(&"Edit".into()) && !t.contains(&"Reply".into()));
    set(&mut app, MessageRules { delete: false, ..old });
    assert!(simulator(app.view_message_menu(&mine).0).find("Delete…").is_ok(), "Remove from Beeper stays");
    app.store.write().delete_message(&id, &mine).expect("delete the message");
    assert!(simulator(app.view_message_menu(&mine).0).find("Delete…").is_err(), "a deleted message has no Delete…");
    assert!(titles(&app).contains(&format!("Delete message is not supported for {network} yet")));
}

/// Desktop's "Delete message?" dialog: your messages get the button of the deletion mode of the room, sent
/// messages get "Remove from Beeper", and the text tells where the removed messages stay.
#[test]
fn delete_dialog_removes_from_beeper_like_desktop() {
    let mut app = demo_app("delete-dialog");
    let (cid, mine, theirs) = {
        let st = app.store.read();
        let c = st.chats.iter().find(|c| {
            let ms = st.messages.iter().filter(|m| m.chat_id == c.id && m.text.is_some() && !m.silent());
            c.chat_type == "single"
                && ms.clone().any(|m| m.is_sender == Some(true))
                && ms.clone().any(|m| m.is_sender != Some(true))
        });
        let c = c.expect("a DM with messages of both people");
        let ms = || st.messages.iter().filter(|m| m.chat_id == c.id && m.text.is_some() && !m.silent());
        let id = |mine: bool| ms().rfind(|m| (m.is_sender == Some(true)) == mine).expect("a message").id.clone();
        (c.id.clone(), id(true), id(false))
    };
    let rules = |app: &mut BeeperGuiApp, delete: bool, delete_for_me: bool| {
        let mut st = app.store.write();
        let i = st.chat_idx(&cid);
        st.update_chat(i, |c| {
            c.messages = MessageRules { delete, delete_for_me, delete_max_age: None, ..BEEPER_MESSAGES }
        });
    };
    let dialog = |app: &mut BeeperGuiApp, ids: &[&String]| -> (String, String, Vec<&'static str>) {
        let _ = app.update(UiMessage::Message(message::Msg::AskDelete(ids.iter().map(|id| id.to_string()).collect())));
        let st = app.store.read();
        let ms: Vec<Message> = ids.iter().map(|id| stored(&st, id)).collect();
        let (body, buttons) = delete_dialog(&st, &ms.iter().collect::<Vec<_>>()).expect("a delete dialog");
        let title = trn("Delete message?", "Delete messages?", ms.len(), &[]);
        (title, body, buttons.into_iter().map(|b| b.0).collect())
    };
    let (title, network, name) = {
        let st = app.store.read();
        let c = st.chat(&cid);
        ("Delete message?".to_string(), network_name(&c.account_id), c.title.clone())
    };
    rules(&mut app, true, false);
    let note = format!("Removing it from Beeper keeps it on {network} for you and {name}.");
    assert_eq!(
        dialog(&mut app, &[&mine]),
        (title.clone(), note.clone(), vec!["Delete for Everyone", "Remove from Beeper"])
    );
    for text in ["Delete message?", note.as_str(), "Delete for Everyone", "Remove from Beeper", "Cancel"] {
        assert!(simulator(app.view_main()).find(text).is_ok(), "{text}");
    }
    rules(&mut app, false, true);
    assert_eq!(dialog(&mut app, &[&mine]).2, ["Delete for Me", "Remove from Beeper"]);
    rules(&mut app, false, false);
    assert_eq!(dialog(&mut app, &[&mine]).2, ["Remove from Beeper"]);
    rules(&mut app, true, false);
    assert!(simulator(app.view_message_menu(&theirs).0).find("Delete…").is_ok(), "received messages have Delete…");
    let both = dialog(&mut app, &[&mine, &theirs]);
    let note = format!("Removing them from Beeper keeps them on {network} for you and {name}.");
    assert_eq!(both, ("Delete messages?".into(), note, vec!["Remove from Beeper"]), "only your messages get a mode");
    let _ = app.update(UiMessage::Message(message::Msg::RemoveFromBeeper(vec![mine.clone(), theirs.clone()])));
    {
        let st = app.store.read();
        assert!([&mine, &theirs].iter().all(|id| stored(&st, id).removed && stored(&st, id).text.is_none()));
    }
    assert!(app.confirm.is_none(), "the button closes the dialog");
    let _ = app.update(UiMessage::SelectChat(cid.clone()));
    assert!(simulator(app.view_main()).find("Removed from Beeper").is_ok(), "the row of a removed message");
    assert!(simulator(app.view_message_menu(&mine).0).find("Delete…").is_err(), "a removed message has no Delete…");
    let account = stored(&app.store.read(), &mine).account_id;
    let note = |account: &str, kind: &str, title: &str| {
        let mut st = app.store.write();
        let i = st.chat_idx(&cid);
        st.update_chat(i, |c| (c.account_id, c.chat_type, c.title) = (account.into(), kind.into(), title.into()));
        let m = stored(&st, &mine);
        removal_note(&st, &[&m])
    };
    assert_eq!(note("hungryserv", "group", "G"), "Removing it from Beeper keeps it for you and the group.");
    assert_eq!(note("hungryserv", "single", "Ann"), "Removing it from Beeper keeps it for you and Ann.");
    assert_eq!(note("hungryserv", "single", " "), "Removing it from Beeper does not delete it for other people.");
    assert_eq!(
        note(&account, "group", "G"),
        format!("Removing it from Beeper keeps it on {network} for you and the group.")
    );
    assert_eq!(
        note(&account, "single", ""),
        format!("Removing it from Beeper keeps it on {network} for everyone in this chat.")
    );
}

#[test]
fn message_menu_speed_and_read_like_desktop() {
    let mut app = demo_app("message-menu-items");
    let apply = |app: &mut BeeperGuiApp, msgs: Vec<UiMessage>| msgs.into_iter().for_each(|m| drop(app.update(m)));
    let voice = {
        let st = app.store.read();
        let m = st.messages.iter().find(|m| m.attachments.first().is_some_and(|a| app.playable(a)));
        m.expect("a voice message").id.clone()
    };
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(voice.clone())));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_message_menu(&voice).0);
        let _ = sim.click("Playback Speed").expect("Playback Speed");
        sim.into_messages().collect()
    };
    apply(&mut app, picked);
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_main());
        let _ = sim.click("1.5x").expect("the rate submenu");
        sim.into_messages().collect()
    };
    apply(&mut app, picked);
    assert!(app.pane.message_menu.is_none(), "a rate closes the menu");
    let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(voice.clone())));
    let _ = app.update(UiMessage::Voice(crate::voice::Msg::Sub));
    assert!(simulator(app.view_main()).find("1.5x").is_ok());
    let _ = app.update(UiMessage::CloseModals);

    let (cid, unread) = {
        let st = app.store.read();
        let m = st.messages.iter().filter(|m| m.is_unread == Some(true)).min_by_key(|m| &m.sort_key);
        let m = m.expect("an unread message");
        (m.chat_id.clone(), m.id.clone())
    };
    let read = |app: &BeeperGuiApp| simulator(app.view_message_menu(&unread).0).find("Mark as Read").is_ok();
    assert!(read(&app));
    let picked: Vec<UiMessage> = {
        let mut sim = simulator(app.view_message_menu(&unread).0);
        let _ = sim.click("Mark as Read").expect("Mark as Read");
        sim.into_messages().collect()
    };
    apply(&mut app, picked);
    assert!(!read(&app), "the message is read");
    let st = app.store.read();
    let left = st.timeline_indices(&cid).iter().filter(|&&i| st.messages[i].is_unread == Some(true)).count();
    assert_eq!(st.chat(&cid).unread_count as usize, left);
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
    let menu = format!("info:{id}");
    assert!(
        matches!(&click(&app, "Edit")[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(k))] if *k == menu),
        "Edit for Me and Chat Theme"
    );
    app.store.write().features.flags["features"]["*"]["personal-room-overrides"] = false.into();
    assert!(
        matches!(&click(&app, "Edit")[..], [UiMessage::ChatTheme(chat_theme::Msg::Open(c))] if *c == id),
        "one action runs at once"
    );
    set_edits(&mut app, ALL_EDITS);
    assert!(matches!(&click(&app, "Edit")[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(k))] if *k == menu));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(menu)));
    {
        let mut sim = simulator(app.view_main());
        assert!(sim.find("Edit Group").is_ok() && sim.find("Chat Theme").is_ok());
        assert!(sim.find("Copy Beeper Rust Core Team").is_err(), "the Edit menu has no copy item");
    }
    let copy = format!("copy:{id}");
    assert!(
        matches!(&click(&app, "Beeper Rust Core Team")[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(k))] if *k == copy)
    );
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(copy)));
    let mut sim = simulator(app.view_main());
    let _ = sim.click("Copy Beeper Rust Core Team").expect("the title menu copies the title");
    assert!(
        matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::Message(message::Msg::CopyText(t))] if t == "Beeper Rust Core Team")
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
    let _ = app.update(UiMessage::Info(info::Msg::Desc(None)));
    let _ = app.update(UiMessage::Info(info::Msg::Desc(Some(text_editor::Action::Edit(text_editor::Edit::Paste(
        std::sync::Arc::new("We build https://example.com".into()),
    ))))));
    let _ = simulator(app.view_info()).click("Done").expect("Done saves");
    let _ = app.update(UiMessage::Info(info::Msg::SaveDesc));
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
    let _ = app.update(UiMessage::Info(info::Msg::More));
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
    let _ = app.update(UiMessage::Info(info::Msg::All));
    assert!(has(&app, "Member 4") && has(&app, "Show less"));
    let _ = app.update(UiMessage::Info(info::Msg::All));
    let _ = app.update(UiMessage::Info(info::Msg::Query("member 4".into())));
    assert!(has(&app, "Member 4") && !has(&app, "Member 3"), "the search filters the members");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("member:{id} @m0:beeper.local"))));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Call").is_ok() && sim.find("Copy Phone Number").is_ok() && sim.find("Copy Email").is_err());
    let _ = sim.click("Copy Name").expect("the menu copies the name");
    assert!(
        matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::Message(message::Msg::CopyText(t))] if t == "Member 0")
    );
}

#[test]
fn chat_info_notes_are_local_per_chat() {
    let mut app = demo_app("chat-info-notes");
    let (a, b) = ("!tg_rust:beeper.com".to_string(), "!note_self:beeper.com".to_string());
    let _ = app.update(UiMessage::SelectChat(a.clone()));
    let paste = |s: &str| text_editor::Action::Edit(text_editor::Edit::Paste(std::sync::Arc::new(s.into())));
    let _ = app.update(UiMessage::Info(info::Msg::Note(paste("Ask about the release"))));
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
    assert!(
        matches!(&sim.into_messages().collect::<Vec<_>>()[..], [UiMessage::ChatList(chat_list::Msg::AskLeave(c))] if *c == id)
    );
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::AskLeave(id.clone())));
    let msgs: Vec<UiMessage> = {
        let mut sim = simulator(app.view_confirm(ASK_LEAVE_CHAT));
        assert!(sim.find("Are you sure you want to leave Beeper Rust Core Team?").is_ok());
        let _ = sim.click("Leave").expect("a Leave button");
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.confirm.is_none() && app.store.read().chat(&id).exited(), "the chat stays, left");
    let composer = iced::widget::Id::new("composer-input");
    assert!(simulator(app.view_conversation()).find(composer).is_err(), "without a composer");
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
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
        simulator(app.view_main()).find("Edit Group...").is_ok()
    };
    set_edits(&mut app, 0);
    assert!(!in_menu(&mut app), "a group without edits has no item");
    set_edits(&mut app, ALL_EDITS);
    assert!(in_menu(&mut app));
    let _ = app.update(UiMessage::OpenEditGroup(id.clone()));
    let d = app.group_window.as_ref().expect("the window opens");
    assert!(app.list.chat_menu.is_none() && d.title == "Beeper Rust Core Team");
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
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(app.group_window.as_ref().expect("open").id)));
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

/// Desktop's `EditGroupModal`: Save waits for the homeserver with "Saving" and Cancel off, applies the changes once it
/// takes them, keeps the window with "Failed to update group" after an error, and a dragged file shows the drop zone.
#[test]
fn edit_group_saves_like_desktop() {
    let mut app = demo_app("edit-group-save");
    let id = "!tg_rust:beeper.com".to_string();
    let _ = app.update(UiMessage::OpenEditGroup(id.clone()));
    let _ = app.update(UiMessage::Group(GroupEdit::Title("Core".into())));
    app.group_window.as_mut().expect("open").saving = true;
    let view = |app: &BeeperGuiApp| {
        simulator(app.view_edit_group(app.group_window.as_ref().expect("open"))).find("Saving").is_ok()
    };
    assert!(view(&app), "the save waits");
    let title = |app: &BeeperGuiApp| app.store.read().chat(&id).title.clone();
    let _ = app.update(UiMessage::Group(GroupEdit::Saved(Err("forbidden".into()))));
    assert!(app.group_window.is_some() && app.status_banner == "Failed to update group");
    assert!(!view(&app) && title(&app) == "Beeper Rust Core Team", "nothing changed");
    let _ = app.update(UiMessage::Group(GroupEdit::Saved(Ok(()))));
    assert_eq!(title(&app), "Core", "the homeserver took the title");
    let window = app.group_window.as_ref().expect("open").id;
    let _ = app.update(UiMessage::Composer(composer::Msg::FileHover(window, true)));
    let drop = |app: &BeeperGuiApp| {
        let mut sim = simulator(app.view_edit_group(app.group_window.as_ref().expect("open")));
        sim.find("Drop image here to upload").is_ok()
    };
    assert!(drop(&app), "a dragged file shows the drop zone");
    let _ = app.update(UiMessage::Composer(composer::Msg::FileHover(window, false)));
    assert!(!drop(&app));
}

#[test]
fn edit_for_me_like_desktop() {
    let mut app = demo_app("edit-for-me");
    let id = "!tg_rust:beeper.com".to_string();
    let room = "Beeper Rust Core Team";
    let chat = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chat(&id).clone()
    };
    let labels = |app: &BeeperGuiApp, key: &str| -> Vec<String> {
        info_menu(&app.store.read(), key, |_| true).into_iter().map(|i| i.map(|i| i.0).unwrap_or_default()).collect()
    };
    assert_eq!(labels(&app, &format!("info:{id}")), ["Edit Group", "Edit for Me", "Chat Theme"]);
    let _ = app.update(UiMessage::OpenEditForMe(id.clone()));
    let d = app.group_window.as_ref().expect("the window opens");
    assert!(d.personal && d.title.is_empty());
    assert_eq!(app.title(d.id), "Edit Chat for Me");
    {
        let mut sim = simulator(app.view_edit_group(d));
        assert!(sim.find("This name and photo are only visible to you, on all your devices.").is_ok());
        assert!(sim.find("Remove Photo").is_err(), "no personal photo to remove");
    }
    let _ = app.update(UiMessage::Group(GroupEdit::Title(format!("  Mine{}", "x".repeat(300)))));
    assert_eq!(app.group_window.as_ref().expect("open").title.chars().count(), PERSONAL_NAME_MAX);
    let _ = app.update(UiMessage::Group(GroupEdit::Title("  Mine  ".into())));
    let _ = app.update(UiMessage::Group(GroupEdit::Save));
    assert_eq!((chat(&app).title.as_str(), chat(&app).room_title()), ("Mine", room));
    assert_eq!(app.toast.as_ref().expect("a toast").text, "Renamed to Mine for you");
    assert_eq!(app.undo_stack.last().expect("an undo").0, format!("Rename {room} for you"));
    let _ = app.update(UiMessage::SelectChat(id.clone()));
    let open: Vec<UiMessage> = {
        let mut sim = tall(app.view_info());
        let line = format!("Originally {room}");
        let at = sim.find(line.as_str()).expect("the room name").visible_bounds().expect("visible").center();
        sim.point_at(at);
        let _ = sim.simulate(
            [mouse::Event::ButtonPressed(Button::Right), mouse::Event::ButtonReleased(Button::Right)]
                .map(iced::Event::Mouse),
        );
        sim.into_messages().collect()
    };
    let [UiMessage::ChatList(chat_list::Msg::OpenMenu(key))] = &open[..] else { panic!("{open:?}") };
    assert_eq!(
        labels(&app, key),
        [
            format!("Copy Originally {room}"),
            String::new(),
            "Edit Group".into(),
            "Edit for Me".into(),
            "Chat Theme".into()
        ]
    );
    let _ = app.update(UiMessage::UndoAction);
    assert!(chat(&app).title == room && chat(&app).personal.is_none(), "Undo removes the name");
    app.store.write().set_personal(&id, None, Some(Some(("mxc://x/me".into(), Vec::new(), "image/png".into()))));
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(app.group_window.as_ref().expect("open").id)));
    let _ = app.update(UiMessage::OpenEditForMe(id.clone()));
    let _ = simulator(app.view_edit_group(app.group_window.as_ref().expect("open")))
        .click("Remove Photo")
        .expect("the personal photo can be removed");
    let _ = app.update(UiMessage::Group(GroupEdit::Remove));
    let _ = app.update(UiMessage::Group(GroupEdit::Save));
    assert!(chat(&app).img_url.is_none() && chat(&app).personal.is_none());
    assert_eq!(app.toast.as_ref().expect("a toast").text, "Removed your photo for this chat");
}

#[test]
fn a_later_launch_shows_the_window_like_desktop() {
    let mut app = demo_app("secondinstance");
    let _ = app.update(UiMessage::RunHotkey("Minimize To Tray"));
    assert!(app.hidden);
    let _ = app.update(UiMessage::Window(windows::Msg::SecondInstance(vec!["--demo".into()])));
    assert!(!app.hidden, "Desktop's second-instance event shows the main window");
}

/// Desktop's `ga` (`mainWindowSetSize`): the onboarding window grows into the main window around its center and on
/// the screen, and shrinks back the same way.
#[test]
fn the_window_grows_around_its_center_like_desktop() {
    let screen = Size::new(1440.0, 900.0);
    let (at, size) = crate::grown(Point::new(555.0, 140.0), Size::new(330.0, 620.0), Size::new(1200.0, 800.0), screen);
    assert_eq!((at, size), (Point::new(120.0, 50.0), Size::new(1200.0, 800.0)), "centered where the onboarding was");
    let corner = crate::grown(Point::new(1100.0, 280.0), Size::new(330.0, 620.0), Size::new(1200.0, 800.0), screen);
    assert_eq!(corner.0, Point::new(240.0, 100.0), "kept on the screen");
    let small =
        crate::grown(Point::ORIGIN, Size::new(330.0, 620.0), Size::new(1200.0, 800.0), Size::new(1024.0, 700.0));
    assert_eq!(small, (Point::ORIGIN, Size::new(1024.0, 700.0)), "no bigger than the screen");
    let back = crate::grown(Point::new(120.0, 50.0), Size::new(1200.0, 800.0), Size::new(330.0, 620.0), screen);
    assert_eq!(back.0, Point::new(555.0, 140.0), "and back around the same center");
}

/// Desktop's `windowStateKeeper` saves a window's state a second after its last resize or move, so a crash keeps it,
/// and only the windows with a state save.
#[test]
fn window_changes_save_the_state_after_the_last_one_like_desktop() {
    let mut app = demo_app("windowsave");
    let main = app.main_window.expect("a main window");
    let resize =
        |app: &mut BeeperGuiApp| app.update(UiMessage::Window(windows::Msg::Resized(main, Size::new(900.0, 700.0))));
    assert!(resize(&mut app).units() > 0, "a resize waits to save");
    assert!(app.update(UiMessage::Window(windows::Msg::Moved(main))).units() > 0, "so does a move");
    let n = app.state_changes;
    let save = |app: &mut BeeperGuiApp, n| app.update(UiMessage::Window(windows::Msg::SaveState(main, n))).units();
    assert_eq!(save(&mut app, n - 1), 0, "a later change replaces the save of an earlier one");
    assert!(save(&mut app, n) > 0, "the last change saves");
    let other = window::Id::unique();
    let task = app.update(UiMessage::Window(windows::Msg::Moved(other)));
    assert_eq!((task.units(), app.state_changes), (0, n), "a window without a state saves nothing");
}

#[test]
fn main_window_keeps_its_state_like_desktop() {
    let mut app = demo_app("windowstate");
    let s = app.main_window_settings();
    assert_eq!((s.size, s.min_size, s.maximized), (Size::new(1200.0, 800.0), Some(Size::new(330.0, 400.0)), false));
    assert!(matches!(s.position, window::Position::Centered));
    let path = app.store.read().config.data_dir.join("window.json");
    write_window_state(&path, "main", false, Size::new(900.0, 700.0), Some(Point::new(40.0, 30.0)));
    write_window_state(&path, "main", true, Size::new(1440.0, 900.0), Some(Point::new(0.0, 0.0)));
    let s = app.main_window_settings();
    assert_eq!((s.size, s.maximized), (Size::new(900.0, 700.0), true), "a maximized window keeps its old bounds");
    assert!(matches!(s.position, window::Position::Specific(p) if p == Point::new(40.0, 30.0)));
    // Desktop's Media window keeps its state under `media-viewer`.
    write_window_state(&path, "media-viewer", false, Size::new(800.0, 600.0), Some(Point::new(10.0, 20.0)));
    let _ = app.update(UiMessage::Media(media_view::Msg::Open));
    assert_eq!(app.media_window.as_ref().unwrap().size, Size::new(800.0, 600.0), "the Media window opens at its size");
    assert_eq!(app.main_window_settings().size, Size::new(900.0, 700.0), "the main window keeps its state");
    // Desktop's chat windows keep the size of the last one under `pop-out-thread`.
    write_window_state(&path, "pop-out-thread", false, Size::new(640.0, 520.0), Some(Point::new(5.0, 5.0)));
    let _ = app.update(UiMessage::Window(windows::Msg::OpenChat(beeper_core::store::NOTE_CHAT_ID.into())));
    assert_eq!(app.chat_windows.last().map(|w| w.1.size), Some(Size::new(640.0, 520.0)), "a chat window opens at it");
    let main = app.main_window.expect("a main window");
    let _ = app.update(UiMessage::SelectChat(beeper_core::store::NOTE_CHAT_ID.into()));
    let title = app.store.read().chats.iter().find(|c| c.id == beeper_core::store::NOTE_CHAT_ID).unwrap().title.clone();
    assert_eq!(app.title(main), format!("Beeper | {title}"));
}

/// A new install on the sign-in window.
pub(crate) fn signed_out_app(name: &str) -> BeeperGuiApp {
    let config = CentralConfig::new(beeper_core::testing::temp_dir(name), "127.0.0.1".into(), 23391, None);
    let app = BeeperGuiApp::boot(StoreLock::shared(BeeperStore::from_config(config))).0;
    assert!(app.login_window.is_some() && app.main_window.is_none(), "a new install opens the sign-in window");
    app
}

/// The pixels of the sign-in window at points of its 330x620 layout.
fn login_pixels(app: &BeeperGuiApp, points: &[(u32, u32)]) -> Vec<Color> {
    let mut sim = Simulator::with_size(Default::default(), Size::new(330.0, 620.0), app.view_login());
    let dir =
        std::env::temp_dir().join(format!("beeper-rs-login-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _ = sim.snapshot(&app.iced_theme()).unwrap().matches_image(dir.join("login"));
    let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let shot = ::image::open(&png).unwrap().into_rgba8();
    std::fs::remove_dir_all(&dir).unwrap();
    let k = shot.width() / 330;
    points
        .iter()
        .map(|&(x, y)| Color::from_rgb8(shot[(x * k, y * k)][0], shot[(x * k, y * k)][1], shot[(x * k, y * k)][2]))
        .collect()
}

/// The sign-in window opens in the middle of the screen with Desktop's minimum size, the terms wrap like
/// `text-wrap: balance` after "terms", and Open Email asks for the default mail app.
#[test]
fn sign_in_window_details_like_desktop() {
    let s = BeeperGuiApp::login_window_settings();
    assert!(matches!(s.position, window::Position::Centered) && s.min_size == Some(Size::new(330.0, 400.0)));
    let sentence = "By continuing, you agree to our terms of use and have read our privacy policy.";
    let width = style::balanced_width(sentence, 278.0, 11.0);
    let first = style::text_width("By continuing, you agree to our terms", 11.0, iced::font::Weight::Normal);
    let more = style::text_width("By continuing, you agree to our terms of", 11.0, iced::font::Weight::Normal);
    assert!(first < width && width < more, "the first line ends with \"terms\", like Desktop's: {width}");
    let mut app = signed_out_app("sign-in-details");
    app.login.screen = login::Screen::Code;
    let open = app.login_act(login::Stop::OpenEmail);
    assert!(matches!(open, Some(UiMessage::Login(login::LoginMsg::OpenEmail))), "Open Email opens the mail app");
}

/// The sign-in window follows the system theme, like Desktop's `prefers-color-scheme`.
#[test]
fn sign_in_window_follows_the_system_theme() {
    let mut app = signed_out_app("sign-in-dark");
    for (mode, p) in [(iced::theme::Mode::Light, style::BEEPER_LIGHT), (iced::theme::Mode::Dark, style::BEEPER_DARK)] {
        let _ = app.update(UiMessage::Settings(settings::Msg::SystemTheme(mode)));
        // The frame, the card, and the Google button.
        let want = [p.window, p.primary_container, p.chip];
        assert_eq!(login_pixels(&app, &[(3, 3), (20, 60), (40, 526)]), want, "{mode:?}");
    }
}

/// Desktop's brand controls on the email screen: the label of the field floats over it once it has text, and Next
/// shows its loading dots on the disabled color while the request runs.
#[test]
fn sign_in_brand_controls_like_desktop() {
    use login::{LoginMsg, Screen};
    let mut app = signed_out_app("sign-in-brand");
    let _ = app.update(LoginMsg::Show(Screen::Email).into());
    // The heading spans x 30 to 300 at y 106 in Desktop 4.3.308, with its letter spacing, in its gradient colors.
    let row: Vec<(u32, u32)> = (0..330).map(|x| (x, 106)).collect();
    let ink: Vec<u32> = login_pixels(&app, &row).iter().zip(0..).filter(|(c, _)| c.g < 0.6).map(|(_, x)| x).collect();
    let (left, right) = (ink[0], ink[ink.len() - 1]);
    assert!(
        left.abs_diff(30) <= 1 && right.abs_diff(300) <= 1,
        "the heading has the width of Desktop's: {left}..{right}"
    );
    let next = (40, 571);
    assert!(login_pixels(&app, &[next])[0].b > 0.9, "Next has the brand gradient");
    // The floating label shows over the top edge of the field, at y 503.
    let label: Vec<(u32, u32)> = (55..95).map(|x| (x, 503)).collect();
    let inked = |app: &BeeperGuiApp| login_pixels(app, &label).iter().any(|c| c.r < 0.8);
    assert!(!inked(&app), "the label is the placeholder of the empty field");
    let _ = app.update(LoginMsg::Email("alex@example.com".into()).into());
    assert!(inked(&app), "the label floats over the field with text");
    app.login.busy = true;
    assert_eq!(login_pixels(&app, &[next]), [app.pal.chip], "a loading button is disabled");
}

/// Desktop's code screen: a code in the clipboard goes in when the window gets the focus, once, with a toast;
/// digits typed anywhere go into the code; and an error turns the slots red on no fill.
#[test]
fn sign_in_code_screen_like_desktop() {
    use login::{LoginMsg, Outcome, Screen};
    let mut app = signed_out_app("sign-in-code");
    let id = app.login_window.expect("the sign-in window");
    let _ = app.update(LoginMsg::Email("alex@example.com".into()).into());
    let _ = app.update(LoginMsg::Done(Ok(Outcome::Requested("req_1".into()))).into());
    assert_eq!(app.login.screen, Screen::Code);
    let _ = app.update(UiMessage::WindowText(id, "7".into()));
    assert_eq!(app.login.code, "7", "a digit typed in the window goes into the code");
    let _ = app.update(LoginMsg::KeyOnFocus(Some("12345".into())).into());
    assert_eq!(app.login.code, "7", "the clipboard pastes only a whole code");
    let _ = app.update(LoginMsg::KeyOnFocus(Some("123 456".into())).into());
    assert!(app.login.code == "123456" && app.login.busy, "a code in the clipboard goes in and is sent");
    assert!(simulator(app.view_login()).find("Code auto pasted from clipboard").is_ok());
    let refused = beeper_core::cloud::CloudError {
        status: 401,
        message: "Invalid code. Please try again.".into(),
        errcode: None,
        body: None,
    };
    let _ = app.update(LoginMsg::Done(Err(refused)).into());
    let _ = app.update(LoginMsg::KeyOnFocus(Some("123456".into())).into());
    assert!(!app.login.busy, "a code that was sent does not go again");
    // The middle of the first slot has no fill, and its ring is red.
    let red = Color::from_rgb8(220, 54, 46);
    assert_eq!(login_pixels(&app, &[(28, 375), (20, 393)]), [app.pal.primary_container, red]);
}

/// Desktop's username screen and the end of the onboarding: the sync icon goes through the suggestions, Next waits
/// for a valid name, Continue waits for a connected account, and the same window becomes the main window.
#[test]
fn sign_in_username_and_continue_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    use login::{LoginMsg, Outcome, Screen};
    let mut app = signed_out_app("sign-in-username");
    let id = app.login_window.expect("the sign-in window");
    let lead = Outcome::Lead("lead".into(), vec!["alex-rs".into(), "alex2".into()]);
    let _ = app.update(LoginMsg::Done(Ok(lead)).into());
    assert_eq!((app.login.screen, app.login.username.as_str()), (Screen::Username, "alex-rs"));
    let _ = app.update(LoginMsg::NextSuggestion.into());
    assert_eq!(app.login.username, "alex2", "the sync icon shows the next suggestion");
    let _ = app.update(LoginMsg::Username("SupportBot".into()).into());
    assert_eq!(app.login.username, "supportbot", "the field takes lower case letters, digits and dashes");
    // Desktop's rules refuse the name when its debounced check runs, without asking the server.
    let _ = app.update(LoginMsg::CheckUsername(app.login.check).into());
    assert!(simulator(app.view_login()).find("Username is not available. Try something else.").is_ok());
    let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Enter), Modifiers::empty()));
    assert!(!app.login.busy, "Enter does not send a name that Next refuses");
    let _ = app.update(LoginMsg::Username("alex-rs".into()).into());
    let _ = app.update(LoginMsg::UsernameChecked(app.login.check, Ok(true)).into());
    let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Enter), Modifiers::empty()));
    assert!(app.login.busy, "Enter sends a valid name");

    app.login.screen = Screen::Accounts;
    assert!(app.store.read().bridge_accounts.is_empty());
    assert!(simulator(app.view_login()).find("Continue").is_err(), "Continue waits for a connected account");
    let mut done = demo_app("sign-in-continue");
    (done.login_window, done.main_window) = (done.main_window, None);
    done.login.screen = Screen::Accounts;
    assert!(simulator(done.view_login()).find("Continue").is_ok());
    let clear = icon::CLOSE.to_string();
    assert!(simulator(done.view_login()).find(clear.as_str()).is_err(), "an empty search has no clear icon");
    let _ = done.update(LoginMsg::Query("tele".into()).into());
    let mut sim = simulator(done.view_login());
    let _ = sim.click(clear.as_str()).expect("the clear icon of the search");
    for m in sim.into_messages() {
        let _ = done.update(m);
    }
    assert!(simulator(done.view_login()).find(clear.as_str()).is_err(), "the clear icon empties the search");
    let window = done.login_window;
    let _ = done.update(LoginMsg::Continue.into());
    assert_eq!((done.login_window, done.main_window), (None, window), "the sign-in window becomes the main window");
    let mut none = demo_app("sign-in-headline");
    (none.login_window, none.main_window) = (none.main_window, None);
    none.login.screen = Screen::Accounts;
    for _ in 0..9 {
        let _ = none.update(LoginMsg::Headline.into());
    }
    assert!(none.main_window.is_none(), "nine clicks on the heading keep the onboarding");
    let _ = none.update(LoginMsg::Headline.into());
    assert!(none.main_window.is_some(), "Desktop's tenth click on the heading finishes the onboarding");
}

/// The keyboard of the sign-in window, like Desktop: Tab goes through the controls in their order, Enter presses
/// the focused control or submits the form, and Escape closes the menu. Errors show in the toast, and an email
/// that is not valid shows Chromium's message.
#[test]
fn sign_in_keyboard_and_errors_like_desktop() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    use login::{LoginMsg, Screen, Stop};
    let mut app = signed_out_app("sign-in-keyboard");
    let id = app.login_window.expect("the sign-in window");
    let key = |app: &mut BeeperGuiApp, k: Named, mods: Modifiers| {
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(k), mods));
        app.login.focus.get()
    };
    let tabs: Vec<_> = (0..5).map(|_| key(&mut app, Named::Tab, Modifiers::empty())).collect();
    let order = [Stop::Menu, Stop::Terms, Stop::Google, Stop::Email, Stop::Menu].map(Some);
    assert_eq!(tabs, order, "Tab cycles the chevron, the terms link, Google and Email");
    assert_eq!(key(&mut app, Named::Tab, Modifiers::SHIFT), Some(Stop::Email), "Shift+Tab goes back");
    let shows = |app: &BeeperGuiApp, label: &str| simulator(app.view_login()).find(label).is_ok();
    key(&mut app, Named::Tab, Modifiers::SHIFT);
    let _ = app.update(UiMessage::WindowText(id, " ".into()));
    assert!(shows(&app, "Google sign-in is not available in beeper-rs. Use email."), "Space presses Google");
    let _ = app.update(LoginMsg::CloseToast.into());
    assert!(app.login.toast.is_none(), "the close button of the toast hides it");
    key(&mut app, Named::Tab, Modifiers::empty());
    key(&mut app, Named::Enter, Modifiers::empty());
    assert_eq!(
        (app.login.screen, app.login.focus.get()),
        (Screen::Email, Some(Stop::Field)),
        "Enter opens the email form"
    );
    key(&mut app, Named::Enter, Modifiers::empty());
    assert_eq!(app.login.toast.as_deref(), Some("Please enter your email."));
    let _ = app.update(LoginMsg::Email("foo".into()).into());
    assert!(app.login.toast.is_none(), "a change of the email hides the toast");
    key(&mut app, Named::Enter, Modifiers::empty());
    let missing = "Please include an '@' in the email address. 'foo' is missing an '@'.";
    assert!(shows(&app, missing) && !app.login.busy, "Chromium checks the email before a request");
    for (screen, items) in [
        (Screen::Welcome, &["Report a Problem"][..]),
        (Screen::Email, &["Go Back", "Report a Problem"]),
        (Screen::Code, &["Go Back", "Report a Problem", "Start Over"]),
        (Screen::Username, &["Report a Problem", "Start Over"]),
    ] {
        app.login.screen = screen;
        let _ = app.update(LoginMsg::Menu.into());
        for item in ["Go Back", "Report a Problem", "Start Over"] {
            assert_eq!(shows(&app, item), items.contains(&item), "{item} in the menu of {screen:?}");
        }
        key(&mut app, Named::Escape, Modifiers::empty());
        assert!(!shows(&app, "Report a Problem"), "Escape closes the menu");
    }
}

/// Every hotkey and tray action on the sign-in window, before and after sign-in. None panics, and Ctrl+, opens the
/// Settings window, which Desktop draws empty before sign-in.
#[test]
fn hotkeys_on_the_sign_in_window_like_desktop() {
    let combo = |keys: &str| {
        let parts: Vec<&str> = keys.split('+').collect();
        let mods = [("ctrl", Modifiers::CTRL), ("shift", Modifiers::SHIFT), ("alt", Modifiers::ALT)];
        let mods = mods.into_iter().filter(|(m, _)| parts.contains(m)).fold(Modifiers::empty(), |a, (_, m)| a | m);
        let k = match *parts.last().expect("a key") {
            "ctrl" | "shift" | "alt" => Key::Character("1".into()),
            "up" => Key::Named(Named::ArrowUp),
            "down" => Key::Named(Named::ArrowDown),
            "left" => Key::Named(Named::ArrowLeft),
            "right" => Key::Named(Named::ArrowRight),
            "enter" => Key::Named(Named::Enter),
            "tab" => Key::Named(Named::Tab),
            "escape" => Key::Named(Named::Escape),
            c => Key::Character(c.into()),
        };
        (k, mods)
    };
    let all = prefs::HOTKEYS.iter().chain(prefs::FIXED_HOTKEYS).flat_map(|h| h.2.split(", "));
    let keys: Vec<(Key, Modifiers)> = all.filter(|k| *k != "disabled").map(combo).collect();
    let mut signed_out = signed_out_app("sign-in-keys");
    let mut accounts = demo_app("sign-in-accounts");
    (accounts.login_window, accounts.main_window, accounts.pane.selected_chat_id) = (accounts.main_window, None, None);
    accounts.login.screen = login::Screen::Accounts;
    for app in [&mut signed_out, &mut accounts] {
        for (key, mods) in keys.iter().cloned() {
            for id in [app.login_window, app.settings_window].into_iter().flatten() {
                let _ = app.update(UiMessage::WindowKey(id, key.clone(), mods));
            }
        }
        for e in [tray::Event::Up, tray::Event::Activate, tray::Event::Hide, tray::Event::Show] {
            let _ = app.update(UiMessage::Window(windows::Msg::Tray(Ok(e))));
        }
    }
    let about = SettingsTab::About.name();
    let blank = signed_out.settings_window.expect("Ctrl+, opens the Settings window");
    assert!(simulator(signed_out.view(blank)).find(about).is_err(), "Desktop draws nothing in it before sign-in");
    let settings = accounts.settings_window.expect("Ctrl+, opens the Settings window");
    assert!(simulator(accounts.view(settings)).find(about).is_ok(), "the Accounts screen comes after sign-in");
}

/// Desktop's onboarding window has the menu bar too: Alt shows it, its items act on the sign-in window, and Escape
/// hides it. Ctrl+/ shows the Keyboard Shortcuts over the sign-in window, and Escape hides them.
#[test]
fn the_sign_in_window_has_the_menu_bar_and_the_shortcuts_like_desktop() {
    let mut app = signed_out_app("sign-in-menu-bar");
    let id = app.login_window.expect("the sign-in window");
    let alt = |app: &mut BeeperGuiApp| {
        let _ = app.update(UiMessage::Modifiers(Modifiers::ALT));
        let _ = app.update(UiMessage::Modifiers(Modifiers::empty()));
    };
    alt(&mut app);
    assert_eq!(app.menu_bar, Some(None), "Alt shows the menu bar");
    let shows = |app: &BeeperGuiApp, label: &str| simulator(app.view(id)).find(label).is_ok();
    assert!(shows(&app, "Window") && shows(&app, "Continue with Email"), "the menu bar is over the sign-in screen");
    let _ = app.update(UiMessage::MenuBar(menubar::Msg::Show(Some(Some(0)))));
    for item in ["About Beeper", "Minimize to Tray", "Alt+Ctrl+Q", "Logout of Beeper...", "Quit Beeper", "Ctrl+Q"] {
        assert!(shows(&app, item), "the Beeper menu has {item}");
    }
    assert!(!shows(&app, "Preferences"), "the items of a signed-in account wait for the sign-in");
    let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), Modifiers::empty()));
    assert_eq!(app.menu_bar, None, "Escape hides the menu bar");
    let _ = app.update(UiMessage::WindowKey(id, Key::Character("/".into()), Modifiers::CTRL));
    assert!(shows(&app, "Keyboard Shortcuts") && shows(&app, "Bold Text Selection"), "Ctrl+/ shows the shortcuts");
    let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), Modifiers::empty()));
    assert!(!shows(&app, "Bold Text Selection"), "Escape hides the shortcuts");
    // Every item before sign-in acts on the sign-in window or does nothing, without a panic.
    for (_, items) in app.menus() {
        for (_, _, msg) in items.into_iter().flatten() {
            let _ = app.update(UiMessage::MenuBar(menubar::Msg::Run(Box::new(msg))));
        }
    }
    let _ = app.update(UiMessage::Window(windows::Msg::Tray(Ok(tray::Event::Show))));
    // The chevron menu shows the accelerator of Report a Problem like Desktop's native menu.
    let _ = app.update(login::LoginMsg::Menu.into());
    assert!(shows(&app, "Ctrl+Shift+P"));
}

/// Electron's native menus write the modifiers of an accelerator in Chromium's order, with "+" between the keys.
#[test]
fn native_menus_write_accelerators_like_electron() {
    for (keys, label) in [
        ("ctrl+alt+q", "Alt+Ctrl+Q"),
        ("ctrl+shift+p", "Ctrl+Shift+P"),
        ("ctrl+,", "Ctrl+,"),
        ("ctrl+/, ctrl+shift+/", "Ctrl+/"),
        ("disabled", ""),
    ] {
        assert_eq!(menubar::accelerator(keys), label, "{keys}");
    }
}

/// Desktop's `BrandButton` shrinks to 98 % while the pointer presses it, and not while it is disabled.
#[test]
fn brand_buttons_shrink_while_pressed_like_desktop() {
    // The pixel at the left edge of the button at y 570, which spans x 26 to 304, with the pointer down on it or not.
    let edge = |app: &BeeperGuiApp, press: bool| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(330.0, 620.0), app.view_login());
        sim.point_at(Point::new(165.0, 570.0));
        if press {
            let _ = sim.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonPressed(Button::Left))]);
        }
        let dir = std::env::temp_dir().join(format!("beeper-rs-press-{}-{press}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = sim.snapshot(&app.iced_theme()).unwrap().matches_image(dir.join("press"));
        let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let shot = ::image::open(&png).unwrap().into_rgba8();
        std::fs::remove_dir_all(&dir).unwrap();
        let k = shot.width() / 330;
        let [r, g, b, _] = shot[(28 * k, 570 * k)].0;
        Color::from_rgb8(r, g, b)
    };
    let mut app = signed_out_app("sign-in-press");
    let card = app.pal.primary_container;
    assert_ne!(edge(&app, false), card, "Continue with Email covers its left edge");
    assert_eq!(edge(&app, true), card, "a pressed button shrinks from its left edge");
    let _ = app.update(login::LoginMsg::Show(login::Screen::Email).into());
    app.login.busy = true;
    assert_eq!(edge(&app, true), app.pal.chip, "a disabled Next does not shrink");
}

/// Each hotkey with no open chat does nothing or acts on the app, like Desktop.
#[test]
fn hotkeys_without_an_open_chat_like_desktop() {
    let mut app = demo_app("no-chat-keys");
    // The composer runs Undo Pending Send itself.
    for (name, ..) in prefs::HOTKEYS.iter().filter(|h| h.0 != "Undo Pending Send") {
        app.pane.selected_chat_id = None;
        let _ = app.update(UiMessage::RunHotkey(name));
    }
}

/// Desktop's `callAction`: the call of a bridge shows as a call pill without its text, the chat list shows a call
/// badge, and nobody reacts to it.
#[test]
fn calls_like_desktop() {
    use beeper_core::models::CallAction;
    let mut app = demo_app("calls");
    let chat = "!wa_eric:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let content = |kind: Option<&str>| {
        let action = serde_json::json!({ "type": "call", "call_type": kind });
        serde_json::json!({ "msgtype": "m.text", "body": "Missed voice call", "com.beeper.action_message": action })
    };
    assert_eq!(CallAction::of(&serde_json::json!({ "body": "hi" })), None);
    let call = |n: i64, kind: Option<&str>| {
        let st = app.store.read();
        let last = st.timeline_indices(chat).last().map(|&i| st.messages[i].clone()).expect("a message");
        let sort_key = beeper_core::store::sort_key(last.sort_key.parse::<i64>().expect("a sort key") + n);
        Message {
            id: format!("call_{n}"),
            sort_key,
            text: Some("Missed voice call".into()),
            call: CallAction::of(&content(kind)),
            ..last
        }
    };
    let voice = call(1, Some("voice"));
    assert!(!voice.reactable(), "nobody reacts to a call");
    assert_eq!(
        (crate::chat_list::message_tag(&voice), crate::chat_list::preview_body(&voice)),
        (Some((crate::chat_list::CALL_TAG, "Voice call", None)), None),
        "the chat list shows the badge alone"
    );
    let video = call(2, Some("video"));
    assert_eq!(crate::chat_list::message_tag(&video), Some((crate::chat_list::VIDEO_TAG, "Video call", None)));
    let other = call(3, None);
    assert_eq!(crate::chat_list::message_tag(&other).map(|t| t.1), Some("Call"));
    app.store.write().upsert_message(voice);
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Voice call").is_ok(), "the call pill");
    assert!(sim.find("Missed voice call").is_err(), "without the text of the bridge");
}

/// Desktop's `BrandChatInfoNotificationSound`: Chat Info has the Notification sound select, which sets the chat sound.
#[test]
fn chat_info_sets_the_notification_sound_like_desktop() {
    let mut app = demo_app("chat-sound");
    let chat = "!wa_eric:beeper.com";
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    {
        let mut sim = tall(app.view_info());
        assert!(sim.find("Notification sound").is_ok());
        assert!(sim.find("Silent chats still show notifications, just without a sound.").is_ok());
    }
    let _ = app.update(UiMessage::Info(crate::info::Msg::SetSound(chat.into(), Some("silent".into()))));
    assert_eq!(app.store.read().chat(chat).notification_sound.as_deref(), Some("silent"));
}

/// Desktop's space sheet is as wide as its longest space name, which stays on one line.
#[test]
fn the_space_sheet_fits_its_names_like_desktop() {
    let app = demo_app("space-sheet");
    let narrow = app.view_space_sheet().1;
    {
        let mut st = app.store.write();
        let kind = |a: &str| account_network(a).kind;
        let other =
            |b: &&BridgeAccountInfo| !matches!(kind(&b.account_id), NetworkKind::WhatsApp | NetworkKind::Telegram);
        let account = st.bridge_accounts.iter().find(other).expect("a network with spaces").account_id.clone();
        let name = "supersketchy's very long server name".into();
        st.network_spaces.push(NetworkSpace {
            id: "!long:beeper.local".into(),
            account_id: account,
            name,
            avatar: None,
        });
    }
    let wide = app.view_space_sheet().1;
    assert_eq!(narrow, 233.0, "short names keep the usual width");
    assert!(wide > 233.0, "a long name widens the sheet: {wide}");
}

/// Desktop's `sit` and `ihe`: a contact of a bridge's contact list with a Matrix user and no phone number shows in
/// New Chat, and its own account offers that user at once.
#[test]
fn matrix_contacts_start_chats_like_desktop() {
    let mut app = demo_app("matrix-contact");
    let account = {
        let st = app.store.read();
        let c = st.chats.iter().find(|c| c.chat_type == "single" && c.account_id != "hungryserv").expect("a DM");
        c.account_id.clone()
    };
    let caps = serde_json::json!({ "resolve_identifier": { "create_dm": true, "contact_list": true } });
    let _ = app.update(UiMessage::Capabilities(account.clone(), Ok(caps)));
    let id = format!("contact:{account}:@signal_alyson:beeper.local");
    let alyson = User {
        id: id.clone(),
        full_name: Some("Alyson Chang".into()),
        mxid: Some("@signal_alyson:beeper.local".into()),
        ..Default::default()
    };
    app.store.write().set_contacts(vec![alyson]);
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert!(simulator(app.view_main()).find("Alyson Chang").is_ok(), "listed without a phone number");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::ToggleContact(id)));
    let nets = &app.contact_networks.as_ref().expect("the networks").1;
    let user = nets.iter().find(|(a, _)| *a == account).and_then(|(_, u)| u.clone().flatten()).expect("a user");
    assert_eq!((user.id.as_str(), user.full_name.as_deref()), ("@signal_alyson:beeper.local", Some("Alyson Chang")));
}

/// Desktop's `isDraftEmpty`: leaving a New Chat draft without people or text removes it, and a draft with a person stays.
#[test]
fn empty_new_chat_drafts_go_like_desktop() {
    let mut app = demo_app("empty-drafts");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert_eq!(app.new_chats.len(), 1);
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    assert!(app.new_chats.is_empty(), "the empty draft goes");
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::Open));
    assert_eq!(app.new_chats.len(), 1, "a new draft replaces an empty one");
    let account =
        app.store.read().chats.iter().find(|c| c.account_id != "hungryserv").expect("a chat").account_id.clone();
    let user = User { id: "@sam:x".into(), full_name: Some("Sam".into()), ..Default::default() };
    let _ = app.update(UiMessage::NewChat(new_chat::Msg::PickUser(account, Box::new(user))));
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    assert_eq!(app.new_chats.len(), 1, "a draft with a person stays");
}

/// Desktop's `profileLink`: the network line of a DM with a profile URL opens the profile.
#[test]
fn chat_info_opens_the_profile_like_desktop() {
    let mut app = demo_app("profile-link");
    let chat = {
        let mut st = app.store.write();
        let i = st
            .chats
            .iter()
            .position(|c| c.chat_type == "single" && crate::profile_link(c).is_none() && c.account_id != "hungryserv");
        let i = i.expect("a DM");
        let account = st.bridge_accounts.iter().find(|b| account_network(&b.account_id).kind == NetworkKind::Instagram);
        let account = account.expect("an Instagram account").account_id.clone();
        st.update_chat(i, |c| {
            c.account_id = account;
            let other = c.participants.items.iter_mut().find(|u| u.is_self != Some(true)).expect("the other person");
            other.username = Some("arnayb_".into());
        });
        st.chats[i].id.clone()
    };
    let _ = app.update(UiMessage::SelectChat(chat));
    let mut sim = tall(app.view_info());
    let _ = sim.click("Instagram").expect("the network link");
    let opened: Vec<String> = sim
        .into_messages()
        .filter_map(|m| match m {
            UiMessage::OpenUrl(u) => Some(u),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["https://www.instagram.com/arnayb_/"]);
}

/// A bridge can send media without a file name or body: the timeline shows it, and a file row says "Attachment" like
/// Desktop.
#[test]
fn nameless_attachments_like_desktop() {
    let mut app = demo_app("nameless");
    let chat = "!wa_eric:beeper.com";
    {
        let mut st = app.store.write();
        let i = *st.timeline_indices(chat).last().expect("a message");
        let sticker = Attachment { attachment_type: "img".into(), is_sticker: Some(true), ..Attachment::default() };
        st.update_message(i, |m| m.attachments = vec![sticker]);
    }
    let _ = app.update(UiMessage::SelectChat(chat.into()));
    let _ = simulator(app.view_main());
    let a = Attachment { attachment_type: "unknown".into(), ..Attachment::default() };
    assert_eq!(crate::message::attachment_name(&a), "Attachment");
}

/// Desktop's "Edit Account Nickname…" (`Zpe`) after Edit Spaces… in the account menus of the sidebar and of the
/// Accounts settings (`rj`), for an account with a bridge login other than the Beeper account.
#[test]
fn account_nickname_menus_like_desktop() {
    let mut app = demo_app("nickname-menus");
    let item = "Edit Account Nickname…";
    let top = |sim: &mut Simulator<'_, UiMessage>, label: &str| {
        sim.find(label).expect(label).visible_bounds().expect("visible").y
    };
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("account:acc_whatsapp".into())));
    let open: Vec<UiMessage> = {
        let mut sim = simulator(app.view_account_menu("acc_whatsapp"));
        let ys = ["Edit Spaces…", item, "Add/Remove Accounts…"].map(|l| top(&mut sim, l));
        assert!(ys[0] < ys[1] && ys[1] < ys[2], "Desktop's order: {ys:?}");
        let _ = sim.click(item).expect("the nickname item");
        sim.into_messages().collect()
    };
    assert!(matches!(&open[..], [UiMessage::Nickname(nickname::Msg::Open(a))] if a == "acc_whatsapp"));
    assert!(simulator(app.view_account_menu("hungryserv")).find(item).is_err(), "the Beeper account has none");
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("prefs:acc_telegram".into())));
    let mut sim = simulator(app.view_settings());
    let ys = ["Edit Spaces…", item, "Remove Account…"].map(|l| top(&mut sim, l));
    assert!(ys[0] < ys[1] && ys[1] < ys[2], "Desktop's order: {ys:?}");
    let _ = sim.click(item).expect("the nickname item of the Accounts settings");
    let open: Vec<UiMessage> = sim.into_messages().collect();
    assert!(matches!(&open[..], [UiMessage::Nickname(nickname::Msg::Open(a))] if a == "acc_telegram"));
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("prefs:hungryserv".into())));
    assert!(simulator(app.view_settings()).find(item).is_err());
}

/// Desktop's Account Nickname window (`RenameAccount`): the name of the account, with its default name as the
/// placeholder, the user of the account, and Save. The nickname then names the account wherever Desktop shows
/// `customName`: the space, the account rows of the settings, and the account's user text.
#[test]
fn account_nickname_window_like_desktop() {
    let mut app = demo_app("nickname-window");
    let account = "acc_whatsapp";
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Open(account.into())));
    let id = app.nickname_windows.first().expect("the window opens").id;
    assert_eq!(app.title(id), "Account Nickname");
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Open(account.into())));
    assert_eq!(app.nickname_windows.len(), 1, "an account has one window");
    {
        let d = &app.nickname_windows[0];
        assert_eq!(d.name, "", "no nickname yet");
        let mut sim = simulator(app.view_nickname(d));
        let phone = fmt_phone("+14155550142");
        let hint = "Changes how this account appears in Beeper, on all your devices.";
        for text in ["Account Nickname", hint, "Alex Rivera", phone.as_str(), "Cancel", "Save"] {
            assert!(sim.find(text).is_ok(), "{text}");
        }
    }
    {
        let st = app.store.read();
        let placeholder = new_chat::default_account_display_name(&st, account, &rail_accounts(&st));
        assert_eq!(placeholder, "WhatsApp");
    }
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Name(id, "  Work phone ".into())));
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Save(id)));
    {
        let st = app.store.read();
        assert_eq!(st.custom_name(account), Some("Work phone"), "the demo keeps the trimmed name");
        assert_eq!(space_name(&st, account), "Work phone", "Desktop's accountDisplayName");
        assert_eq!(account_user(&st, account), "Work phone", "Desktop's userDisplayText");
        assert_eq!(account_display_name(&st, account, &rail_accounts(&st)), "Work phone");
    }
    let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
    assert!(app.nickname_windows.is_empty());
    let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
    assert!(simulator(app.view_settings()).find("Work phone").is_ok(), "the account row shows the nickname");
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Open(account.into())));
    let id = app.nickname_windows[0].id;
    assert_eq!(app.nickname_windows[0].name, "Work phone", "the window starts with the nickname");
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Saved(id, Err("offline".into()))));
    let error = "Something went wrong: offline";
    assert!(simulator(app.view_nickname(&app.nickname_windows[0])).find(error).is_ok(), "the error toast");
    let _ = app.update(UiMessage::Nickname(nickname::Msg::HideError(id, error.into())));
    assert!(simulator(app.view_nickname(&app.nickname_windows[0])).find(error).is_err());
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Name(id, String::new())));
    let _ = app.update(UiMessage::Nickname(nickname::Msg::Save(id)));
    let st = app.store.read();
    assert_eq!(st.custom_name(account), None, "an empty name removes the nickname");
    assert_eq!(space_name(&st, account), "WhatsApp");
}

/// Desktop's dot of new requests: the requests that came after the Requests view last showed their account, by
/// `com.beeper.message_requests.last_viewed_at`, whatever their unread state.
#[test]
fn requests_dot_like_desktop() {
    let mut app = demo_app("requests-dot");
    let id = {
        let st = app.store.read();
        let free = |c: &&Chat| c.id != NOTE_CHAT_ID && c.is_pinned != Some(true) && c.account_id == "acc_signal";
        st.chats.iter().find(free).expect("a Signal chat").id.clone()
    };
    {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the chat");
        st.update_chat(i, |c| {
            (c.is_request, c.flags, c.unread_count) = (Some(true), (c.flags | CHAT_REQUEST) & !CHAT_UNREAD, 0)
        });
    }
    let new = |app: &BeeperGuiApp| {
        let st = app.store.read();
        st.chats.iter().filter(|c| st.new_request(c)).count()
    };
    assert_eq!(new(&app), 1, "a request that was never viewed is new, also when read");
    let key = {
        let st = app.store.read();
        st.request_key(st.chat(&id)).expect("a bridge login")
    };
    assert_eq!(key, "signal.login_signal_01", "Desktop's <bridge ID>.<login ID>");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Requests)));
    assert_eq!(new(&app), 0, "the Requests view saw it");
    app.store.write().merge_requests_viewed(&serde_json::json!({ key.clone(): 1 }));
    assert_eq!(new(&app), 0, "another device's older time keeps the newer one");
    {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&id).expect("the chat");
        let later = st.requests_viewed[&key] + 1;
        st.update_chat(i, |c| c.created_at = Some(later));
    }
    assert_eq!(new(&app), 1, "a request created after the view is new");
}

/// Desktop's press on a trigger dismisses its tooltip: the "Filter" tooltip does not cover the filter menu.
#[test]
fn no_tooltip_over_an_open_menu_like_desktop() {
    let mut app = demo_app("tooltip-menu");
    // The simulator does not find the text of overlays, so the test reads a pixel of the tooltip left of the menu.
    let tip_pixel = |app: &BeeperGuiApp, name: &str| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view_main());
        let at = Point::new(235.0, 26.0);
        sim.point_at(at);
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
        let dir = std::env::temp_dir().join(format!("beeper-rs-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = sim.snapshot(&iced::Theme::Light).unwrap().matches_image(dir.join("tip"));
        let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let shot = ::image::open(&png).unwrap().into_rgba8();
        std::fs::remove_dir_all(&dir).unwrap();
        let k = shot.width() / 1200;
        shot.get_pixel(214 * k, 57 * k).0
    };
    assert_eq!(tip_pixel(&app, "tip-shown"), [255; 4], "the filter button has its tooltip");
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu));
    assert_ne!(tip_pixel(&app, "tip-hidden"), [255; 4], "not over the open filter menu");
}

/// Desktop's keys of pending attachments (`C_t`): ← and → move between them, Space opens a Quick Look that follows
/// the focus, Backspace removes one and focuses the one before, and → past the last goes back to the input.
#[test]
fn pending_attachment_keys_like_desktop() {
    use iced::keyboard::{key::Named, Key};
    let mut app = demo_app("attachment-keys");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let mut png = Vec::new();
    ::image::RgbaImage::new(30, 20).write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).unwrap();
    let file = |name: &str, bytes: &[u8]| {
        let path = std::env::temp_dir().join(format!("beeper-rs-keys-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let f = portal::load_file(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        PendingMedia { name: name.into(), ..f }
    };
    app.pane.attachments = vec![file("a.png", &png), file("b.pdf", b"%PDF"), file("c.png", &png)];
    let press = |app: &mut BeeperGuiApp, k: Named| app.update(UiMessage::KeyPressed(Key::Named(k), Default::default()));
    let _ = app.update(UiMessage::Composer(composer::Msg::FocusAttachment(2)));
    let _ = press(&mut app, Named::ArrowLeft);
    assert_eq!(app.pane.att_focus, Some(1));
    let _ = press(&mut app, Named::ArrowLeft);
    let _ = press(&mut app, Named::ArrowLeft);
    assert_eq!(app.pane.att_focus, Some(0), "the first one keeps the focus");
    let _ = press(&mut app, Named::Space);
    assert_eq!(app.pane.quick_look, Some(0));
    assert!(app.view_attachment_quick_look().is_some(), "the Quick Look shows");
    let _ = press(&mut app, Named::ArrowRight);
    assert_eq!((app.pane.att_focus, app.pane.quick_look), (Some(1), Some(1)), "the Quick Look follows the focus");
    let _ = press(&mut app, Named::Escape);
    assert_eq!((app.pane.att_focus, app.pane.quick_look), (Some(1), None));
    let _ = press(&mut app, Named::Backspace);
    let names: Vec<&str> = app.pane.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!((names, app.pane.att_focus), (vec!["a.png", "c.png"], Some(0)), "the one before takes the focus");
    let _ = press(&mut app, Named::ArrowRight);
    let _ = press(&mut app, Named::ArrowRight);
    assert_eq!(app.pane.att_focus, None, "past the last one, the input");
    drop(simulator(app.view_main()));
}

/// Desktop's hover actions of a chat row fade and grow in over 0.1 s, and back out after the pointer leaves.
#[test]
fn row_hover_actions_fade_like_desktop() {
    let mut app = demo_app("row-hover-fade");
    let id = app.visible_chats(&app.store.read())[0].id.clone();
    let hover = |app: &mut BeeperGuiApp, on| app.update(UiMessage::ChatList(chat_list::Msg::Hover(id.clone(), on)));
    let _ = hover(&mut app, true);
    assert!(app.hover_progress(&id).is_some_and(|t| t < 0.5), "the actions start small and clear");
    std::thread::sleep(std::time::Duration::from_millis(120));
    assert_eq!(app.hover_progress(&id), Some(1.0), "and are in after the transition");
    let _ = hover(&mut app, false);
    assert!(app.hover_progress(&id).is_some_and(|t| t > 0.5), "they fade back out");
    std::thread::sleep(std::time::Duration::from_millis(120));
    assert_eq!(app.hover_progress(&id), None, "and are gone after it");
}

/// Desktop's view-limited media: a pill in place of the media, the viewer that counts a view, the Play queue of the
/// chat list, the time limit, the HEIC toast and the expired media.
#[test]
fn view_once_media_like_desktop() {
    use crate::view_once::{Msg, Shown};
    let mut app = demo_app("view-once");
    let b = app.store.read().bridge_accounts.iter().find(|b| !b.login_id.is_empty()).cloned().expect("a bridge login");
    let them = |c: &Chat| c.participants.items.iter().find(|u| u.is_self != Some(true)).map(|u| u.id.clone());
    let st = app.store.read();
    let chat = st.chats.iter().find(|c| c.account_id == b.account_id && c.chat_type == "single" && them(c).is_some());
    let chat = chat.cloned().expect("a direct chat of the login");
    drop(st);
    let _ = app.update(UiMessage::SelectChat(chat.id.clone()));
    let media = |app: &BeeperGuiApp, n: i64, limit: serde_json::Value| {
        let st = app.store.read();
        let last = st.timeline_indices(&chat.id).last().map(|&i| st.messages[i].clone()).expect("a message");
        let a = Attachment {
            attachment_type: "img".into(),
            src_url: Some(format!("mxc://x/{n}")),
            mime_type: Some("image/jpeg".into()),
            ..Default::default()
        };
        Message {
            id: format!("vo_{n}"),
            sort_key: beeper_core::store::sort_key(last.sort_key.parse::<i64>().expect("a sort key") + 1),
            sender_id: them(&chat).expect("the other person"),
            is_sender: Some(false),
            is_deleted: None,
            text: None,
            html: None,
            links: Vec::new(),
            poll: None,
            call: None,
            unresolved_media: None,
            attachments: vec![a],
            view_limit: Some(limit),
            ..last
        }
    };
    let once = media(&app, 1, serde_json::json!({ "type": "count", "count": 1 }));
    assert_eq!(crate::chat_list::preview_body(&once).as_deref(), Some("Sent a photo"));
    assert_eq!(crate::chat_list::message_tag(&once), None, "no media badge");
    app.store.write().upsert_message(once);
    let mut sim = simulator(app.view_main());
    let _ = sim.click("View photo").expect("the pill of received media");
    let open: Vec<UiMessage> = sim.into_messages().collect();
    assert!(matches!(&open[..], [UiMessage::ViewOnce(Msg::Open(m))] if m == "vo_1"));
    assert!(app.plays(app.store.read().chat(&chat.id)), "the Play button of the chat list");
    let _ = app.update(open[0].clone());
    assert!(app.view_once.shows("vo_1"), "the viewer opens while the media loads");
    let frame = (iced::widget::image::Handle::from_rgba(1, 1, vec![0; 4]), 0);
    let shown = |mid: &str| UiMessage::ViewOnce(Msg::Loaded(mid.into(), Ok(Shown::Image(vec![frame.clone()]))));
    let _ = app.update(shown("vo_1"));
    let _ = simulator(app.view_main());
    let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(!app.view_once.is_open(), "Escape closes the viewer");
    assert!(!app.plays(app.store.read().chat(&chat.id)), "the only view was used");
    let mut sim = simulator(app.view_main());
    assert!(sim.find("View photo").is_err() && sim.find("Photo").is_ok(), "the pill of used media is outlined");
    drop(sim);
    // Desktop's `playAll`: the received media at the end of the chat, one after the other.
    for n in [2, 3] {
        let m = media(&app, n, serde_json::json!({ "type": "count", "count": 1 }));
        app.store.write().upsert_message(m);
    }
    let _ = app.update(UiMessage::ViewOnce(Msg::PlayAll(chat.id.clone())));
    assert!(app.view_once.shows("vo_2"), "the queue starts after the used media");
    let _ = app.update(shown("vo_2"));
    let _ = app.update(UiMessage::ViewOnce(Msg::Dismiss));
    assert!(app.view_once.shows("vo_3"), "a viewed image goes on to the next media");
    let _ = app.update(shown("vo_3"));
    let _ = app.update(UiMessage::ViewOnce(Msg::Dismiss));
    assert!(!app.view_once.is_open() && !app.plays(app.store.read().chat(&chat.id)));
    // Your own media with a time limit, and media that Desktop cannot open.
    let timed = media(&app, 4, serde_json::json!({ "type": "count", "count": 2, "time": 5000 }));
    app.store.write().upsert_message(Message { is_sender: Some(true), ..timed });
    let mut sim = simulator(app.view_main());
    assert!(sim.find("5s").is_ok(), "the time limit of your own media");
    drop(sim);
    let mut heic = media(&app, 5, serde_json::json!({ "type": "once" }));
    heic.attachments[0].mime_type = Some("image/heic".into());
    app.store.write().upsert_message(heic);
    let _ = app.update(UiMessage::ViewOnce(Msg::Open("vo_5".into())));
    assert!(!app.view_once.is_open() && app.status_banner == "This media can't be opened on desktop yet.");
    // The bridge's redaction after the last view.
    let mut st = app.store.write();
    let i = st.find_message("vo_5").expect("the HEIC media");
    st.update_message(i, |m| m.attachments[0].attachment_type = "video".into());
    st.expire_message(&chat.id, "vo_5").expect("expire the media");
    let gone = st.chat(&chat.id).preview.clone().expect("the expired media previews the chat");
    drop(st);
    assert_eq!(crate::chat_list::preview_body(&gone).as_deref(), Some("Expired video"));
    let mut sim = simulator(app.view_main());
    assert!(sim.find("Video").is_ok(), "the outlined pill");
    assert!(sim.find("This message has been deleted").is_err());
}

/// Desktop's reactions on a network that takes only some (`allowed_reactions`): React shows them in a row instead of
/// the emoji picker, the hover bar's quick reactions and the double-click reaction are only allowed ones.
#[test]
fn allowed_reactions_like_desktop() {
    let mut app = demo_app("allowed-reactions");
    let chat = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let mid = {
        let mut st = app.store.write();
        let i = st.resolve_chat_idx(&chat).expect("the chat");
        st.update_chat(i, |c| c.allowed_reactions = vec!["👍".into(), "❤️".into()]);
        let last = *st.timeline_indices(&chat).last().expect("a message");
        st.messages[last].id.clone()
    };
    let allowed = app.store.read().chat(&chat).allowed_reactions.clone();
    assert_eq!(crate::emoji::allowed_reaction(&allowed, "❤"), Some("❤️".into()), "without its variation selector");
    assert_eq!(crate::emoji::allowed_reaction(&allowed, "😂"), None);
    assert!(crate::emoji::quick_reactions(&app.store.read(), &allowed, true).iter().all(|e| allowed.contains(e)));
    let _ = app.update(UiMessage::Message(message::Msg::React(mid.clone())));
    {
        let st = app.store.read();
        let mut sim = simulator(app.view_emoji_picker(&st).expect("the reactions"));
        assert!(sim.find("👍").is_ok() && sim.find("😂").is_err(), "only the allowed reactions");
    }
    let _ = app.update(UiMessage::Emoji(crate::emoji::EmojiMsg::React("👍".into())));
    assert!(app.pane.emoji.is_none(), "a pick closes them");
    let reacted = app.store.read().message(&mid).reactions.iter().any(|r| r.reaction_key == "👍");
    assert!(reacted, "and reacts");
    app.patch_settings(|s| s.quick_reaction = "😂".into());
    assert!(matches!(app.quick_react(mid), UiMessage::Noop), "a double click does not send what the network refuses");
}

/// Desktop's `thread-header-disappearing-timer`: a chat whose messages disappear shows a timer after its name in the
/// header, which makes the name pill wider.
#[test]
fn the_header_shows_the_disappearing_timer_like_desktop() {
    let mut app = demo_app("header-timer");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let header = |app: &BeeperGuiApp| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view_conversation());
        let dir = std::env::temp_dir().join(format!("beeper-rs-header-timer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = sim.snapshot(&iced::Theme::Light).unwrap().matches_image(dir.join("header"));
        let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let shot = ::image::open(&png).unwrap().into_rgba8();
        std::fs::remove_dir_all(&dir).unwrap();
        // The row of the name pill, under the avatar.
        let k = shot.width() / 1200;
        (0..shot.width()).filter(|&x| shot.get_pixel(x, 62 * k).0 != shot.get_pixel(0, 62 * k).0).count()
    };
    let plain = header(&app);
    {
        let mut st = app.store.write();
        let i = st.chat_idx(NOTE_CHAT_ID);
        st.update_chat(i, |c| c.disappearing = Some(86_400_000));
    }
    assert!(header(&app) > plain, "the timer widens the name pill");
}

/// Desktop's `.left` transition: the space bar and the chat list slide out and back in over 0.2 s.
#[test]
fn sidebar_slides_like_desktop() {
    let mut app = demo_app("sidebar-slide");
    assert!((crate::style::cubic_bezier(0.5, [0.165, 0.84, 0.44, 1.0]) - 0.92).abs() < 0.02, "an ease-out curve");
    let _ = app.update(UiMessage::RunHotkey("Toggle Sidebar"));
    let started = app.sidebar_slide.expect("a slide").elapsed().as_secs_f32();
    assert!(app.sidebar_hidden && (started >= 0.2 || app.sidebar_progress().is_some()), "it starts to slide out");
    drop(simulator(app.view_main()));
    std::thread::sleep(std::time::Duration::from_millis(220));
    assert_eq!(app.sidebar_progress(), None, "and is gone after 0.2 s");
    // Desktop's "Show Sidebar" button at the left of the thread header while the sidebar is hidden.
    let show = |app: &BeeperGuiApp| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view_main());
        sim.point_at(Point::new(30.0, 22.0));
        let press = iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let _ = sim.simulate([press, release]);
        sim.into_messages().any(|m| matches!(m, UiMessage::RunHotkey("Toggle Sidebar")))
    };
    assert!(show(&app), "the header shows the sidebar again");
    let _ = app.update(UiMessage::RunHotkey("Toggle Sidebar"));
    // The slide starts at the toggle; a slow debug build can be past its 0.2 s by the time the update returns.
    assert!(!app.sidebar_hidden && app.sidebar_slide.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0));
    drop(simulator(app.view_main()));
    std::thread::sleep(std::time::Duration::from_millis(220));
    assert!(!show(&app), "not while the sidebar shows");
}

/// Desktop's `Wfe` contact card of a `.vcf` attachment in the timeline, with the Call and Copy menu of a number.
#[test]
fn contact_cards_in_the_timeline_like_desktop() {
    let mut app = demo_app("vcard-timeline");
    let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
    let url = "mxc://x/ada".to_string();
    {
        let mut st = app.store.write();
        let i = *st.timeline_indices(NOTE_CHAT_ID).last().expect("a message");
        let a = Attachment {
            attachment_type: "unknown".into(),
            file_name: Some("ada.vcf".into()),
            src_url: Some(url.clone()),
            ..Attachment::default()
        };
        (st.messages[i].text, st.messages[i].attachments) = (None, vec![a]);
    }
    let card = crate::vcard::parse_vcard("BEGIN:VCARD\nFN:Ada Lovelace\nORG:Engines\nTEL:+14155550199\nEND:VCARD\n");
    app.vcards.insert(url, Some(Ok(card)));
    {
        let mut sim = simulator(app.view_conversation());
        assert!(sim.find("Ada Lovelace").is_ok() && sim.find("Phone Number").is_ok() && sim.find("Engines").is_ok());
    }
    let items = info::info_menu(&app.store.read(), "vcard:phone +14155550199", |_| true);
    let labels: Vec<&str> = items.iter().flatten().map(|i| i.0.as_str()).collect();
    assert_eq!(labels, ["Call +1 415-555-0199", "Copy +1 415-555-0199"]);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu("vcard:phone +14155550199".into())));
    assert!(simulator(app.view_main()).find("Call +1 415-555-0199").is_ok(), "a click on the number opens its menu");
}

/// The chat that the custom emoji tests open, with its last message, and Desktop's custom emoji of the own image pack
/// (`m.image_pack`) for it.
fn custom_emoji_chat(app: &mut BeeperGuiApp, takes: bool) -> (String, String) {
    let chat = app.visible_chats(&app.store.read())[0].id.clone();
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    let mut st = app.store.write();
    st.image_pack = serde_json::json!({ "images": {
        "party_parrot": { "url": "mxc://x/parrot" }, "cat": { "url": "mxc://x/cat", "usage": ["sticker"] } } });
    let i = st.resolve_chat_idx(&chat).expect("the chat");
    st.update_chat(i, |c| c.custom_emoji = takes);
    let last = *st.timeline_indices(&chat).last().expect("a message");
    (chat, st.messages[last].id.clone())
}

/// The pixels of a frame of `sim`, at its scale of 2.
fn custom_emoji_pixels(sim: &mut Simulator<'_, UiMessage>) -> ::image::RgbaImage {
    let thread = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("beeper-rs-custom-emoji-{}-{thread:?}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _ = sim.snapshot(&iced::Theme::Light).unwrap().matches_image(dir.join("shot"));
    let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let shot = ::image::open(&png).unwrap().into_rgba8();
    std::fs::remove_dir_all(&dir).unwrap();
    shot
}

/// Desktop's emoji picker with `customEmojisForReactions`: the own pack's images in emoji-mart's "Custom" category
/// and its search, its preview with the URL as the shortcode, a pick reacting with the image, which then is a quick
/// reaction where the room takes custom emoji. A room without `custom_emoji_reactions` has none.
#[test]
fn custom_emoji_picker_like_desktop() {
    use crate::emoji::{EmojiMsg, Item};
    let mut app = demo_app("custom-emoji-picker");
    let (_, mid) = custom_emoji_chat(&mut app, true);
    let red = image::Handle::from_rgba(8, 8, [255, 0, 0, 255].repeat(64));
    app.media.insert("mxc://x/parrot".into(), Some(Ok(vec![(red, 0)])));
    let _ = app.update(UiMessage::Message(message::Msg::React(mid.clone())));
    let custom = |app: &BeeperGuiApp| {
        let st = app.store.read();
        app.picker_custom(app.pane.emoji.as_ref().expect("the picker"), &st)
    };
    let names: Vec<String> = custom(&app).into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["party_parrot", "cat"], "every image of the pack, stickers too");
    let _ = app.update(UiMessage::Emoji(EmojiMsg::Query("party_parrot".into())));
    {
        let st = app.store.read();
        let picker = app.view_emoji_picker(&st).unwrap();
        let mut sim = Simulator::with_size(Default::default(), Size::new(352.0, 435.0), picker);
        // The first cell of "Search Results": 12 px in, under the nav (42), the search (47) and the header (30).
        let px = custom_emoji_pixels(&mut sim).get_pixel(2 * 30, 2 * (42 + 47 + 30 + 18)).0;
        assert!(px[0] > 200 && px[1] < 60 && px[2] < 60, "the custom emoji's image comes first: {px:?}");
    }
    let _ = app.update(UiMessage::Emoji(EmojiMsg::Hover(Item::Custom(0))));
    {
        let st = app.store.read();
        let mut sim = simulator(app.view_emoji_picker(&st).unwrap());
        assert!(sim.find("party_parrot").is_ok() && sim.find(":mxc://x/parrot:").is_ok(), "emoji-mart's preview");
    }
    let picked = custom(&app)[0].clone();
    let _ = app.update(UiMessage::Emoji(EmojiMsg::PickCustom(picked)));
    let st = app.store.read();
    let reaction = st.message(&mid).reactions.iter().find(|r| st.is_me(&r.participant_id)).cloned();
    assert_eq!(reaction.map(|r| r.reaction_key), Some("mxc://x/parrot".into()), "the key is the image");
    assert!(crate::emoji::quick_reactions(&st, &[], true).contains(&"mxc://x/parrot".to_string()));
    assert!(!crate::emoji::quick_reactions(&st, &[], false).iter().any(|e| crate::emoji::is_mxc(e)));
    drop(st);
    let (_, mid) = custom_emoji_chat(&mut app, false);
    let _ = app.update(UiMessage::Message(message::Msg::React(mid)));
    assert!(custom(&app).is_empty(), "a room without custom_emoji_reactions");
    // The quick reactions of Settings take the pack's emoji in any room.
    let _ = app.update(UiMessage::QuickEmoji(Some(1)));
    assert_eq!(custom(&app).len(), 2);
}

/// Desktop's composer autocomplete with custom emoji: ":par" offers the image with its name, a pick puts Desktop's
/// `customEmoji` node in (":name:" here), and the send has `#[name](url)` as the body and the image in the HTML.
/// The draft keeps it.
#[test]
fn custom_emoji_composer_like_desktop() {
    let mut app = demo_app("custom-emoji-composer");
    let (chat, _) = custom_emoji_chat(&mut app, true);
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed("hi :par".into())));
    let rows = |app: &BeeperGuiApp| {
        let st = app.store.read();
        let rows = app.suggestions(&st).map_or_else(Vec::new, |(_, rows)| rows);
        let emoji = |s| match s {
            Suggestion::Emoji(c, name) => Some((c, name)),
            Suggestion::Mention(..) => None,
        };
        rows.into_iter().filter_map(emoji).collect::<Vec<_>>()
    };
    // match-sorter ranks "parachute" first, whose first term starts with "par", before "party" of the name.
    let at = rows(&app).iter().position(|r| *r == ("mxc://x/parrot".to_string(), "party_parrot".to_string()));
    assert!(at.is_some_and(|at| at > 0) && rows(&app)[0].1 == "Parachute", "{:?}", rows(&app));
    let _ = app.update(UiMessage::Composer(composer::Msg::PickSuggestion(at.unwrap())));
    assert_eq!(app.pane.composer.text(), "hi :party_parrot: ");
    // The draft has Desktop's text of the node, and gives the node back.
    let other = app.store.read().chats.iter().find(|c| c.id != chat).unwrap().id.clone();
    let _ = app.update(UiMessage::SelectChat(other));
    let draft = app.store.read().chat(&chat).draft.clone().and_then(|d| d.text);
    assert_eq!(draft.as_deref(), Some("hi #[party_parrot](mxc://x/parrot) "));
    let _ = app.update(UiMessage::SelectChat(chat.clone()));
    assert_eq!(app.pane.composer.text(), "hi :party_parrot: ");
    let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
    let st = app.store.read();
    let sent = st.outbox.iter().find_map(|(_, _, o)| match o {
        Outgoing::Send { text, .. } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(sent.as_deref(), Some("hi #[party_parrot](mxc://x/parrot)"));
    let echo = st.timeline_indices(&chat).last().and_then(|&i| st.messages[i].html.clone());
    let img =
        r#"<img data-mx-emoticon src="mxc://x/parrot" alt=":party_parrot:" title=":party_parrot:" height="32" />"#;
    assert_eq!(echo, Some(format!("hi {img}")));
    drop(st);
    // A room without custom emoji offers none.
    custom_emoji_chat(&mut app, false);
    let _ = app.update(UiMessage::Composer(composer::Msg::Changed(":par".into())));
    assert!(rows(&app).iter().all(|(c, _)| !crate::emoji::is_mxc(c)));
}

/// Desktop's "Add :%s: to Custom Emoji" (M24): a right click on an inline custom emoji (named by its `title`) or on a
/// reaction of an image with a shortcode adds it to the own image pack, with Desktop's toasts, and is off when the pack
/// has the name. A right click on the image hides Copy with Timestamp like one on any image.
#[test]
fn add_custom_emoji_like_desktop() {
    use message::Msg::{AddCustomEmoji, EmoticonMenu, ReactionMenu};
    let mut app = demo_app("add-custom-emoji");
    let (_, mid) = custom_emoji_chat(&mut app, true);
    fn menu<'a>(app: &'a BeeperGuiApp, mid: &str) -> Simulator<'a, UiMessage> {
        simulator(app.view_message_menu(mid).0)
    }
    let item = "Add :owl: to Custom Emoji";
    let _ = app.update(UiMessage::Message(EmoticonMenu(mid.clone(), Some(":owl:".into()), "mxc://x/owl".into())));
    assert!(menu(&app, &mid).find(item).is_ok() && menu(&app, &mid).find("Copy with Timestamp").is_err());
    let _ = app.update(UiMessage::Message(AddCustomEmoji("owl".into(), "mxc://x/owl".into())));
    assert_eq!(app.status_banner, "Added emoji :owl:");
    assert_eq!(app.store.read().image_pack["images"]["owl"], serde_json::json!({ "url": "mxc://x/owl" }));
    let mut sim = menu(&app, &mid);
    let _ = sim.click(item);
    assert!(sim.into_messages().next().is_none(), "off once the pack has the name");
    let _ = app.update(UiMessage::Message(AddCustomEmoji("owl".into(), "mxc://x/owl".into())));
    assert_eq!(app.status_banner, "Couldn't add the emoji: You already have a custom emoji named owl");
    let _ = app.update(UiMessage::Message(ReactionMenu(mid.clone(), "blob".into(), "mxc://x/blob".into())));
    assert!(
        menu(&app, &mid).find("Add :blob: to Custom Emoji").is_ok()
            && menu(&app, &mid).find("Copy with Timestamp").is_ok()
    );
    // A reaction whose key is the image itself, and an image without a title, have no name.
    let _ = app.update(UiMessage::Message(ReactionMenu(mid.clone(), "mxc://x/b".into(), "mxc://x/b".into())));
    assert!(menu(&app, &mid).find("Add :mxc://x/b: to Custom Emoji").is_err());
    let _ = app.update(UiMessage::Message(EmoticonMenu(mid.clone(), None, "mxc://x/c".into())));
    assert!(menu(&app, &mid).find("Copy with Timestamp").is_err() && app.pane.menu_emoticon.is_none());
}

/// Desktop's `ZGe`: the chat row of a message that failed to send shows the red `ErroredIcon` in place of its sent
/// icon.
#[test]
fn failed_sends_show_their_icon_in_the_chat_list_like_desktop() {
    let app = demo_app("chat-row-errored");
    let red = |app: &BeeperGuiApp| {
        let mut sim = Simulator::with_size(Default::default(), Size::new(320.0, 800.0), app.view_sidebar());
        let shot = custom_emoji_pixels(&mut sim);
        shot.pixels().filter(|px| px[0] > 220 && (40..110).contains(&px[1]) && (30..100).contains(&px[2])).count()
    };
    let unpinned = |c: &&&Chat| !c.flag(ChatFlag::Pinned) && c.preview.as_ref().is_some_and(|m| m.reactions.is_empty());
    let idx = app.visible_chats(&app.store.read()).iter().find(unpinned).map(|c| c.id.clone()).expect("a chat");
    let idx = app.store.read().chat_idx(&idx);
    app.store.write().update_chat(idx, |c| {
        let m = c.preview.as_mut().expect("a preview");
        (m.is_sender, m.send_error) = (Some(true), None);
    });
    let before = red(&app);
    app.store.write().update_chat(idx, |c| {
        let m = c.preview.as_mut().expect("a preview");
        m.send_error = Some(beeper_core::models::SendError { message: Some("not connected".into()), retrying: false });
    });
    assert!(red(&app) > before + 20, "the row shows the red icon");
}

/// Like Electron's native menus, the chat menu fits its longest row: a long hotkey, or a longer translation such as
/// German's "Als ungelesen markieren  Ctrl Shift U", widens it past Desktop's 233 px instead of wrapping the row.
#[test]
fn the_chat_menu_fits_its_rows() {
    let app = demo_app("chat-menu-width");
    assert_eq!(chat_list::chat_menu_w(&app.store.read()), 233.0);
    let keys = ("Chat: Toggle Mark As Read".to_string(), "ctrl+shift+alt+backspace".to_string());
    app.store.write().settings.keybindings.extend([keys]);
    let w = |s: &str| style::text_width(s, 13.0, iced::font::Weight::Normal);
    let row = 26.0 + w("Mark as Unread") + 16.0 + w("Ctrl Shift Alt Backspace");
    let menu = chat_list::chat_menu_w(&app.store.read());
    assert!(row > 233.0 && menu >= row, "{menu} fits {row}");
}
