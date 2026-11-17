//! Desktop's application menu (`menubar`), which Electron's `autoHideMenuBar` hides on Linux and Windows until Alt
//! shows it: the Beeper, File, Edit, View, Window and Help menus over the main window.
use crate::composer;
use crate::i18n::{tr, trf, trx};
use crate::settings;
use crate::style::{fill_box, shadow_box, text_width, txt};
use crate::windows;
use crate::{menu_item, menu_rule, BeeperGuiApp, El, SettingsTab, UiMessage, ASK_LOGOUT};
use iced::widget::{container, mouse_area, row, text_editor, Column};
use iced::{font::Weight, Fill, Padding};

/// The height of the menu bar.
pub(crate) const BAR_H: f32 = 26.0;
/// The padding of a menu title on each side, and the start of the first title.
const TITLE_PAD: f32 = 8.0;
const BAR_PAD: f32 = 4.0;
const MENU_W: f32 = 233.0;

/// An item of a menu: its label, its hotkey, and what it does, or `None` for a rule.
pub(crate) type Item = Option<(String, &'static str, UiMessage)>;

impl BeeperGuiApp {
    /// Desktop's `menubar` template, without what beeper-rs has nothing for: Check for Updates, Undo and Redo, and the
    /// roles of macOS only.
    pub(crate) fn menus(&self) -> Vec<(String, Vec<Item>)> {
        let st = self.store.read();
        let hotkey = |label: &str, name: &'static str| Some((label.to_string(), name, UiMessage::RunHotkey(name)));
        let plain = |label: String, msg: UiMessage| Some((label, "", msg));
        let url = |label: String, url: &str| plain(label, UiMessage::OpenUrl(url.into()));
        let logout = || plain(trf("Logout of %s...", &[&"Beeper"]), UiMessage::Ask(Some(ASK_LOGOUT)));
        let signed_in = st.signed_in();
        let mut beeper =
            vec![plain(trf("About %s", &[&"Beeper"]), UiMessage::Settings(settings::Msg::Open(SettingsTab::About)))];
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
        let main = self.main_window;
        let file = vec![
            hotkey(tr("Open..."), "Chat: Send File"),
            hotkey(tr("New Chat"), "Chat: Start New"),
            None,
            main.map(|id| (tr("Close Window").to_string(), "", UiMessage::Window(windows::Msg::CloseRequested(id)))),
            hotkey(tr("Open Chat in New Window"), "Open in New Window"),
        ];
        let edit = vec![
            plain(tr("Cut").into(), UiMessage::Composer(composer::Msg::Clip(true))),
            plain(tr("Copy").into(), UiMessage::Composer(composer::Msg::Clip(false))),
            plain(tr("Paste").into(), UiMessage::Composer(composer::Msg::Paste)),
            plain(tr("Select All").into(), UiMessage::Composer(composer::Msg::Edit(text_editor::Action::SelectAll))),
        ];
        let bar = if st.settings.on("showSidebar") { tr("Hide Space Bar") } else { tr("Show Space Bar") };
        let sidebar = if self.sidebar_hidden { tr("Show Sidebar") } else { tr("Hide Sidebar") };
        let view = vec![hotkey(bar, "Toggle Space Bar"), hotkey(sidebar, "Toggle Sidebar")];
        let window = vec![
            main.map(|id| (tr("Minimize").to_string(), "", UiMessage::Window(windows::Msg::Minimize(id)))),
            main.map(|id| (tr("Zoom").to_string(), "", UiMessage::Window(windows::Msg::ToggleMaximize(Some(id))))),
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
            (tr("Edit").into(), edit),
            (tr("View").into(), view),
            (tr("Window").into(), window),
            (tr("Help").into(), help),
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
                .on_press(UiMessage::MenuBar(Some((!on).then_some(i))))
                .on_enter(UiMessage::MenuBarHover(i))
                .into()
        });
        let bar = row(titles).padding(Padding::ZERO.left(BAR_PAD));
        container(bar).width(Fill).height(BAR_H).style(fill_box(p.window, 0.0)).into()
    }

    /// The open menu, and its left edge under its title.
    pub(crate) fn view_menu_dropdown(&self, open: usize) -> (El<'_>, f32) {
        let p = &self.pal;
        let mut menus = self.menus();
        let x = BAR_PAD
            + menus[..open].iter().map(|(t, _)| text_width(t, 13.0, Weight::Normal) + 2.0 * TITLE_PAD).sum::<f32>();
        let st = self.store.read();
        let rows = menus.swap_remove(open).1.into_iter().map(|item| match item {
            Some((label, hotkey, msg)) => {
                let keys = match hotkey {
                    "" => String::new(),
                    name => beeper_core::prefs::hotkey_label(st.settings.hotkey_keys(name)),
                };
                menu_item(p, None, label, keys, Some(UiMessage::MenuBarRun(Box::new(msg))))
            }
            None => menu_rule(p),
        });
        let menu = container(Column::with_children(rows).padding([4, 0]));
        (menu.width(MENU_W).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into(), x)
    }
}
