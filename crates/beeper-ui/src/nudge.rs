//! Desktop's nudges: `sendNudge` from the composer, `maybeTriggerNudge` for the nudges that come in, and the
//! `beeper-nudge-shake` of the timeline of the open chat.

use super::*;
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum Msg {
    /// Desktop's `sendNudge` of the open chat, from "Send a nudge" or `/nudge`.
    Send,
}

/// Desktop's `nudgedMessageIds`, `lastLocalNudgeSentAt` of each chat, and the shake of the chat whose `nudgeTick`
/// moved.
#[derive(Default)]
pub(crate) struct Nudges {
    seen: HashSet<String>,
    sent: HashMap<String, Instant>,
    shake: Option<(String, Instant)>,
}

/// Desktop's `beeper-nudge-shake`: 0.8 s.
pub(crate) const SHAKE: Duration = Duration::from_millis(800);

/// The keyframes of `beeper-nudge-shake` as (progress, x, y). iced has no rotation, so the turns of up to 0.5° are
/// left out.
const KEYFRAMES: [(f32, f32, f32); 18] = [
    (0.0, 0.0, 0.0),
    (0.05, 7.0, 2.0),
    (0.10, -8.0, -2.0),
    (0.15, 7.0, -2.0),
    (0.20, -7.0, 2.0),
    (0.25, 6.0, 2.0),
    (0.30, -6.0, -1.0),
    (0.35, 5.0, 2.0),
    (0.40, -5.0, -2.0),
    (0.45, 4.0, 1.0),
    (0.50, -4.0, -1.0),
    (0.55, 3.0, 1.0),
    (0.60, -3.0, -1.0),
    (0.65, 2.0, 1.0),
    (0.70, -2.0, 0.0),
    (0.78, 1.0, 0.0),
    (0.88, -1.0, 0.0),
    (1.0, 0.0, 0.0),
];

/// The `animation-timing-function` of the shake, which CSS applies between each two keyframes.
const EASE: [f32; 4] = [0.36, 0.07, 0.19, 0.97];

/// The move of the shaking timeline `t` seconds into the shake.
pub(crate) fn shake_offset(t: f32) -> (f32, f32) {
    let at = (t / SHAKE.as_secs_f32()).clamp(0.0, 1.0);
    let i = KEYFRAMES.iter().rposition(|k| k.0 <= at).expect("the first keyframe is at 0").min(KEYFRAMES.len() - 2);
    let ((a, ax, ay), (b, bx, by)) = (KEYFRAMES[i], KEYFRAMES[i + 1]);
    let e = crate::cubic_bezier((at - a) / (b - a), EASE);
    (ax + (bx - ax) * e, ay + (by - ay) * e)
}

/// Desktop's nudges come only from the chats of the Beeper account.
const BEEPER: &str = "hungryserv";

/// Desktop's `maybeTriggerNudge` skips a nudge sent more than 15 s ago, and the echo of its own nudge 5 s after it.
const FRESH_MS: i64 = 15_000;
const ECHO: Duration = Duration::from_secs(5);

impl Nudges {
    /// The move of the timeline of `chat` now.
    pub(crate) fn offset(&self, chat: Option<&str>) -> (f32, f32) {
        match &self.shake {
            Some((c, at)) if Some(c.as_str()) == chat && at.elapsed() < SHAKE => shake_offset(at.elapsed().as_secs_f32()),
            _ => (0.0, 0.0),
        }
    }

    pub(crate) fn shaking(&self) -> bool {
        self.shake.as_ref().is_some_and(|s| s.1.elapsed() < SHAKE)
    }

    /// Desktop's `beeper-nudge-active`: a shake that runs goes on, so a second nudge does not start it again.
    fn start(&mut self, chat: String) {
        if !self.shaking() {
            self.shake = Some((chat, Instant::now()));
        }
    }
}

