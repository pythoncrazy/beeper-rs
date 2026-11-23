//! Settings > Integrations: the Desktop API tokens and connections, Remote Access, and the MCP install.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// The "Install to..." choice, as an index into [`MCP_TARGETS`].
    McpTarget(usize),
    /// Installs the Beeper MCP server in a client of [`MCP_CLIENTS`] or [`MCP_TARGETS`].
    McpInstall(&'static str),
    /// Copies a value of Settings > Integrations; its copy button shows a check for 2 s.
    CopyField(u8, String),
    CopyDone(u8),
    /// A new token request of the Desktop API (no outcome), or the end of one.
    Request(String, Option<beeper_core::connect::Outcome>),
    Write,
    Expiry(Expiry),
    /// Approve or Decline in the "Approve connection" dialog.
    Answer(bool),
    Close,
    /// Revoke in Approved connections, which asks first.
    Revoke(String),
    /// The Remote Access toggle, which asks before it turns on.
    RemoteAccess(bool),
    RevokeConfirmed,
    /// The "+" of Approved connections opens the token dialog, and Escape closes it.
    Add(bool),
    AddName(String),
    AddExpiry(Expiry),
    AddWrite,
    Create,
    CopyToken,
}

impl BeeperGuiApp {
    pub(crate) fn integrations(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::McpTarget(i) => self.integrations.target = i,
            Msg::McpInstall(client) => {
                let value = mcp_value(client, &self.store.read().config.base_url());
                let app = match client {
                    "Claude Desktop" => return self.update(UiMessage::OpenUrl(value)),
                    "Cursor" => "Cursor",
                    "VS Code" => "Visual Studio Code",
                    _ => {
                        self.status_banner = tr("Command copied to clipboard! Run it in your terminal.").into();
                        return iced::clipboard::write(value);
                    }
                };
                self.status_banner = trf("Opening %s to install Beeper MCP...", &[&app]);
                return self.update(UiMessage::OpenUrl(value));
            }
            Msg::CopyField(i, value) => {
                self.integrations.copied = Some(i);
                let wait = async { tokio::time::sleep(std::time::Duration::from_secs(2)).await };
                let done = Task::perform(wait, move |()| UiMessage::Integrations(Msg::CopyDone(i)));
                return Task::batch([iced::clipboard::write(value), done]);
            }
            Msg::CopyDone(i) => {
                if self.integrations.copied == Some(i) {
                    self.integrations.copied = None;
                }
            }
            Msg::Request(id, None) => {
                let st = self.store.read();
                if let Some(r) = st.connect.requests.iter().find(|r| r.id == id) {
                    let name = r.client.name.clone().filter(|n| !n.is_empty());
                    let name = name.unwrap_or_else(|| tr("Unknown Client").into());
                    let version = r.client.version.as_ref().map(|v| format!("v{v}")).unwrap_or_default();
                    let write = r.scopes.iter().any(|s| s == beeper_core::connect::WRITE);
                    let app = format!("{name} {version}").trim_end().to_string();
                    self.token_ask = Some(TokenAsk { id, app, write, expiry: Expiry(2_592_000), timed_out: false });
                }
            }
            Msg::Request(id, Some(outcome)) => {
                if let Some(a) = self.token_ask.as_mut().filter(|a| a.id == id) {
                    a.timed_out = outcome == beeper_core::connect::Outcome::Timeout;
                }
            }
            Msg::Write => self.token_ask.as_mut().expect("an open token request").write ^= true,
            Msg::Expiry(e) => self.token_ask.as_mut().expect("an open token request").expiry = e,
            Msg::Answer(approve) => {
                let a = self.token_ask.take().expect("an open token request");
                let scopes = match a.write {
                    true => vec![beeper_core::connect::READ.into(), beeper_core::connect::WRITE.into()],
                    false => vec![beeper_core::connect::READ.into()],
                };
                self.store.write().answer_token_request(&a.id, approve.then_some((scopes, a.expiry.0)));
            }
            Msg::Close => self.token_ask = None,
            Msg::Revoke(id) => (self.integrations.revoke, self.confirm) = (Some(id), Some(ASK_REVOKE_CONNECTION)),
            Msg::RemoteAccess(true) if self.confirm != Some(ASK_REMOTE_ACCESS) => {
                self.confirm = Some(ASK_REMOTE_ACCESS);
            }
            Msg::RemoteAccess(on) => {
                self.confirm = None;
                self.patch_settings(|s| s.remote_access = on);
                self.relaunch_toast = Some(match on {
                    true => "Remote Access will be enabled after relaunch",
                    false => "Remote Access will be disabled after relaunch",
                });
            }
            Msg::RevokeConfirmed => {
                self.confirm = None;
                self.store.write().revoke_token(&self.integrations.revoke.take().expect("a connection to revoke"));
            }
            Msg::Add(open) => {
                self.integrations.add = open.then(|| AddToken {
                    name: String::new(),
                    expiry: Expiry(2_592_000),
                    write: false,
                    token: None,
                    copied: false,
                });
                return operation::focus("token-name");
            }
            Msg::AddName(name) => self.integrations.add.as_mut().expect("the token dialog").name = name,
            Msg::AddExpiry(e) => self.integrations.add.as_mut().expect("the token dialog").expiry = e,
            Msg::AddWrite => self.integrations.add.as_mut().expect("the token dialog").write ^= true,
            Msg::Create => {
                let a = self.integrations.add.as_mut().expect("the token dialog");
                let mut scopes = vec![beeper_core::connect::READ.to_string()];
                scopes.extend(a.write.then(|| beeper_core::connect::WRITE.to_string()));
                let client = (!a.name.is_empty())
                    .then(|| beeper_core::connect::ClientInfo { name: Some(a.name.clone()), ..Default::default() });
                a.token = Some(self.store.write().create_token(scopes, a.expiry.0, client));
            }
            Msg::CopyToken => {
                let a = self.integrations.add.as_mut().expect("the token dialog");
                a.copied = true;
                return iced::clipboard::write(a.token.clone().expect("a created token"));
            }
        }
        self.settle()
    }
}

