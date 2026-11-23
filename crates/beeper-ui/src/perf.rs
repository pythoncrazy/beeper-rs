//! The speed of the main window on a large account (`beeper_core::testing::large_store`): the frames of the open
//! chat, Chat Info and the chat list, and the common actions. The views keep what they compute from the store until
//! the store changes, so each frame is timed both with that memo and right after a store write, which empties it.
//!
//! `cargo test --release -p beeper-ui --lib perf::profile -- --ignored --nocapture` prints the times.

use super::*;
use beeper_core::store::StoreLock;
use beeper_core::testing::{large_store, sized_store, widen_large, LARGE_OPEN};
use iced::mouse::ScrollDelta;
use iced_test::simulator::Simulator;
use std::time::{Duration, Instant};

/// The median time of `runs` runs of `f`, after one run that warms the caches.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    f();
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

/// A frame of the main window.
fn frame(app: &BeeperGuiApp) {
    drop(app.view(app.main_window.expect("a main window")));
}

/// A frame of the main window after a write to the store, which empties the memo of the views.
fn frame_after_write(app: &BeeperGuiApp) {
    drop(app.store.write());
    frame(app);
}

/// Lays out the main window from a new widget tree, sends it `events` and a redraw, and runs the messages of the
/// window.
fn simulate(app: &mut BeeperGuiApp, at: Point, events: Vec<iced::Event>) {
    let id = app.main_window.expect("a main window");
    let mut sim = Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view(id));
    sim.point_at(at);
    let redraw = iced::Event::Window(window::Event::RedrawRequested(Instant::now()));
    let _ = sim.simulate(events.into_iter().chain([redraw]));
    let msgs: Vec<UiMessage> = sim.into_messages().collect();
    for m in msgs {
        let _ = app.update(m);
    }
}

fn wheel(y: f32) -> Vec<iced::Event> {
    vec![iced::Event::Mouse(mouse::Event::WheelScrolled { delta: ScrollDelta::Pixels { x: 0.0, y } })]
}

fn large_id(i: usize) -> String {
    format!("!large_{i}:beeper.com")
}

/// The UI messages of the store events in `rx`, like the subscription of the store.
fn store_events(rx: &mut tokio::sync::broadcast::Receiver<beeper_core::models::WsEvent>) -> Vec<UiMessage> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|e| match (e.event_type.as_str(), e.message, e.chat) {
            ("message.upserted", Some(m), _) => UiMessage::Upserted(Box::new(m)),
            ("chat.upserted", _, Some(c)) => UiMessage::ChatUpserted(Box::new(c)),
            _ => UiMessage::StoreChanged,
        })
        .collect()
}

