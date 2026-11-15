//! Desktop's `beeper://` links (`deep-link-CFIhWY4D.js` `Ld`): from the launch arguments, and from a later launch.
use crate::i18n::tr;
use crate::login::LoginMsg;
use crate::{composer_with, encode_uri_component, BeeperGuiApp, InboxTab, SettingsTab, Toast, UiMessage, User};
use beeper_core::codec::url_decode;
use beeper_core::config::{account_network, find_network, NETWORK_SPECS};
use beeper_core::models::NetworkSpec;
use iced::{window, Task};
use std::collections::HashMap;

pub const SCHEME: &str = "beeper://";

/// The networks whose Desktop platform name (`platform.name` without `bridge-`, Desktop's `Uae`) is not the
/// beeper-rs bridge ID.
const PLATFORMS: [(&str, &str); 7] = [
    ("discord", "discordgo"),
    ("slack", "slackgo"),
    ("tumblr", "tumblrdms"),
    ("facebook", "facebookgo"),
    ("instagram", "instagramgo"),
    ("imessage", "imessagecloud"),
    ("bluesky", "local-bluesky"),
];

fn platform(n: &NetworkSpec) -> &'static str {
    PLATFORMS.iter().find(|p| p.0 == n.bridge_id).map_or(n.bridge_id, |p| p.1)
}

/// The link of a chat, Desktop's "Copy Chat Link".
pub fn chat_link(account_id: &str, chat_id: &str) -> String {
    format!("{SCHEME}select-thread/{}/{}", platform(account_network(account_id)), encode_uri_component(chat_id))
}

/// Desktop's `decodeURIComponent`, which keeps `+`.
fn decode_uri_component(s: &str) -> Option<String> {
    url_decode(&s.replace('+', "%2B"))
}

/// Desktop's `ag`: the path parts and the query of a link, or `None` when a part does not decode. The parts of
/// `join-matrix-room` stay encoded.
fn parse(link: &str) -> Option<(Vec<String>, HashMap<String, String>)> {
    let rest = link.strip_prefix(SCHEME).expect("a beeper:// link");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let query = query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, "")));
    let query = query.map(|(k, v)| Some((url_decode(k)?, url_decode(v)?))).collect::<Option<_>>()?;
    let parts = path.split('/').map(str::to_string);
    let parts = match path.split('/').next() == Some("join-matrix-room") {
        true => parts.collect(),
        false => parts.map(|p| decode_uri_component(&p)).collect::<Option<_>>()?,
    };
    Some((parts, query))
}

/// Desktop's `registerLinuxConfig` desktop entry. Its name is the app ID, which the window and the launcher badge use.
#[cfg(target_os = "linux")]
fn desktop_entry(exe: &str, icon: &str) -> String {
    format!(
        "[Desktop Entry]\nName=Beeper\nExec=\"{exe}\" %u\nType=Application\nTerminal=false\n\
         Comment=The ultimate messaging app\nCategories=Network;Chat;\nMimeType=x-scheme-handler/beeper;\n\
         StartupWMClass={}\nStartupNotify=true\nIcon={icon}\n",
        beeper_core::config::APP_BUNDLE_ID
    )
}

/// Desktop's `registerLinuxConfig`: installs the desktop entry and the icon, and makes the entry the default handler
/// of `beeper://` links.
#[cfg(target_os = "linux")]
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quotable = exe.to_str().filter(|e| !e.contains(['"', '`', '$', '\\']));
    let exe = quotable.ok_or(format!("the program path {} has characters that Exec cannot quote", exe.display()))?;
    let home = std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
    let data = std::env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), Into::into);
    let name = format!("{}.desktop", beeper_core::config::APP_BUNDLE_ID);
    let (entry, icon) = (data.join("applications").join(&name), data.join("icons/beeper-rs.png"));
    let write = |path: &std::path::Path, bytes: &[u8]| {
        let dir = path.parent().expect("a file in a folder");
        let done = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(path, bytes));
        done.map_err(|e| format!("write {}: {e}", path.display()))
    };
    write(&icon, include_bytes!("../assets/icon.png"))?;
    write(&entry, desktop_entry(exe, &icon.to_string_lossy()).as_bytes())?;
    let mime = std::process::Command::new("xdg-mime").args(["default", &name, "x-scheme-handler/beeper"]).status();
    match mime.map_err(|e| format!("xdg-mime: {e}"))?.success() {
        true => Ok(()),
        false => Err("xdg-mime did not set the beeper:// handler".into()),
    }
}

impl BeeperGuiApp {
    /// Desktop's `second-instance` event: opens the last argument as a link, and shows the main window.
    pub(crate) fn second_instance(&mut self, args: Vec<String>) -> Task<UiMessage> {
        let link = args.last().map_or_else(Task::none, |l| self.open_link(l));
        Task::batch([link, self.show_main()])
    }

    fn show_main(&mut self) -> Task<UiMessage> {
        match self.main_window {
            Some(_) => self.set_hidden(false),
            None => window::gain_focus(self.login_window.expect("a window is open")),
        }
    }

