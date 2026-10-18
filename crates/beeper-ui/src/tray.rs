//! Desktop's tray icon (`createTray`) as a StatusNotifierItem with a `com.canonical.dbusmenu` menu, and its app badge
//! (`app.badgeCount`) as a Unity launcher entry.
use std::collections::HashMap;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Structure};

const ITEM: &str = "/StatusNotifierItem";
const MENU: &str = "/MenuBar";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The tray host shows the icon.
    Up,
    Show,
    Hide,
    Quit,
}

type Pixmap = Vec<(i32, i32, Vec<u8>)>;

/// The PNG as one StatusNotifierItem pixmap: ARGB32 in network byte order.
fn pixmap(png_bytes: &[u8]) -> Pixmap {
    let img = ::image::load_from_memory(png_bytes).expect("the tray icon is a PNG").into_rgba8();
    let (w, h) = (img.width() as i32, img.height() as i32);
    vec![(w, h, img.chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect())]
}

struct Item {
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    unread: u32,
    icons: [Pixmap; 2],
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "Communications"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "beeper"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Beeper"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmap {
        self.icons[usize::from(self.unread > 0)].clone()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Pixmap, String, String) {
        let title = match self.unread {
            0 => "Beeper".to_string(),
            n => format!("Beeper [{n}]"),
        };
        (String::new(), Vec::new(), title, String::new())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> zbus::zvariant::ObjectPath<'_> {
        zbus::zvariant::ObjectPath::from_static_str_unchecked(MENU)
    }
    fn activate(&self, _x: i32, _y: i32) {
        _ = self.tx.send(Event::Show);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
    #[zbus(signal)]
    async fn new_icon(e: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(e: &SignalEmitter<'_>) -> zbus::Result<()>;
}

struct Menu {
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    visible: bool,
    revision: u32,
}

/// Desktop's tray menu: "Minimize to Tray" while the window shows, "Show Window" while it hides, then "Quit Beeper".
const ENTRIES: [(i32, &str, &str, Event); 3] = [
    (1, "Minimize to Tray", "Alt", Event::Hide),
    (2, "Show Window", "", Event::Show),
    (3, "Quit Beeper", "Control", Event::Quit),
];

impl Menu {
    fn items(&self) -> impl Iterator<Item = &'static (i32, &'static str, &'static str, Event)> + '_ {
        ENTRIES.iter().filter(|e| e.3 != if self.visible { Event::Show } else { Event::Hide })
    }
}

fn props(label: &str, modifier: &str) -> HashMap<String, OwnedValue> {
    let mut p = HashMap::from([("label".to_string(), OwnedValue::from(zbus::zvariant::Str::from(label.to_string())))]);
    if !modifier.is_empty() {
        let keys = vec![vec![modifier.to_string(), "q".to_string()]];
        p.insert("shortcut".into(), OwnedValue::try_from(zbus::zvariant::Value::from(keys)).expect("shortcut value"));
    }
    p
}

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
    fn get_layout(&self, parent_id: i32, _depth: i32, _names: Vec<String>) -> (u32, Layout) {
        let children = (parent_id == 0).then(|| self.items()).into_iter().flatten().map(|&(id, label, m, _)| {
            let item: Layout = (id, props(label, m), Vec::new());
            OwnedValue::try_from(Structure::from(item)).expect("menu item value")
        });
        let root =
            HashMap::from([("children-display".to_string(), OwnedValue::from(zbus::zvariant::Str::from("submenu")))]);
        (self.revision, (parent_id, root, children.collect()))
    }
    fn get_group_properties(&self, ids: Vec<i32>, _names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        self.items().filter(|e| ids.is_empty() || ids.contains(&e.0)).map(|&(id, l, m, _)| (id, props(l, m))).collect()
    }
    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let item = self.items().find(|e| e.0 == id);
        let mut p = item.map(|&(_, l, m, _)| props(l, m)).unwrap_or_default();
        p.remove(&name).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on item {id}")))
    }
    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if let Some(e) = ENTRIES.iter().find(|e| e.0 == id).filter(|_| event_id == "clicked") {
            _ = self.tx.send(e.3);
        }
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        events.into_iter().for_each(|(id, event_id, data, ts)| self.event(id, event_id, data, ts));
        Vec::new()
    }
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
    #[zbus(signal)]
    async fn layout_updated(e: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

/// Removes the tray when the subscription stops, as Desktop does when "Show tray icon" is turned off.
struct Registered(zbus::Connection, String);

impl Drop for Registered {
    fn drop(&mut self) {
        let (bus, name) = (self.0.clone(), std::mem::take(&mut self.1));
        tokio::spawn(async move {
            _ = bus.object_server().remove::<Item, _>(ITEM).await;
            _ = bus.object_server().remove::<Menu, _>(MENU).await;
            _ = bus.release_name(name).await;
        });
    }
}

async fn register(tx: tokio::sync::mpsc::UnboundedSender<Event>) -> Result<Registered, String> {
    let bus = crate::notify::bus().await?.clone();
    let icons =
        [pixmap(include_bytes!("../assets/tray/icon.png")), pixmap(include_bytes!("../assets/tray/unread.png"))];
    let server = bus.object_server();
    server.at(ITEM, Item { tx: tx.clone(), unread: 0, icons }).await.map_err(|e| e.to_string())?;
    server.at(MENU, Menu { tx, visible: true, revision: 1 }).await.map_err(|e| e.to_string())?;
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    bus.request_name(name.as_str()).await.map_err(|e| e.to_string())?;
    let registered = Registered(bus.clone(), name.clone());
    bus.call_method(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        Some("org.kde.StatusNotifierWatcher"),
        "RegisterStatusNotifierItem",
        &name,
    )
    .await
    .map_err(|e| format!("no tray host: {e}"))?;
    Ok(registered)
}

/// `Up`, then the tray clicks while the tray shows. A failed registration ends the stream with the error.
pub fn events() -> impl iced::futures::Stream<Item = Result<Event, String>> {
    iced::stream::channel(8, async |mut out: iced::futures::channel::mpsc::Sender<Result<Event, String>>| {
        use iced::futures::SinkExt;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _registered = match register(tx).await {
            Ok(r) => r,
            Err(e) => {
                _ = out.send(Err(e)).await;
                return;
            }
        };
        _ = out.send(Ok(Event::Up)).await;
        while let Some(e) = rx.recv().await {
            _ = out.send(Ok(e)).await;
        }
    })
}

/// Shows the unread count on the tray icon and the app badge, and the window state in the tray menu.
pub async fn update(unread: u32, visible: bool) -> Result<(), String> {
    let bus = crate::notify::bus().await?;
    let uri = format!("application://{}.desktop", beeper_core::config::APP_BUNDLE_ID);
    let badge = HashMap::from([
        ("count", OwnedValue::from(i64::from(unread))),
        ("count-visible", OwnedValue::from(unread > 0)),
    ]);
    bus.emit_signal(None::<&str>, "/", "com.canonical.Unity.LauncherEntry", "Update", &(uri, badge))
        .await
        .map_err(|e| e.to_string())?;
    let server = bus.object_server();
    if let Ok(item) = server.interface::<_, Item>(ITEM).await {
        let mut i = item.get_mut().await;
        if i.unread != unread {
            i.unread = unread;
            Item::new_icon(item.signal_emitter()).await.map_err(|e| e.to_string())?;
            Item::new_tool_tip(item.signal_emitter()).await.map_err(|e| e.to_string())?;
        }
    }
    if let Ok(menu) = server.interface::<_, Menu>(MENU).await {
        let mut m = menu.get_mut().await;
        if m.visible != visible {
            m.visible = visible;
            m.revision += 1;
            Menu::layout_updated(menu.signal_emitter(), m.revision, 0).await.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
