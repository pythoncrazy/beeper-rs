//! The speed of the main window on a large account (`beeper_core::testing::large_store`): the frames of the open
//! chat, Chat Info and the chat list, and the common actions. The views keep what they compute from the store until
//! the store changes, so each frame is timed both with that memo and right after a store write, which empties it.
//!
//! `cargo test --release -p beeper-ui --lib perf::profile -- --ignored --nocapture` prints the times.

use super::*;
use beeper_core::store::StoreLock;
use beeper_core::testing::{large_store, widen_large, LARGE_OPEN};
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
    let layout =
        median(11, || drop(Simulator::with_size(Default::default(), Size::new(1440.0, 900.0), app.view_main())));
    time("layout of the main window from a new widget tree", 100, layout);
    let scrolled = median(11, || {
        simulate(&mut app, Point::new(900.0, 450.0), wheel(300.0));
        frame(&app);
    });
    time("scroll the timeline: new layout, scroll, updates, frame", 100, scrolled);
    let template = app.store.read().stored_message("$large_66").expect("a message of the large chat");
    let (mut seq, mut room) = (0, 0);
    // Like the sync of each room.
    let mut sync = |app: &mut BeeperGuiApp, rooms: usize| {
        let mut rx = app.store.read().ws_tx.subscribe();
        let mut st = app.store.write();
        for _ in 0..rooms {
            room += 1;
            let chat_id = large_id(1 + room * 37 % 4999);
            let account_id = st.chat(&chat_id).account_id.clone();
            for _ in 0..100 / rooms {
                seq += 1;
                let (id, sort_key) = (format!("$sync_{seq}"), beeper_core::store::sort_key(1_000_000 + seq));
                let (chat_id, account_id) = (chat_id.clone(), account_id.clone());
                st.upsert_message(Message { id, chat_id, account_id, sort_key, ..template.clone() });
            }
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