    /// Desktop's `handleDeepLink`.
    pub(crate) fn open_link(&mut self, link: &str) -> Task<UiMessage> {
        if link == SCHEME || !link.starts_with(SCHEME) {
            return Task::none();
        }
        let show = self.show_main();
        Task::batch([show, self.follow(link)])
    }

    /// Desktop's `joinMatrixRoom` result for a link.
    pub(crate) fn room_joined(&mut self, r: Result<String, String>) {
        let text = match r {
            Ok(id) if self.store.read().resolve_chat_idx(&id).is_some() => return self.select_chat(&id),
            Ok(_) => tr("You joined the room, but Beeper couldn't open it yet. Try again."),
            Err(e) => {
                eprintln!("Failed to join Matrix room: {e}");
                tr("Couldn't join this Matrix room. Check the address and your access, then try again.")
            }
        };
        self.show(Toast::new(text.into()));
    }

    fn follow(&mut self, link: &str) -> Task<UiMessage> {
        let toast = |app: &mut Self, text: String| {
            app.show(Toast::new(text));
            Task::none()
        };
        let Some((parts, query)) = parse(link) else { return toast(self, "Invalid deep link".into()) };
        let (text, attachment) = (query.get("text").cloned(), query.get("attachmentPath").cloned());
        let part = |i: usize| parts.get(i).map(String::as_str);
        match part(0).expect("split gives a part") {
            "refresh-user" if self.main_window.is_some() && !self.store.read().config.demo => {
                return self.refresh_cloud_accounts();
            }
            "refresh-user" => return Task::none(),
            "fill-textarea" => {
                if let (Some(t), Some(_)) = (text, &self.pane.selected_chat_id) {
                    self.pane.composer = composer_with(&t);
                }
                return iced::widget::operation::focus("composer-input");
            }
            "join-matrix-room" => {
                let Some(room) = part(1).filter(|r| !r.is_empty()) else {
                    return toast(self, tr("No Matrix room ID was provided in the link.").into());
                };
                let Some(room) = decode_uri_component(room) else {
                    return toast(self, tr("The link contains an invalid Matrix room ID.").into());
                };
                if self.store.read().resolve_chat_idx(&room).is_some() {
                    self.select_chat(&room);
                    return Task::none();
                }
                let join = beeper_core::matrix::join(self.store.read().matrix.clone(), room);
                return Task::perform(join, UiMessage::RoomJoined);
            }
            "otp" => {
                let code = part(1).unwrap_or_default().to_string();
                return match self.login_window {
                    Some(_) => self.update_login(LoginMsg::Code(code)),
                    None => Task::none(),
                };
            }
            "focus" => {
                let id = query.get("windowID").map_or("main", String::as_str);
                let thread = id.strip_prefix("thread-");
                let window = match id {
                    "main" => None,
                    "prefs" => self.settings_window,
                    _ => self.chat_windows.iter().find(|w| w.1.selected_chat_id.as_deref() == thread).map(|w| w.0),
                };
                return window.map_or_else(Task::none, window::gain_focus);
            }
            "connect" => return self.open_settings(SettingsTab::Integrations),
            "tips" if part(1) == Some("add-account") && parts.len() == 2 => {
                let open = self.open_settings(SettingsTab::Accounts);
                self.adding_account = true;
                return open;
            }
            _ => {}
        }
        let name = part(1).map(|m| m.strip_prefix("bridge-").unwrap_or(m));
        let Some(network) = name.and_then(|m| NETWORK_SPECS.iter().find(|n| platform(n) == m)) else {
            return toast(self, "Invalid deep link".into());
        };
        let st = self.store.read();
        let account = query.get("accountID").filter(|a| st.accounts.iter().any(|x| x.account_id == **a)).cloned();
        let accounts: Vec<String> = match account {
            Some(a) => vec![a],
            None => {
                let on = |a: &String| find_network(a).is_some_and(|n| n.bridge_id == network.bridge_id);
                st.accounts.iter().map(|a| a.account_id.clone()).filter(on).collect()
            }
        };
        if accounts.is_empty() {
            drop(st);
            return toast(self, format!("Could not find any {} accounts", network.display_name));
        }
        let thread =
            part(2).and_then(|id| st.resolve_chat_idx(id)).filter(|&i| accounts.contains(&st.chats[i].account_id));
        let thread = thread.map(|i| (st.chats[i].id.clone(), st.chats[i].is_request == Some(true)));
        let at_key = |key: &str| {
            let (cid, _) = thread.as_ref()?;
            Some(st.stored("chat_id = ?1 AND sort_key = ?2", [cid.as_str(), key]).pop()?.id)
        };
        let sorted = part(3).and_then(at_key);
        drop(st);
        let missing = format!("Could not find {} thread", network.display_name);
        match part(0).expect("split gives a part") {
            "compose" => {
                let (kind, value) = (part(2).unwrap_or_default(), part(3).unwrap_or_default().to_string());
                let user = match kind {
                    "id" | "username" | "email" => User { id: value, ..User::default() },
                    "phone" => User { id: value.clone(), phone_number: Some(value), ..User::default() },
                    _ => return toast(self, format!("Could not compose {} chat", network.display_name)),
                };
                self.open_dm(accounts[0].clone(), user, None, text)
            }
            "select-thread" => {
                let Some((cid, request)) = thread else { return toast(self, missing) };
                if request {
                    let _ = self.update(UiMessage::SelectTab(InboxTab::Requests));
                }
                self.select_chat(&cid);
                if let Some(t) = text {
                    self.pane.composer = composer_with(&t);
                }
                let jump = sorted.map_or_else(Task::none, |mid| self.update(UiMessage::JumpTo(cid, mid)));
                let files = attachment.map(|p| self.update(UiMessage::FilesPicked(Ok(vec![p.into()]))));
                Task::batch([jump].into_iter().chain(files))
            }
            "highlight-message" => {
                let Some((cid, _)) = thread else { return toast(self, missing) };
                let mid = part(3).unwrap_or_default().to_string();
                self.update(UiMessage::JumpTo(cid, mid))
            }
            _ => toast(self, format!("Unrecognized deep link: {link}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse_like_desktop() {
        let (parts, query) = parse("beeper://select-thread/whatsapp/%21a%3Ab.c/k?text=hi+there&accountID=x").unwrap();
        assert_eq!(parts, ["select-thread", "whatsapp", "!a:b.c", "k"]);
        assert_eq!((query["text"].as_str(), query["accountID"].as_str()), ("hi there", "x"));
        assert_eq!(parse("beeper://join-matrix-room/%23a%3Ab.c").unwrap().0[1], "%23a%3Ab.c");
        assert!(parse("beeper://select-thread/%zz").is_none());
        let n = |id| NETWORK_SPECS.iter().find(|n| n.bridge_id == id).unwrap();
        assert_eq!(
            [platform(n("whatsapp")), platform(n("facebook")), platform(n("hungryserv"))],
            ["whatsapp", "facebookgo", "hungryserv"]
        );
        assert_eq!(chat_link("acc_instagram", "!r:beeper.com"), "beeper://select-thread/instagramgo/!r%3Abeeper.com");
        #[cfg(target_os = "linux")]
        assert!(desktop_entry("/a b/beeper-rs", "/i.png").contains("\nExec=\"/a b/beeper-rs\" %u\n"));
    }

    #[test]
    fn links_open_like_desktop() {
        let mut app = crate::fuzz::demo_app("deeplink");
        let (cid, account, mid) = {
            let st = app.store.read();
            let c = st.chats.iter().find(|c| c.account_id == "acc_whatsapp").expect("a WhatsApp chat");
            let first = st.chat_msg_indices[&c.id].first().expect("the chat has messages");
            (c.id.clone(), c.account_id.clone(), st.messages[*first].id.clone())
        };
        let open = |app: &mut BeeperGuiApp, link: &str| {
            app.status_banner.clear();
            let _ = app.open_link(link);
            app.status_banner.clone()
        };
        let _ = app.update(UiMessage::RunHotkey("Minimize To Tray"));
        assert_eq!(open(&mut app, &format!("{}?text=hi", chat_link(&account, &cid))), "");
        assert!(!app.hidden, "a link shows the main window");
        assert_eq!(app.pane.selected_chat_id.as_deref(), Some(cid.as_str()), "the copied link opens its chat");
        assert_eq!(app.pane.composer.text(), "hi");
        let _ = app.update(UiMessage::SelectChat(crate::NOTE_CHAT_ID.into()));
        assert_eq!(open(&mut app, &format!("beeper://highlight-message/bridge-whatsapp/{cid}/{mid}")), "");
        assert_eq!(app.flash.as_ref().map(|f| f.0.as_str()), Some(mid.as_str()));
        let cases = [
            ("beeper://select-thread/%zz", "Invalid deep link"),
            ("beeper://select-thread/nonet/x", "Invalid deep link"),
            ("beeper://select-thread/whatsapp/nope", "Could not find WhatsApp thread"),
            ("beeper://select-thread/linkedin/x", "Could not find any LinkedIn accounts"),
            ("beeper://subscriptions/whatsapp", "Unrecognized deep link: beeper://subscriptions/whatsapp"),
            ("beeper://join-matrix-room", "No Matrix room ID was provided in the link."),
            ("beeper://join-matrix-room/%zz", "The link contains an invalid Matrix room ID."),
            ("https://example.com", ""),
        ];
        for (link, toast) in cases {
            assert_eq!(open(&mut app, link), toast, "{link}");
        }
        assert_eq!(open(&mut app, "beeper://connect"), "");
        assert_eq!(app.active_settings_tab, SettingsTab::Integrations);
        let _ = open(&mut app, "beeper://tips/add-account");
        assert!(app.active_settings_tab == SettingsTab::Accounts && app.adding_account);
    }
}