/// The times of the frames and actions on the large account, each with a generous bound for a release build.
fn times(name: &str) -> Vec<(&'static str, Duration, u64)> {
    let mut out = Vec::new();
    let mut time = |what, bound, t| out.push((what, t, bound));
    let mut app = BeeperGuiApp::boot(large_store(name)).0;
    let _ = app.update(UiMessage::SelectChat(LARGE_OPEN.into()));
    simulate(&mut app, Point::new(900.0, 450.0), vec![]);
    // The open chat shows the merge suggestion banner, as one of the first three chats with a suggestion.
    assert!(app.pane.merge_banner, "the large chat has a merge suggestion");
    time("frame, chat open", 10, median(21, || frame(&app)));
    time("frame after a store write, chat open", 25, median(21, || frame_after_write(&app)));
    let _ = app.update(UiMessage::ToggleRightPanel);
    simulate(&mut app, Point::new(900.0, 450.0), vec![]);
    time("frame, Chat Info open", 10, median(21, || frame(&app)));
    time("frame after a store write, Chat Info open", 25, median(21, || frame_after_write(&app)));
    let _ = app.update(UiMessage::ToggleRightPanel);
    for _ in 0..3 {
        simulate(&mut app, Point::new(200.0, 500.0), wheel(-20_000.0));
    }
    time("frame, chat list scrolled", 10, median(21, || frame(&app)));
    time("frame after a store write, chat list scrolled", 25, median(21, || frame_after_write(&app)));
    let mut next = (1..40).map(|i| large_id(i * 97)).cycle();
    time(
        "SelectChat + frame",
        50,
        median(21, || {
            let _ = app.update(UiMessage::SelectChat(next.next().expect("a chat")));
            frame(&app);
        }),
    );
    let _ = app.update(UiMessage::SelectChat(LARGE_OPEN.into()));
    simulate(&mut app, Point::new(900.0, 450.0), vec![]);
    let changed = median(21, || {
        let _ = app.update(UiMessage::StoreChanged);
        frame(&app);
    });
    time("StoreChanged + frame", 10, changed);
    let typed = median(21, || {
        let insert = text_editor::Action::Edit(text_editor::Edit::Insert('a'));
        let _ = app.update(UiMessage::Composer(composer::Msg::Edit(insert)));
        frame(&app);
    });
    time("type in the composer + frame", 10, typed);
    let layout = median(11, || drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view_main())));
    time("layout of the main window from a new widget tree", 100, layout);
    let scrolled = median(11, || {
        simulate(&mut app, Point::new(900.0, 450.0), wheel(300.0));
        frame(&app);
    });
    time("scroll the timeline: new layout, scroll, updates, frame", 100, scrolled);
    let template = app.store.read().stored_message("$large_66").expect("a message of the large chat");
    let (mut seq, mut room) = (0, 0);
    // Like the sync of each room, which writes its messages in one batch.
    let mut sync = |app: &mut BeeperGuiApp, rooms: usize| {
        let mut rx = app.store.read().ws_tx.subscribe();
        let mut st = app.store.write();
        for _ in 0..rooms {
            room += 1;
            let chat_id = large_id(1 + room * 37 % 4999);
            let account_id = st.chat(&chat_id).account_id.clone();
            st.batch(|st| {
                for _ in 0..100 / rooms {
                    seq += 1;
                    let (id, sort_key) = (format!("$sync_{seq}"), beeper_core::store::sort_key(1_000_000 + seq));
                    let (chat_id, account_id) = (chat_id.clone(), account_id.clone());
                    st.upsert_message(Message { id, chat_id, account_id, sort_key, ..template.clone() });
                }
            });
        }
        drop(st);
        for m in store_events(&mut rx) {
            let _ = app.update(m);
        }
        frame(app);
    };
    let synced = median(5, || sync(&mut app, 100));
    time("sync of 100 new messages in 100 chats: upserts, updates, frame", 250, synced);
    let synced = median(5, || sync(&mut app, 10));
    time("sync of 100 new messages in 10 chats: upserts, updates, frame", 250, synced);
    let mut next = (1..30).map(|i| large_id(i * 5 * 31)).cycle();
    let read = median(21, || {
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::MarkRead(next.next().expect("a chat"))));
        frame(&app);
    });
    time("mark read + frame", 50, read);
    let _ = app.update(UiMessage::CommandBar(commands::Msg::TogglePalette));
    let mut n = 0;
    let mut search = |q: &dyn Fn(usize) -> String| {
        median(11, || {
            n += 1;
            let _ = app.update(UiMessage::CommandBar(commands::Msg::Query(q(n))));
            frame(&app);
        })
    };
    let chats = search(&|n| format!("Person{}", 100 + n));
    time("chat search: query + frame", 50, chats);
    let messages = search(&|n| format!("message {}", 1000 + n));
    time("message search: query + frame", 50, messages);
    time("frame, command bar open", 10, median(21, || frame(&app)));
    let config = app.store.read().config.clone();
    drop(app);
    let load = || {
        let mut st = BeeperStore::from_config(config.clone());
        widen_large(&mut st);
        st
    };
    time("store load from SQLite", 1000, median(5, || drop(load())));
    time("store load and app start", 1000, median(5, || drop(BeeperGuiApp::boot(StoreLock::shared(load())))));
    out
}

#[test]
#[ignore = "a profile of the large account for release builds"]
fn profile() {
    for (what, t, _) in times("perf-profile") {
        println!("{what:<65} {:>8.2} ms", t.as_secs_f64() * 1000.0);
    }
}

/// The frames and actions on a large account stay well under their bounds in a release build.
#[test]
#[cfg_attr(debug_assertions, ignore = "the times are for release builds")]
fn a_large_account_stays_fast() {
    let slow: Vec<String> = (times("perf-bounds").into_iter())
        .filter(|(_, t, bound)| *t > Duration::from_millis(*bound))
        .map(|(what, t, bound)| format!("{what}: {:.1} ms, more than {bound} ms", t.as_secs_f64() * 1000.0))
        .collect();
    assert!(slow.is_empty(), "too slow: {slow:#?}");
}

