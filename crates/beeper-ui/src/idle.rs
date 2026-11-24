//! Desktop's idle state (`layoutStore.idleState`): after 5 minutes without input the app is idle, and the open chat
//! does not read the messages that come in (`markAsRead` returns while idle). Input brings it back, which reads them.
//!
//! Desktop asks Electron's `powerMonitor.getSystemIdleState(300)` every minute (`Sy`), and asks again on the first
//! mouse, key, wheel or touch event in a window while idle (`EWe`). beeper-rs counts the input that its windows get:
//! a chat only reads while a window of the app is focused, and the focused window gets the keys.

use super::*;
use crate::prelude::*;
use std::sync::atomic::AtomicU64;

/// Desktop's default `idle_timeout` of 300 seconds.
pub(crate) const IDLE_AFTER: Duration = Duration::from_secs(300);
/// How often Desktop's idle monitor asks for the idle state.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// When the last input came, in milliseconds since [`start`].
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);

fn start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

fn note_input() {
    LAST_INPUT.store(start().elapsed().as_millis() as u64, Ordering::Relaxed);
}

/// How long since the last input.
fn since_input() -> Duration {
    start().elapsed().saturating_sub(Duration::from_millis(LAST_INPUT.load(Ordering::Relaxed)))
}

/// The input that Desktop's `EWe` waits for: a press, a move or the wheel of the mouse, a key or a touch.
fn is_input(e: &Event) -> bool {
    matches!(
        e,
        Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::CursorMoved { .. } | mouse::Event::WheelScrolled { .. })
            | Event::Keyboard(keyboard::Event::KeyPressed { .. })
            | Event::Touch(_)
    )
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The minute of Desktop's idle monitor.
    Check,
    /// Input came while idle.
    Returned,
}

impl BeeperGuiApp {
    pub(crate) fn idle(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Check => {
                self.idle = since_input() >= IDLE_AFTER;
                crate::style::trim_shaped_words();
            }
            Msg::Returned => {
                note_input();
                self.idle = false;
                self.read_open_chat();
                self.read_open_thread();
            }
        }
        self.settle()
    }

    /// Desktop's `markReplyThreadRead` of the open thread pane at its bottom.
    fn read_open_thread(&mut self) {
        let shows = |t: &&Pane| self.focused && t.scrolled_up.is_none_or(|(up, _)| !up);
        let Some(t) = self.pane.thread.as_deref().filter(shows) else { return };
        let (Some(chat), Some(root)) = (t.selected_chat_id.clone(), t.thread_root.clone()) else { return };
        let mut st = self.store.write();
        if let Some(i) = st.resolve_chat_idx(&chat).filter(|&i| st.thread_unread(&st.chats[i], &root).0 > 0) {
            let cid = st.chats[i].id.clone();
            st.mark_thread_read(&cid, &root, None);
        }
    }

    /// The clock of the input, which sends no message, the minute of the check, and while idle, the first input.
    pub(crate) fn idle_events(&self) -> impl Iterator<Item = Subscription<UiMessage>> {
        let clock = iced::event::listen_with(|e, _, _| {
            if is_input(&e) {
                note_input();
            }
            None
        });
        let check = iced::time::every(CHECK_EVERY).map(|_| UiMessage::Idle(Msg::Check));
        let back = self.idle.then(|| iced::event::listen_with(|e, _, _| is_input(&e).then_some(UiMessage::Idle(Msg::Returned))));
        [clock, check].into_iter().chain(back)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `markAsRead` returns while the app is idle: a message that comes into the open chat stays unread,
    /// and input reads it. A "Mark as Unread" from a message (`preventAutoMarkAsRead`) keeps the open chat unread
    /// until you leave it.
    #[test]
    fn the_open_chat_reads_only_while_you_are_there_like_desktop() {
        let mut app = crate::fuzz::demo_app("idle-read");
        let (chat, msg) = {
            let st = app.store.read();
            let has_msgs = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
            let c = st.chats.iter().filter(|c| c.flags & CHAT_PINNED == 0).find(has_msgs).expect("a chat").clone();
            let last = st.messages[*st.timeline_indices(&c.id).last().expect("a message")].clone();
            (c.id.clone(), last)
        };
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let _ = app.update(UiMessage::Window(windows::Msg::Focused(true)));
        let unread = |app: &BeeperGuiApp| app.store.read().chat(&chat).is_unread();
        assert!(!unread(&app), "the open chat is read");
        let arrive = |app: &mut BeeperGuiApp, i: u32| {
            let m = Message {
                id: format!("$idle{i}"),
                text: Some(format!("While away {i}")),
                sort_key: format!("{:016}", msg.sort_key.parse::<u64>().expect("a numeric sort key") + u64::from(i)),
                is_sender: Some(false),
                is_unread: Some(true),
                ..msg.clone()
            };
            let mut st = app.store.write();
            st.upsert_message(m.clone());
            let idx = st.resolve_chat_idx(&chat).expect("the chat");
            st.update_chat(idx, |c| c.unread_count += 1);
            drop(st);
            let _ = app.update(UiMessage::Upserted(Box::new(m)));
        };
        app.idle = true;
        arrive(&mut app, 1);
        assert!(unread(&app), "a message that comes in while idle stays unread");
        let _ = app.update(UiMessage::Idle(Msg::Returned));
        assert!(!app.idle && !unread(&app), "input reads it");
        let _ = app.update(UiMessage::Message(message::Msg::MarkUnreadFromHere(chat.clone(), "$idle1".into())));
        assert!(unread(&app));
        arrive(&mut app, 2);
        let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(false)));
        assert!(unread(&app), "a chat marked unread from a message stays unread while it is open");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let _ = app.update(UiMessage::Timeline(timeline::Msg::Scrolled(false)));
        assert!(!unread(&app), "it reads again once you come back to it");
    }

    #[test]
    fn idle_counts_from_the_last_input() {
        note_input();
        assert!(since_input() < IDLE_AFTER);
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: Point::ORIGIN });
        assert!(is_input(&moved) && !is_input(&Event::Window(window::Event::Focused)));
    }
}
