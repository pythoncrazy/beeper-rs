//! Desktop's application menu (`menubar`), which Electron's `autoHideMenuBar` hides on Linux and Windows until Alt
//! shows it: the Beeper, File, Edit, View, Window and Help menus over the main window.
use crate::composer;
use crate::i18n::{tr, trf, trx};
use crate::prelude::*;
use crate::settings;
use crate::windows;
use iced::widget::column;

/// The height of the menu bar.
pub(crate) const BAR_H: f32 = 26.0;
/// The padding of a menu title on each side, and the start of the first title.
const TITLE_PAD: f32 = 8.0;
const BAR_PAD: f32 = 4.0;
const MENU_W: f32 = 233.0;

/// The text of the first combination of hotkey `keys` in Electron's native menus, which put the modifiers in
/// Chromium's order (Alt, Ctrl, Shift) with "+" between them: "Alt+Ctrl+Q" for `ctrl+alt+q`.
pub(crate) fn accelerator(keys: &str) -> String {
    let combo = keys.split(", ").next().expect("split gives one part");
    if combo == "disabled" {
        return String::new();
    }
    let mut key = combo.rsplit('+').next().expect("rsplit gives one part").chars();
    let key: String = key.next().into_iter().flat_map(char::to_uppercase).chain(key).collect();
    let mods = [("alt", "Alt"), ("ctrl", "Ctrl"), ("shift", "Shift")];
    let mods = mods.into_iter().filter(|(m, _)| combo.split('+').any(|p| p == *m)).map(|(_, name)| name);
    mods.chain([key.as_str()]).collect::<Vec<_>>().join("+")
}

/// An item of a menu: its label, its accelerator, and what it does, or `None` for a rule.
pub(crate) type Item = Option<(String, String, UiMessage)>;

/// An item of one of Electron's roles, whose English label and accelerator Desktop does not translate.
fn role(label: &str, keys: &str, msg: UiMessage) -> Item {
    Some((label.into(), keys.into(), msg))
}