/// The merge suggestion of a chat is its group of all merge suggestions, and the memo of the views follows the store.
#[test]
fn the_suggestion_of_a_chat_is_its_group_of_all_suggestions() {
    let app = BeeperGuiApp::boot(sized_store("perf-suggestions", 400, 2_000)).0;
    let st = app.store.read();
    let all = merge_suggest::merge_suggestions(&st);
    assert!(all.len() > 50 && all.iter().any(|s| s.rooms.len() == 3), "many groups, some of three chats");
    for c in &st.chats {
        let group = all.iter().find(|s| s.rooms.contains(&c.id)).cloned();
        assert_eq!(merge_suggest::suggestion_of(&st, &c.id), group, "{}", c.id);
        assert_eq!(app.suggestion_of(&st, &c.id), group, "{}", c.id);
    }
    drop(st);
    let (a, b) = (large_id(6), large_id(6 + 160));
    let joined = app.suggestion_of(&app.store.read(), &a).expect("person 6 has chats on three accounts");
    assert!(joined.rooms.contains(&b));
    let idx = app.store.read().chat_idx(&b);
    app.store.write().update_chat(idx, |c| {
        let other = c.participants.items.iter_mut().find(|u| u.is_self != Some(true)).expect("a person");
        (other.full_name, other.phone_number, other.email) = (Some("Someone Else".into()), None, None);
    });
    let left = app.suggestion_of(&app.store.read(), &a);
    assert!(left.is_none_or(|s| !s.rooms.contains(&b)), "a write to the store finds the suggestion again");
}

/// A new message in another chat changes nothing that the views keep, so its update takes no write lock, which
/// would empty their memo.
#[test]
fn a_message_in_another_chat_keeps_the_memo_of_the_views() {
    let mut app = BeeperGuiApp::boot(sized_store("perf-upserted", 200, 1_000)).0;
    let _ = app.update(UiMessage::SelectChat(LARGE_OPEN.into()));
    frame(&app);
    let mut rx = app.store.read().ws_tx.subscribe();
    let template = app.store.read().stored_message("$large_66").expect("a message of the large chat");
    let chat_id = large_id(7);
    let account_id = app.store.read().chat(&chat_id).account_id.clone();
    let sort_key = beeper_core::store::sort_key(1_000_000);
    app.store.write().upsert_message(Message { id: "$new".into(), chat_id, account_id, sort_key, ..template });
    let msgs = store_events(&mut rx);
    assert!(msgs.iter().any(|m| matches!(m, UiMessage::Upserted(_))));
    frame(&app);
    let generation = app.store.generation();
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(app.store.generation(), generation, "the updates do not write to the store");
}

/// The rows that the view of the open chat made.
fn rows_of(app: &BeeperGuiApp) -> Rc<message::Rows> {
    frame(app);
    app.pane.rows.borrow().as_ref().expect("the view made the rows").1.clone()
}

/// Runs `change` on the store, and tells whether the view made the rows of the open chat again, which show its
/// messages either way.
fn rows_changed(app: &BeeperGuiApp, kept: &mut Rc<message::Rows>, change: impl FnOnce(&mut BeeperStore)) -> bool {
    change(&mut app.store.write());
    let next = rows_of(app);
    let st = app.store.read();
    let ids: Vec<&str> = next.msgs.iter().map(|&i| st.messages[i].id.as_str()).collect();
    let (hide, members) = (st.settings.on("hideDeleted"), st.settings.on("membershipEvents"));
    let hidden = |m: &Message| hide && m.is_deleted == Some(true) && !m.view_once_expired();
    let shown = app.pane_msgs(&st).into_iter().filter(|m| !hidden(m) && (members || !membership(m)));
    assert_eq!(ids, shown.map(|m| m.id.as_str()).collect::<Vec<_>>(), "the rows show the messages");
    drop(st);
    !Rc::ptr_eq(&std::mem::replace(kept, next), kept)
}

/// A new message `id` in `chat`.
fn new_message(st: &BeeperStore, id: &str, chat: &str) -> Message {
    let template = st.stored_message("$large_66").expect("a message of the large chat");
    let (chat_id, account_id) = (chat.to_string(), st.chat(chat).account_id.clone());
    let sort_key = beeper_core::store::sort_key(1_000_000 + st.messages.len() as i64);
    Message { id: id.into(), chat_id, account_id, sort_key, ..template }
}

