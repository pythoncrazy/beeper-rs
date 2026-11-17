//! Desktop's command bar view: the palette of chats, commands and messages, and the search filter chips.

use super::*;
use crate::time::list_ts;
use iced::widget::column;

impl BeeperGuiApp {
    pub(crate) fn view_search_select(&self, chip: usize, at: iced::Rectangle) -> El<'_> {
        let p = &self.pal;
        let f = &self.search_filters;
        let pick = |m: SearchPick| Some(UiMessage::CommandBar(commands::Msg::PickSearchFilter(m)));
        let radio = |on: bool, label: &str, m: SearchPick| {
            let lead = container(on.then(|| ic(icon::CHECK, 14.0))).center_x(14).padding(Padding::ZERO.right(6.0));
            menu_item(p, Some(lead.into()), label.into(), "", pick(m))
        };
        let menu = |items: Vec<El<'static>>| -> El<'static> {
            container(Column::with_children(items).padding([4, 0]))
                .width(233)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into()
        };
        let brand = |options: Vec<(String, bool, SearchPick)>| -> El<'static> {
            let any = options.iter().any(|o| o.1);
            let widest = options.iter().map(|o| crate::style::text_width(&o.0, 11.0, Weight::Semibold));
            let width = widest.fold(at.width, |w, t| w.max(t + 48.0));
            let hover = Color { a: 0.1, ..p.text };
            let options = options.into_iter().map(|(label, on, m)| {
                let fg = if on || !any { p.text } else { p.sub };
                let line = row![
                    semi(label, 11.0).color(fg),
                    Space::new().width(Fill),
                    ic(icon::CHECK, 16.0).color(if on { fg } else { CLEAR })
                ]
                .spacing(8)
                .align_y(Center);
                let style = btn_style([CLEAR, hover, hover], [fg; 3], 4.0);
                button(line)
                    .padding([6, 4])
                    .width(Fill)
                    .style(style)
                    .on_press(UiMessage::CommandBar(commands::Msg::PickSearchFilter(m)))
                    .into()
            });
            let head = row![
                semi(f.label(chip).unwrap_or_else(|| tr(SEARCH_CHIPS[chip]).into()), 13.0).color(p.text),
                Space::new().width(Fill),
                ic(icon::EXPAND_MORE, 14.0).color(p.text)
            ]
            .spacing(4)
            .padding([0, 12])
            .align_y(Center);
            let body = column![head, Column::with_children(options).spacing(4).padding([0, 8])].spacing(8);
            container(body).width(width).padding([4, 0]).style(shadow_box(p.chip, CLEAR, 8.0, 4.0)).into()
        };
        match chip {
            0 => {
                let st = self.store.read();
                let mut accounts: Vec<String> = std::iter::once("hungryserv".into())
                    .chain(st.accounts.iter().map(|a| a.account_id.clone()))
                    .collect();
                accounts.sort_by_key(|a| network_label(a));
                brand(
                    accounts
                        .into_iter()
                        .map(|a| (network_label(&a).to_string(), f.accounts.contains(&a), SearchPick::Account(a)))
                        .collect(),
                )
            }
            1 => menu(vec![
                radio(f.sent == Some(true), tr("Sent"), SearchPick::Sent(true)),
                radio(f.sent == Some(false), tr("Received"), SearchPick::Sent(false)),
                menu_rule(p),
                radio(f.group == Some(true), trx("Group", "general search chat type filter"), SearchPick::Group(true)),
                radio(f.group == Some(false), tr("Direct message"), SearchPick::Group(false)),
            ]),
            2 => menu(
                PERIODS
                    .iter()
                    .enumerate()
                    .map(|(i, l)| radio(f.period == Some(i), tr(l), SearchPick::Period(i)))
                    .collect(),
            ),
            3 => brand(
                MEDIA_KINDS
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (tr(l).to_string(), f.media.contains(&i), SearchPick::Media(i)))
                    .collect(),
            ),
            _ => unreachable!("chip {chip} has no menu"),
        }
    }

    pub(crate) fn view_command_palette<'a>(&'a self) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let room = self.search_room.as_deref();
        let placeholder = match (self.cmd_bar_commands_mode, room) {
            _ if self.cmd_submenu.is_some() => self.cmd_submenu.as_ref().map(|m| m.placeholder()).unwrap_or_default(),
            (true, _) => tr("Search commands").into(),
            (false, Some(_)) => tr("Search this chat").into(),
            (false, None) => tr("Search chats").into(),
        };
        let input = text_input(&placeholder, &self.command_palette_query)
            .id("cmd-palette-input")
            .on_input(|a0| UiMessage::CommandBar(commands::Msg::Query(a0)))
            .size(13)
            .padding([6, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let kbd = |label: &'static str| {
            container(semi(label, 13.0).color(p.sub))
                .center_x(Shrink)
                .center_y(20)
                .padding([0, 2])
                .style(fill_box(Color { a: 0.1, ..p.strong }, 4.0))
        };
        let frame = |body: Column<'a, UiMessage>| -> El<'a> {
            container(body)
                .width(602)
                .height(360)
                .style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 4.0, 84.0))
                .into()
        };
        let mode_btn = button(
            row![
                txt(if self.cmd_bar_commands_mode { tr("Chat") } else { tr("Commands") }, 13.0).color(p.sub),
                kbd("Ctrl"),
                kbd(if self.cmd_bar_commands_mode { "K" } else { "J" })
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([2, 6])
        .style(ghost(p, 8.0))
        .on_press(UiMessage::CommandBar(commands::Msg::ToggleMode));
        let schedule = matches!(self.cmd_submenu, Some(commands::SubMenu::Schedule(_)));
        let no_reply = [tr("If no reply"), if schedule { tr("Regardless") } else { tr("Always") }];
        let inputs = match &self.cmd_submenu {
            Some(commands::SubMenu::Remind(_) | commands::SubMenu::Schedule(_)) => {
                let pick = move |c: &str| UiMessage::CommandBar(commands::Msg::NoReply(c == no_reply[0]));
                let pad = Padding::new(4.0).left(6.0).right(22.0);
                row![input, sized_select(p, no_reply, no_reply[usize::from(!self.cmd_no_reply)], pick, 13.0, pad)]
            }
            Some(_) => row![input],
            None => row![ic(icon::SEARCH, 14.0).color(p.sub), input, mode_btn],
        };
        let inputs = inputs.spacing(8).align_y(Center).padding(Padding::new(6.0).left(13.0).right(8.0));
        let q = self.command_palette_query.trim().to_lowercase();
        if self.cmd_bar_commands_mode {
            let cmds = self.commands(&st, self.command_palette_query.trim());
            let header = self.cmd_submenu.as_ref().filter(|m| !m.heading().is_empty()).map(|m| {
                let heading = txt(m.heading(), 13.0).color(p.strong);
                let clock = match m {
                    commands::SubMenu::AddMembers => Some(tinted(&ADD_ACCOUNT, 20.0, p.strong)),
                    _ => schedule.then(|| tinted(&COMPOSER_ICONS[6], 20.0, p.strong)),
                };
                container(row![].push(clock).push(heading).spacing(4).align_y(Center))
                    .width(Fill)
                    .center_y(44)
                    .padding([8, 13])
                    .style(fill_box(p.chip, iced::border::top(18)))
            });
            let footer = (schedule || matches!(self.cmd_submenu, Some(commands::SubMenu::Remind(_)))).then(|| {
                let note = match (schedule, self.cmd_no_reply) {
                    (true, _) => trf(
                        "Send Later messages can only send if %s app is running (and your computer is awake)",
                        &[&"Beeper"],
                    ),
                    (_, true) => {
                        tr("Remind Me marks this chat as new at the scheduled time, if there is no reply.").into()
                    }
                    (_, false) => tr("Remind Me marks this chat as new at the scheduled time").into(),
                };
                let line =
                    container(Space::new()).width(Fill).height(2).style(fill_box(p.strong.scale_alpha(0.1), 0.0));
                let note = container(txt(note, 13.0).color(p.sub)).center_y(44).padding([8, 13]);
                column![line, container(note).width(Fill).style(fill_box(p.chip, iced::border::bottom(18)))]
            });
            let body = column![].push(header).push(inputs).push(self.view_commands(&st, &cmds));
            return frame(body.push(footer));
        }
        let hint =
            self.pane().selected_chat_id.as_deref().and_then(|cid| st.resolve_chat_idx(cid).map(|i| &st.chats[i])).map(
                |c| {
                    let key =
                        |name| prefs::hotkey_label(st.settings.hotkey_keys(name).split(", ").next().unwrap_or(""));
                    let label = match (room, c.chat_type == "group") {
                        (Some(_), _) => trf("Press %s to search all of your conversations", &[&key("Search")]),
                        (None, true) => {
                            trf("Press %1$s to search your conversation in %2$s", &[&key("Search: Room"), &c.title])
                        }
                        (None, false) => {
                            trf("Press %1$s to search your conversation with %2$s", &[&key("Search: Room"), &c.title])
                        }
                    };
                    row![tinted(&DROP_ARROW, 20.0, p.sub), txt(label, 13.0).color(p.text)]
                        .spacing(16)
                        .align_y(Center)
                        .padding([8, 24])
                },
            );
        let f = &self.search_filters;
        let chips = Row::with_children(SEARCH_CHIPS.iter().enumerate().map(|(i, &placeholder)| {
            let disabled = room.is_some() && i == 0;
            let fg = if disabled { p.text.scale_alpha(0.3) } else { p.text };
            let label = f.label(i);
            let style = match label {
                Some(_) => bordered_box(CLEAR, p.text, 1.0, 20.0),
                None => bordered_box(p.chip, CLEAR, 1.0, 20.0),
            };
            let body = row![
                semi(label.unwrap_or_else(|| tr(placeholder).into()), 13.0).color(fg),
                ic(icon::EXPAND_MORE, 14.0).color(fg)
            ]
            .spacing(4)
            .align_y(Center);
            let chip = button(body).padding([4, 13]).style(btn_style([CLEAR; 3], [fg; 3], 20.0));
            container(
                chip.on_press_maybe((!disabled).then_some(UiMessage::CommandBar(commands::Msg::OpenSearchSelect(i)))),
            )
            .id(format!("search-chip-{i}"))
            .style(style)
            .into()
        }))
        .spacing(12)
        .padding(Padding::new(6.0).left(13.0).right(13.0));
        let filtered = *f != SearchFilters::default();
        let mut chats: Vec<&Chat> = if room.is_some() || !f.media.is_empty() || (filtered && q.is_empty()) {
            Vec::new()
        } else if q.is_empty() && !st.recent_chat_ids.is_empty() {
            st.recent_chat_ids.iter().filter_map(|id| st.find_chat(id)).filter(|c| c.flags & CHAT_HIDDEN == 0).collect()
        } else {
            // The view shows 12 chats at most, and one more tells that there are more.
            let (order, words) = self.chat_words(&st);
            let hit = |&&i: &&usize| {
                let c = &st.chats[i];
                c.flags & CHAT_HIDDEN == 0
                    && (f.accounts.is_empty() || f.accounts.contains(&c.account_id))
                    && words[i].contains(&q)
            };
            order.iter().filter(hit).take(13).map(|&i| &st.chats[i]).collect()
        };
        if q.is_empty() && chats.len() > 1 {
            chats.retain(|c| self.pane().selected_chat_id.as_deref() != Some(c.id.as_str()));
        }
        let has_more = chats.len() > 5;
        let limit = if self.search_expanded { 12 } else { 5 };
        let cat = row![
            txt(if q.is_empty() { tr("Recent chats") } else { tr("Chats") }, 12.0).color(p.sub),
            Space::new().width(Fill)
        ]
        .push(has_more.then(|| {
            button(semi(if self.search_expanded { tr("Show less") } else { tr("Show more") }, 11.0).color(p.accent))
                .padding(0)
                .style(ghost(p, 4.0))
                .on_press(UiMessage::CommandBar(commands::Msg::ToggleSearchExpanded))
        }))
        .align_y(Center)
        .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0));
        let hits = self.search_hits(&st, &q);
        let found: Vec<&Message> = hits.0.iter().collect();
        let media: Vec<&Message> = hits.1.iter().collect();
        let none =
            found.is_empty() && chats.is_empty() && media.is_empty() && (q.chars().count() > 1 || !f.media.is_empty());
        let heading = |label: &'static str| {
            container(txt(label, 12.0).color(p.sub)).padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        };
        let h24 = st.settings.pref("timeFormat") == 1;
        let needle = q.to_ascii_lowercase();
        let messages =
            found.iter().fold(Column::new().push((!found.is_empty()).then(|| heading(tr("Messages")))), |col, m| {
                let c = st.chat(&m.chat_id);
                let mine = m.is_sender == Some(true);
                let label = match (c.chat_type == "group", mine) {
                    (false, _) => c.title.clone(),
                    (true, true) => format!("{} • {}", c.title, chat_network(c)),
                    (true, false) => {
                        format!("{} in {} • {}", sender_label(c, m), c.title, chat_network(c))
                    }
                };
                let head = row![
                    txt(label, 11.0).line_height(Pixels(7.0)).color(p.text),
                    Space::new().width(Fill),
                    txt(list_ts(&m.timestamp, h24), 11.0).line_height(Pixels(7.0)).color(p.text)
                ];
                let (bg, fg) = if mine { (p.sent, p.sent_fg) } else { (p.received, p.strong) };
                let body = clamped(m.text.as_deref().expect("text message"), 554.0, 3, 13.0, Weight::Normal);
                let lower = body.to_ascii_lowercase();
                let (mut spans, mut at): (Vec<iced::widget::text::Span<()>>, _) = (Vec::new(), 0);
                for (k, hit) in lower.match_indices(&needle) {
                    spans.push(span(body[at..k].to_string()).size(13.0).color(fg));
                    let mark = span(body[k..k + hit.len()].to_string()).size(13.0);
                    spans.push(mark.color(hex(0x202124)).background(hex(0xf6ce46)));
                    at = k + hit.len();
                }
                spans.push(span(body[at..].to_string()).size(13.0).color(fg));
                let bubble = rich_text(spans)
                    .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
                let bubble = container(container(bubble).padding([8, 11]).style(fill_box(bg, 17.0)))
                    .width(Fill)
                    .align_x(if mine { Right } else { iced::Left });
                col.push(
                    button(column![head, bubble].spacing(6))
                        .padding([4, 13])
                        .width(Fill)
                        .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                        .on_press(UiMessage::CommandBar(commands::Msg::JumpTo(c.id.clone(), m.id.clone()))),
                )
            });
        let tiles = media.iter().map(|m| -> El<'_> {
            let c = st.chat(&m.chat_id);
            let card = |title: String, sub: El<'static>| -> El<'static> {
                let body =
                    column![clamp(&title, 94.0, 3, 10.0, Weight::Normal).line_height(Pixels(12.0)).color(p.text), sub];
                container(body).align_bottom(Fill).width(Fill).padding(8).style(fill_box(p.window, 8.0)).into()
            };
            let thumb = m.attachments.iter().find(|a| matches!(a.attachment_type.as_str(), "img" | "video"));
            let body: El<'_> = match (thumb, m.links.first(), m.attachments.first()) {
                (Some(a), _, _) => match a.poster_img.as_ref().or(a.src_url.as_ref()).and_then(|u| self.media_at(u)) {
                    Some(Ok(frames)) => image(frames[0].0.clone())
                        .width(Fill)
                        .height(Fill)
                        .content_fit(iced::ContentFit::Cover)
                        .border_radius(8.0)
                        .into(),
                    _ => Space::new().into(),
                },
                (None, Some(l), _) => {
                    let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
                    let title = if l.title.is_empty() { host.to_string() } else { l.title.clone() };
                    let sub = row![
                        ic(icon::LINK, 12.0).color(p.text),
                        ell(&l.url, 77.0, 10.0, Weight::Semibold).color(p.text)
                    ];
                    card(title, sub.spacing(4).align_y(Center).into())
                }
                (None, None, Some(a)) => {
                    let kind = file_kind(a.mime_type.as_deref());
                    let name = a.file_name.clone().unwrap_or_else(|| tr("Attachment").into());
                    card(name, semi(kind, 10.0).color(p.text).into())
                }
                (None, None, None) => {
                    let text = m.text.clone().expect("a link message has text");
                    card(text, Space::new().into())
                }
            };
            let face = container(self.chat_avatar(c, 24.0, 0.0)).padding(2).style(bordered_box(
                CLEAR,
                Color::WHITE,
                2.0,
                14.0,
            ));
            let tile = stack![container(body).width(110).height(110).style(fill_box(p.chip, 8.0))]
                .push(container(face).padding(6));
            button(tile)
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
                .on_press(UiMessage::CommandBar(commands::Msg::JumpTo(c.id.clone(), m.id.clone())))
                .into()
        });
        let mut grid = Column::new().spacing(6.5).padding(Padding::new(0.0).left(13.0).right(13.0).bottom(4.0));
        let mut tiles = tiles.peekable();
        while tiles.peek().is_some() {
            grid = grid.push(Row::with_children(tiles.by_ref().take(5)).spacing(6.5));
        }
        let media_grid = (!media.is_empty()).then(|| column![heading(tr("Media")), grid]);
        let empty = none.then(|| {
            let desc = "Only messages loaded in Beeper can be searched.\nFor more results, scroll through older conversations.";
            column![
                container(semi(tr("No messages matching those terms"), 16.0).color(p.strong)).center(Fill),
                txt(desc, 13.0).color(p.strong).align_x(Center).width(Fill),
            ]
            .padding(Padding::new(0.0).left(44.0).right(44.0).bottom(64.0))
        });
        let cat = (!chats.is_empty()).then_some(cat);
        let hits = chats.into_iter().take(limit).enumerate().fold(Column::new().push(cat), |col, (i, c)| {
            let sub = chat_search_subtitle(c);
            let title = row![
                ell(&c.title, 260.0, 13.0, Weight::Semibold).color(p.strong),
                ell(&sub, 220.0, 13.0, Weight::Normal).color(p.sub),
            ]
            .spacing(4)
            .align_y(Center);
            let line = row![
                self.chat_avatar(c, 28.0, 0.0),
                title,
                Space::new().width(Fill),
                svg(brand_svg(brand_key(&c.account_id))).width(16).height(16)
            ]
            .spacing(8)
            .align_y(Center);
            let bg = if i == 0 { p.chip } else { CLEAR };
            col.push(
                container(
                    mouse_area(
                        button(line)
                            .height(36)
                            .padding([0, 6])
                            .width(Fill)
                            .style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0))
                            .on_press(UiMessage::CommandBar(commands::Msg::ExecuteChat(c.id.clone()))),
                    )
                    .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(c.id.clone()))),
                )
                .padding([0, 6]),
            )
        });
        frame(column![inputs].push(hint).push(chips).push(empty).push((!none).then(|| {
            scrollable(hits.push(media_grid).push(messages)).direction(bar()).height(Fill).style(scroll_style(p.scroll))
        })))
    }
}