impl BeeperGuiApp {
    pub(crate) fn nudge(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Send => {
                let st = self.store.read();
                let Some(chat) = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c)) else {
                    return Task::none();
                };
                let target = self.nudge_target(&st, &st.chats[chat]);
                drop(st);
                let Some(target) = target else { return Task::none() };
                // Desktop's `nudgeTick++` shakes the own chat at once, and its echo does not shake it again.
                self.nudges.sent.insert(target.clone(), Instant::now());
                self.nudges.start(self.open_chat_key());
                self.store.write().send_nudge(&target);
            }
        }
        self.settle()
    }

    /// The canonical ID of the open chat, which a shake names.
    fn open_chat_key(&self) -> String {
        let st = self.store.read();
        let open = self.pane.selected_chat_id.as_deref();
        open.and_then(|c| st.resolve_canonical_chat_id(c)).or(open.map(String::from)).unwrap_or_default()
    }

    /// Desktop's `canNudge` of the open chat: the chat that the nudge goes to, its `composeTarget`, when the account
    /// has the `nudge` feature with "Enable nudges", and the chat is a Beeper chat that takes messages.
    pub(crate) fn nudge_target(&self, st: &BeeperStore, chat: &Chat) -> Option<String> {
        let merged = !st.members(st.chat_idx(&chat.id)).is_empty();
        let target = if merged { &st.chats[self.compose_target_of(st, st.chat_idx(&chat.id))] } else { chat };
        let readonly = unavailable(st, target).is_some() || st.user_blocked(target) || target.exited();
        let on = st.feature("nudge").on && st.settings.on("enableNudges");
        (on && target.account_id == BEEPER && !readonly).then(|| target.id.clone())
    }

    /// Desktop's `maybeTriggerNudge` for a message that came in: a fresh nudge shakes its chat when it is open, and the
    /// first nudge from someone else asks whether to keep nudges.
    pub(crate) fn maybe_nudge(&mut self, m: &Message) {
        let st = self.store.read();
        let on = st.feature("nudge").on && st.settings.on("enableNudges") && m.account_id == BEEPER;
        if !on || !m.nudge || self.nudges.seen.contains(&m.id) {
            return;
        }
        let at = millis_from_rfc3339(&m.timestamp).unwrap_or(0);
        let own = m.is_sender == Some(true);
        let echo = own && self.nudges.sent.get(&m.chat_id).is_some_and(|t| t.elapsed() < ECHO);
        if at == 0 || now_unix_millis() - at > FRESH_MS || echo {
            return;
        }
        let chat = st.resolve_canonical_chat_id(&m.chat_id).unwrap_or_else(|| m.chat_id.clone());
        let seen = st.settings.has_seen_first_nudge;
        drop(st);
        self.nudges.seen.insert(m.id.clone());
        if self.panes().any(|p| p.selected_chat_id.as_deref() == Some(chat.as_str())) {
            self.nudges.start(chat);
        }
        if !own && !seen {
            self.patch_settings(|s| s.has_seen_first_nudge = true);
            let text = tr("You got nudged! Want to disable nudges?").to_string();
            let disable = UiMessage::Settings(settings::Msg::SetPref("enableNudges", 0));
            let buttons = vec![("Disable", disable), ("Keep them", UiMessage::Noop)];
            self.show(Toast { buttons, stays: true, ..Toast::new(text) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composer::{Suggestion, SLASH_COMMANDS};
    use beeper_core::matrix::Outgoing;
    use iced::keyboard::{key::Named, Key, Modifiers};

    fn flag(app: &BeeperGuiApp, key: &str, on: bool) {
        app.store.write().features.flags["features"]["*"][key] = on.into();
    }

    /// The slash commands that the composer lists for `text`.
    fn commands(app: &mut BeeperGuiApp, text: &str) -> Vec<&'static str> {
        let _ = app.update(UiMessage::Composer(composer::Msg::Changed(text.into())));
        let st = app.store.read();
        let rows = app.suggestions(&st).map_or_else(Vec::new, |(_, rows)| rows);
        let id = |s: Suggestion| if let Suggestion::Command(i) = s { Some(SLASH_COMMANDS[i].0) } else { None };
        rows.into_iter().filter_map(id).collect()
    }

    fn nudge_from(app: &BeeperGuiApp, id: &str, own: bool, ago_ms: i64) -> Message {
        Message {
            id: id.into(),
            account_id: BEEPER.into(),
            chat_id: NOTE_CHAT_ID.into(),
            sender_id: if own { app.store.read().me().id.clone() } else { "@ann:beeper.com".into() },
            timestamp: beeper_core::config::rfc3339_from_millis(now_unix_millis() - ago_ms),
            is_sender: Some(own),
            text: Some("Ann sent you a nudge".into()),
            nudge: true,
            ..Default::default()
        }
    }

    /// Desktop's `createSlashCommandTriggerSettings` and `sendNudge`: `/` lists the commands of the chat while the
    /// composer is only the command, with `slash-commands-available`, and `/nudge` with `nudge` and "Enable nudges".
    #[test]
    fn slash_commands_run_like_desktop() {
        let mut app = crate::fuzz::demo_app("slash-commands");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        assert!(commands(&mut app, "/").is_empty(), "the flag is off");
        flag(&app, "slash-commands-available", true);
        assert_eq!(commands(&mut app, "/"), ["gif", "voice"], "nudges are off");
        flag(&app, "nudge", true);
        assert_eq!(commands(&mut app, "/"), ["gif", "voice", "nudge"]);
        assert_eq!(commands(&mut app, "/NU"), ["nudge"], "the query matches the start of the ID in any case");
        assert!(commands(&mut app, "/x").is_empty());
        assert!(commands(&mut app, "hi /g").is_empty(), "the command is the whole composer");
        assert!(commands(&mut app, "hi\n/g").is_empty(), "in one paragraph");
        let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("enableNudges", 0)));
        assert_eq!(commands(&mut app, "/"), ["gif", "voice"], "\"Enable nudges\" is off");
        let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("enableNudges", 1)));
        assert!(app.pane_actions(&app.store.read()).contains(&8), "\"Send a nudge\" is behind the plus button");

        // ↓ goes around the list, and ↑ stops at the first command.
        assert_eq!(commands(&mut app, "/"), ["gif", "voice", "nudge"]);
        let key = |app: &mut BeeperGuiApp, k| _ = app.update(UiMessage::KeyPressed(Key::Named(k), Modifiers::empty()));
        (0..3).for_each(|_| key(&mut app, Named::ArrowDown));
        assert_eq!(app.pane.suggest, 0);
        key(&mut app, Named::ArrowUp);
        assert_eq!(app.pane.suggest, 0, "↑ does not go around");

        // Enter runs the command: it leaves the composer, the nudge goes out, and the own chat shakes.
        let _ = commands(&mut app, "/nudge");
        let _ = app.update(UiMessage::Composer(composer::Msg::Submit));
        assert_eq!(app.pane.composer.text().trim_end(), "");
        let st = app.store.read();
        let sent = st.outbox.iter().find_map(|(_, _, o)| if let Outgoing::Nudge(b) = o { Some(b.clone()) } else { None });
        assert!(sent.is_some_and(|b| b.ends_with(" sent you a nudge")), "{:?}", st.outbox.len());
        drop(st);
        let (dx, dy) = app.nudges.offset(Some(NOTE_CHAT_ID));
        assert!(app.nudges.shaking() && dx.abs() <= 8.0 && dy.abs() <= 2.0, "the own chat shakes");
        assert!(app.nudges.offset(Some("!other:beeper.com")) == (0.0, 0.0), "only the nudged chat shakes");
        // The echo of the own nudge does not count again.
        let echo = nudge_from(&app, "$echo", true, 0);
        app.maybe_nudge(&echo);
        assert!(!app.nudges.seen.contains("$echo"));
        assert!(app.status_banner.is_empty(), "no prompt for the own nudge");
    }

    /// Desktop's `maybeTriggerNudge` and `maybeShowFirstNudgePrompt`: a fresh nudge of someone else shakes the open
    /// chat, and the first one asks whether to keep nudges.
    #[test]
    fn nudges_that_come_in_shake_the_chat_like_desktop() {
        let mut app = crate::fuzz::demo_app("nudges-in");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        app.maybe_nudge(&nudge_from(&app, "$a", false, 0));
        assert!(!app.nudges.shaking(), "the nudge flag is off");
        flag(&app, "nudge", true);
        app.maybe_nudge(&nudge_from(&app, "$old", false, 20_000));
        assert!(!app.nudges.shaking(), "a nudge of 20 s ago does not shake");
        app.maybe_nudge(&nudge_from(&app, "$a", false, 1_000));
        assert!(app.nudges.shaking());
        assert_eq!(app.status_banner, "You got nudged! Want to disable nudges?");
        assert!(app.store.read().settings.has_seen_first_nudge);
        let disable = app.toast.as_ref().map(|t| t.buttons.iter().map(|b| b.0).collect::<Vec<_>>());
        assert_eq!(disable.as_deref(), Some(&["Disable", "Keep them"][..]));
        let _ = app.update(UiMessage::Toast(toasts::Msg::DismissBanner));
        app.nudges.shake = None;
        app.maybe_nudge(&nudge_from(&app, "$b", false, 0));
        assert!(app.nudges.shaking() && app.status_banner.is_empty(), "the prompt shows once");
        app.nudges.shake = None;
        let _ = app.update(UiMessage::Settings(settings::Msg::SetPref("enableNudges", 0)));
        app.maybe_nudge(&nudge_from(&app, "$c", false, 0));
        assert!(!app.nudges.shaking(), "\"Enable nudges\" is off");
    }

    #[test]
    fn the_shake_follows_desktops_keyframes() {
        let near = |t: f32, (x, y): (f32, f32)| {
            let (dx, dy) = shake_offset(t);
            assert!((dx - x).abs() < 0.01 && (dy - y).abs() < 0.01, "at {t} s: ({dx}, {dy})");
        };
        near(0.0, (0.0, 0.0));
        near(0.04, (7.0, 2.0));
        near(0.08, (-8.0, -2.0));
        near(0.8, (0.0, 0.0));
        near(2.0, (0.0, 0.0));
        let (x, _) = shake_offset(0.06);
        assert!(x < 0.0 && x > -8.0, "the move eases between two keyframes: {x}");
    }
}