impl BeeperGuiApp {
    /// Desktop's `menubar` template, without what beeper-rs has nothing for: Check for Updates and the roles of macOS
    /// only.
    pub(crate) fn menus(&self) -> Vec<(String, Vec<Item>)> {
        let st = self.store.read();
        let hotkey = |label: &str, name: &'static str| {
            Some((label.to_string(), accelerator(st.settings.hotkey_keys(name)), UiMessage::RunHotkey(name)))
        };
        let plain = |label: String, msg: UiMessage| Some((label, String::new(), msg));
        let url = |label: String, url: &str| plain(label, UiMessage::OpenUrl(url.into()));
        let signed_in = st.signed_in();
        // The sign-in window shows no dialog, so its Logout logs out at once like the Beeper menu of Desktop, whose
        // `logout` gets the menu item as its reason. Before sign-in, Desktop's `doLogout` only relaunches the app.
        let logout = || {
            let msg = match (self.main_window, signed_in) {
                (Some(_), _) => UiMessage::Ask(Some(Confirm::Logout(None))),
                (None, true) => UiMessage::Logout,
                (None, false) => UiMessage::Window(windows::Msg::Relaunch),
            };
            plain(trf("Logout of %s...", &[&"Beeper"]), msg)
        };
        let mut beeper = vec![plain(trf("About %s", &[&"Beeper"]), UiMessage::Settings(settings::Msg::Open(SettingsTab::About)))];
        if signed_in {
            beeper.extend([
                None,
                plain(tr("Accounts…").into(), UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts))),
                hotkey(tr("Preferences"), "Settings"),
                None,
                hotkey(tr("Command Bar…"), "Command Bar"),
                hotkey(tr("Search..."), "Search"),
            ]);
        }
        beeper.push(None);
        if st.settings.on("showTrayIcon") {
            beeper.extend([hotkey(&trf("Minimize to %s", &[&"Tray"]), "Minimize To Tray"), None]);
        }
        beeper.extend([logout(), hotkey(&trf("Quit %s", &[&"Beeper"]), "Quit App")]);
        // The menu bar of the sign-in window acts on it, which Desktop does not let maximize.
        let main = self.main_window.or(self.login_window);
        let file = vec![
            hotkey(tr("Open..."), "Chat: Send File"),
            hotkey(tr("New Chat"), "Chat: Start New"),
            None,
            main.and_then(|id| role("Close", "Ctrl+W", UiMessage::Window(windows::Msg::CloseRequested(id)))),
            // Desktop's `openChatInNewWindow` opens the open chat, and does nothing without one.
            plain(
                tr("Open Chat in New Window").into(),
                self.pane.selected_chat_id.clone().map_or(UiMessage::Noop, |c| UiMessage::Window(windows::Msg::OpenChat(c))),
            ),
        ];
        let text = |a: text_editor::Action| UiMessage::Composer(composer::Msg::Edit(a));
        // Chromium's Delete of the menu deletes a selection, and does nothing at a caret.
        let delete = match self.pane.composer.selection() {
            Some(_) => text(text_editor::Action::Edit(text_editor::Edit::Delete)),
            None => UiMessage::Noop,
        };
        // Electron's `editMenu` role.
        let edit = vec![
            role("Undo", "Ctrl+Z", UiMessage::Composer(composer::Msg::History(false))),
            role("Redo", "Ctrl+Shift+Z", UiMessage::Composer(composer::Msg::History(true))),
            None,
            role("Cut", "Ctrl+X", UiMessage::Composer(composer::Msg::Clip(true))),
            role("Copy", "Ctrl+C", UiMessage::Composer(composer::Msg::Clip(false))),
            role("Paste", "Ctrl+V", UiMessage::Composer(composer::Msg::Paste)),
            role("Delete", "", delete),
            None,
            role("Select All", "Ctrl+A", text(text_editor::Action::SelectAll)),
        ];
        let bar = if st.settings.on("showSidebar") { tr("Hide Space Bar") } else { tr("Show Space Bar") };
        let sidebar = if self.sidebar_hidden { tr("Show Sidebar") } else { tr("Hide Sidebar") };
        let view = vec![hotkey(bar, "Toggle Space Bar"), hotkey(sidebar, "Toggle Sidebar")];
        // Desktop's Window menu: Electron runs the macOS roles Zoom and Bring All to Front on macOS only.
        let window = vec![
            role("Zoom", "", UiMessage::Noop),
            main.and_then(|id| role("Minimize", "Ctrl+M", UiMessage::Window(windows::Msg::Minimize(id)))),
            role("Bring All to Front", "", UiMessage::Noop),
        ];
        let mut help = vec![
            hotkey(tr("Report a Problem"), "Report a Problem"),
            url(trf("Open %s Help", &[&"Beeper"]), "https://help.beeper.com/?ref=desktop-app"),
            None,
            url(trf("Open %s", &[&"Beeper.com"]), "https://www.beeper.com/"),
            url(trf("Follow %s on X", &[&"@Beeper"]), "https://www.beeper.com/twitter"),
            None,
            url(tr("Changelog").into(), "https://www.beeper.com/changelog/desktop"),
            // Electron's `&&` is a literal `&`.
            url(tr("Privacy && Security").replace("&&", "&"), "https://www.beeper.com/privacy"),
            hotkey(tr("Keyboard Shortcuts"), "Keyboard Shortcuts"),
        ];
        if signed_in {
            help.extend([None, logout()]);
        }
        vec![
            ("Beeper".into(), beeper),
            (trx("File", "desktop menu title").into(), file),
            ("Edit".into(), edit),
            (tr("View").into(), view),
            ("Window".into(), window),
            ("Help".into(), help),
        ]
    }

    /// The menu bar under the title bar, with the title of the open menu raised. A click opens or closes a menu, and
    /// with a menu open the pointer opens the menu that it comes onto.
    pub(crate) fn view_menu_bar(&self, open: Option<usize>) -> El<'_> {
        let p = &self.pal;
        let titles = self.menus().into_iter().enumerate().map(|(i, (title, _))| -> El<'_> {
            let on = open == Some(i);
            let label = container(txt(title, 13.0).color(p.strong)).padding([0.0, TITLE_PAD]).center_y(BAR_H);
            let label = container(label).style(fill_box(if on { p.selected } else { p.window }, 4.0));
            mouse_area(label)
                .on_press(UiMessage::MenuBar(Msg::Show(Some((!on).then_some(i)))))
                .on_enter(UiMessage::MenuBar(Msg::Hover(i)))
                .into()
        });
        let bar = row(titles).padding(Padding::ZERO.left(BAR_PAD));
        container(bar).width(Fill).height(BAR_H).style(fill_box(p.window, 0.0)).into()
    }

    /// `base` under the menu bar when Alt shows it, and the open menu, which goes over all the window.
    pub(crate) fn with_menu_bar<'a>(&'a self, base: El<'a>) -> (El<'a>, Option<El<'a>>) {
        let Some(open) = self.menu_bar else { return (base, None) };
        let drop = open.map(|i| {
            let (menu, x) = self.view_menu_dropdown(i);
            let at = Padding::ZERO.left(x).top(BAR_H);
            crate::backdrop(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MenuBar(Msg::Show(None)))
                .into()
        });
        (column![self.view_menu_bar(open), base].into(), drop)
    }

    /// The open menu, and its left edge under its title.
    pub(crate) fn view_menu_dropdown(&self, open: usize) -> (El<'_>, f32) {
        let p = &self.pal;
        let mut menus = self.menus();
        let x = BAR_PAD + menus[..open].iter().map(|(t, _)| text_width(t, 13.0, Weight::Normal) + 2.0 * TITLE_PAD).sum::<f32>();
        let rows = menus.swap_remove(open).1.into_iter().map(|item| match item {
            Some((label, keys, msg)) => menu_item(p, None, label, keys, Some(UiMessage::MenuBar(Msg::Run(Box::new(msg))))),
            None => menu_rule(p),
        });
        let menu = Column::with_children(rows);
        (crate::menu_card(p, menu, MENU_W).into(), x)
    }
}

#[derive(Clone)]
pub enum Msg {
    /// Desktop's menu bar: hidden (`None`), shown, or shown with a menu open.
    Show(Option<Option<usize>>),
    /// The pointer came onto a menu title, which opens its menu while another menu is open.
    Hover(usize),
    /// An item of the menu bar: the menu bar hides, and the item runs.
    Run(Box<UiMessage>),
}

impl BeeperGuiApp {
    pub(crate) fn menu_bar(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Show(state) => self.menu_bar = state,
            Msg::Hover(i) => {
                if let Some(Some(_)) = self.menu_bar {
                    self.menu_bar = Some(Some(i));
                }
            }
            Msg::Run(msg) => {
                self.menu_bar = None;
                return self.update(*msg);
            }
        }
        self.settle()
    }
}