/// Desktop's "Approve connection" dialog of a token request from the Desktop API.
pub(crate) struct TokenAsk {
    pub(crate) id: String,
    /// The client name and version.
    pub(crate) app: String,
    pub(crate) write: bool,
    pub(crate) expiry: Expiry,
    pub(crate) timed_out: bool,
}

impl BeeperGuiApp {
    /// Desktop's `BeeperConnectAccessTokenRequestModal`, over the main window.
    pub(crate) fn view_token_ask<'a>(&'a self, a: &'a TokenAsk) -> El<'a> {
        let p = &self.pal;
        let sentence = tr("A third-party app <app/> wants to access your chats through Beeper Desktop API.");
        let (before, after) = sentence.split_once("<app/>").expect("the sentence names the app");
        let main =
            rich_text![span::<(), _>(before), span(a.app.as_str()).font(fw(Weight::Bold)).color(p.strong), span(after)]
                .size(rem(15.0))
                .line_height(lh(20.0))
                .color(p.text)
                .align_x(Center)
                .width(Fill);
        let button = |label: &'static str, primary: bool, msg: UiMessage| -> El<'a> {
            let (bg, fg) = if primary { (p.accent, Color::WHITE) } else { (p.button, p.strong) };
            let label = txt(tr(label), 13.0).font(fw(Weight::Medium)).line_height(lh(16.0)).color(fg);
            button(container(label).center_y(Fill))
                .height(32)
                .padding([0, 12])
                .style(bordered_btn([bg; 3], [fg; 3], if primary { bg } else { p.button_border }, 1.0, 6.0))
                .on_press(msg)
                .into()
        };
        let body: El<'a> = match a.timed_out {
            true => {
                let notice = column![
                    semi(tr("The request has timed out."), 15.0).line_height(lh(20.0)).color(p.strong),
                    txt(tr("To try again you will need to relaunch the third-party app or re-request login in its MCP settings."), 13.0)
                        .color(p.text)
                ]
                .spacing(8);
                let notice = container(notice).width(Fill).padding([8, 16]).style(fill_box(p.chip, 8.0));
                column![notice, row![button("Close", false, UiMessage::Integrations(integrations::Msg::Close))]]
                    .spacing(16)
                    .into()
            }
            false => {
                let label = |t: &'static str| txt(tr(t), 12.0).color(p.text);
                let sensitive = column![
                    row![
                        label("Allow sensitive actions").width(Fill),
                        toggle(p, a.write, Some(UiMessage::Integrations(integrations::Msg::Write)))
                    ]
                    .align_y(Center)
                    .height(38)
                    .padding([0, 8]),
                    container(txt(tr("Grant permission to send messages and modify your account"), 11.0).color(p.sub))
                        .padding(Padding::new(8.0).top(0.0))
                ];
                let expiry = select(p, EXPIRIES, a.expiry, |a0| UiMessage::Integrations(integrations::Msg::Expiry(a0)));
                let expiry = row![label("Expires in").width(Fill), expiry].align_y(Center).height(38).padding([0, 8]);
                let rows = column![sensitive, divider(p.prefs_border, 0.0), expiry, divider(p.prefs_border, 0.0)];
                let group =
                    container(rows).width(Fill).padding(1).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
                let buttons = row![
                    button("Decline", false, UiMessage::Integrations(integrations::Msg::Answer(false))),
                    button("Approve", true, UiMessage::Integrations(integrations::Msg::Answer(true)))
                ]
                .spacing(8);
                let fine = txt(tr("Only approve if you initiated this connection and trust the application."), 12.0)
                    .color(p.sub)
                    .align_x(Center)
                    .width(Fill);
                column![
                    container(group).padding([8, 0]),
                    container(buttons).padding(Padding::new(0.0).top(16.0).bottom(8.0)),
                    fine
                ]
                .into()
            }
        };
        let title = container(semi(tr("Approve connection"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let content = column![container(main).padding(Padding::new(0.0).top(32.0).bottom(16.0)), body];
        let pane = column![title, container(content).padding([0, 16])];
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.prefs_pane, 10.0));
        mouse_area(container(pane).padding(8)).on_press(UiMessage::Noop).into()
    }
}