/// The rows of the open chat stay over the writes to other chats, and each change that they show makes them again.
#[test]
fn the_rows_of_the_open_chat_follow_its_messages() {
    let mut app = BeeperGuiApp::boot(sized_store("perf-rows", 200, 1_000)).0;
    let _ = app.update(UiMessage::SelectChat(LARGE_OPEN.into()));
    let other = large_id(7);
    let upsert = |id: &'static str, chat: String| move |st: &mut BeeperStore| st.upsert_message(new_message(st, id, &chat));
    let edit =
        |id: &'static str| move |st: &mut BeeperStore| drop(st.update_message(st.message_idx(id), |m| m.text = Some("edited".into())));
    let flip = |key: &'static str| move |st: &mut BeeperStore| st.settings.set_pref(key, (!st.settings.on(key)).into());
    let threads = |chat: String| {
        move |st: &mut BeeperStore| {
            st.features.flags["features"]["*"]["enable-threads"] = true.into();
            drop(st.update_chat(st.chat_idx(&chat), |c| c.threads = true));
        }
    };
    let kept = &mut rows_of(&app);
    assert!(!rows_changed(&app, kept, |_| ()), "a write lock alone keeps the rows");
    assert!(!rows_changed(&app, kept, upsert("$other", other.clone())), "a new message in another chat keeps them");
    assert!(!rows_changed(&app, kept, edit("$other")), "an edit in another chat keeps them");
    assert!(!rows_changed(&app, kept, |st| drop(st.remove_message(&large_id(7), "$other"))), "so does a removal");
    assert!(!rows_changed(&app, kept, threads(other.clone())), "and threads in another chat");
    assert!(rows_changed(&app, kept, upsert("$open", LARGE_OPEN.into())), "a new message in the open chat makes them");
    assert!(rows_changed(&app, kept, edit("$open")), "an edit");
    assert!(rows_changed(&app, kept, |st| drop(st.remove_message(LARGE_OPEN, "$open"))), "a removal");
    assert!(rows_changed(&app, kept, flip("hideDeleted")), "hideDeleted");
    assert!(rows_changed(&app, kept, flip("membershipEvents")), "membershipEvents");
    assert!(rows_changed(&app, kept, threads(LARGE_OPEN.into())), "the threads of the chat");
    // A removed chat compacts the loaded messages, which moves their indices.
    let moves = app.store.read().message_moves();
    assert!(rows_changed(&app, kept, |st| st.remove_chat(&other)), "a move of the messages");
    assert!(app.store.read().message_moves() > moves, "the removal moved the messages");
    app.store.write().config.demo = false;
    let local = app.store.write().send_message(LARGE_OPEN, Some("hi".into()), None, None).expect("a send").2.id;
    assert!(rows_changed(&app, kept, |_| ()) && kept.any_pending, "the local echo is pending");
    assert!(rows_changed(&app, kept, |st| drop(st.sent.insert(local, "$sent".into()))), "its send");
    assert!(!kept.any_pending, "the sent message is not pending");
    let merged = "!large_union_0:beeper.com";
    let _ = app.update(UiMessage::SelectChat(merged.into()));
    let member = {
        let st = app.store.read();
        st.chats[st.members(st.chat_idx(merged))[0]].id.clone()
    };
    let kept = &mut rows_of(&app);
    assert!(!rows_changed(&app, kept, upsert("$other2", large_id(8))), "a merged chat keeps its rows");
    assert!(rows_changed(&app, kept, upsert("$member", member)), "but not over a message of a member chat");
}

/// The space bar counts the unread chats of each space from the chats it counts, which it finds again after a write
/// to the store.
#[test]
fn the_space_bar_counts_follow_the_store() {
    let mut app = BeeperGuiApp::boot(sized_store("perf-counts", 200, 1_000)).0;
    let counted = |app: &BeeperGuiApp| app.counted_chats(&app.store.read()).to_vec();
    let before = counted(&app);
    let (st, i) = (app.store.read(), before[0]);
    let id = st.chats[i].id.clone();
    assert!(st.chats[i].is_unread() && !st.notifications_muted(&st.chats[i]));
    drop(st);
    frame(&app);
    let _ = app.update(UiMessage::ChatList(chat_list::Msg::MarkRead(id)));
    let after = counted(&app);
    assert_eq!(after, before[1..], "a read chat is not counted");
}
