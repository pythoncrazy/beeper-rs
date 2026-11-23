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
        let menu =
            |items: Vec<El<'static>>| -> El<'static> { crate::menu_card(p, Column::with_children(items), 233).into() };
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
        let inputs = self.palette_inputs();
        let q = self.command_palette_query.trim().to_lowercase();
        if self.cmd_bar_commands_mode {
            return self.view_palette_commands(&st, inputs);
        }
        let hint = self.palette_hint(&st);
        let chips = self.palette_chips();
        let f = &self.search_filters;
        let raw = self.command_palette_query.trim();
        let blank = room.is_none() && q.is_empty() && *f == SearchFilters::default();
        let (recent, archived) = self.palette_recent(&st, blank);
        let hits = self.search_hits(&st, raw);
        let chats: Vec<&Chat> = hits.chats.iter().map(|&i| &st.chats[i]).collect();
        let in_groups: Vec<&Chat> = hits.in_groups.iter().map(|&i| &st.chats[i]).collect();
        let (bookmarks, media, found) = (&hits.bookmarks, &hits.media, &hits.messages);
        let any = [&recent, &archived, &chats, &in_groups].iter().any(|l| !l.is_empty())
            || !(bookmarks.is_empty() && media.is_empty() && found.is_empty());
        let none = !any && (raw.chars().count() > 1 || !f.media.is_empty());
        let needle = q.to_ascii_lowercase();
        let message = |m: &Message| self.palette_message(&st, m, &needle);
        let media_grid = (!media.is_empty()).then(|| self.palette_media(&st, media));
        let empty = none.then(|| self.palette_empty());
        let partial = if self.search_partial { tr("Hide partial matches") } else { tr("Show partial matches") };
        let partial = Some((partial, commands::Msg::TogglePartialMatches));
        // Desktop's `Jqt`: the Messages heading stays without messages while partial matches are hidden.
        let messages = (!found.is_empty() || (!raw.is_empty() && any && !self.search_partial))
            .then(|| column![self.palette_heading("Messages", partial)].extend(found.iter().map(&message)));
        let (marked, more_marked) = self.palette_rows("Bookmarks", bookmarks.len());
        let bookmarks = (!bookmarks.is_empty()).then(|| {
            let rows = bookmarks.iter().take(marked).map(&message);
            column![self.palette_heading("Bookmarks", self.palette_more("Bookmarks", more_marked))].extend(rows)
        });
        // Desktop's `Jqt` order of the chat sections, before the bookmarks, the media and the messages.
        let sections =
            [("Recent chats", &recent), ("Last archived", &archived), ("Chats", &chats), ("In Groups", &in_groups)];
        let lead = sections.iter().find(|s| !s.1.is_empty()).map(|s| s.0);
        let results = sections.into_iter().filter(|s| !s.1.is_empty()).fold(Column::new(), |col, (label, list)| {
            let (n, has_more) = match label {
                "Last archived" => (1, false),
                _ => self.palette_rows(label, list.len()),
            };
            let col = col.push(self.palette_heading(label, self.palette_more(label, has_more)));
            // Desktop highlights the first result.
            col.extend(
                list.iter().take(n).enumerate().map(|(i, c)| self.palette_chat(c, lead == Some(label) && i == 0)),
            )
        });
        let results = results.push(bookmarks).push(media_grid).push(messages);
        self.palette_frame(
            column![inputs]
                .push(hint)
                .push(chips)
                .push(empty)
                .push((!none).then(|| scrollable(results).direction(bar()).height(Fill).style(scroll_style(p.scroll)))),
        )
    }

    /// Desktop's `results` without a query or filters (`blank`): the recent chats but the open one, and the chat
    /// archived last when it is not one of the recent chats that show.
    fn palette_recent<'s>(&self, st: &'s BeeperStore, blank: bool) -> (Vec<&'s Chat>, Vec<&'s Chat>) {
        let selected = self.pane().selected_chat_id.as_deref();
        let recent: Vec<&Chat> = match (blank, st.recent_chat_ids.is_empty()) {
            (false, _) => Vec::new(),
            (true, false) => st.recent_chat_ids.iter().filter_map(|id| st.find_chat(id)).collect(),
            (true, true) => self.chats_by_activity(st).iter().map(|&i| &st.chats[i]).collect(),
        };
        let mut recent: Vec<&Chat> = recent.into_iter().filter(|c| c.flags & CHAT_HIDDEN == 0).collect();
        if recent.len() > 1 {
            recent.retain(|c| selected != Some(c.id.as_str()));
        }
        let last = self.last_archived.borrow().clone().filter(|_| blank);
        let archived: Vec<&Chat> = last
            .and_then(|id| st.find_chat(&id))
            .filter(|c| {
                let recents = recent.iter().take(self.palette_rows("Recent chats", recent.len()).0);
                c.flags & CHAT_HIDDEN == 0
                    && selected != Some(c.id.as_str())
                    && !recents.into_iter().any(|r| r.id == c.id)
            })
            .into_iter()
            .collect();
        (recent, archived)
    }

    /// Desktop's `X5` with the `J5` limits: the rows that a section of the results shows, and whether it has more.
    fn palette_rows(&self, s: &str, n: usize) -> (usize, bool) {
        let (collapsed, open) = if s == "Bookmarks" { (3, 100) } else { (5, 12) };
        (if self.search_expanded.contains(s) { open } else { collapsed }, n > collapsed)
    }

    /// The Show more or Show less action of a section of the results that has `more` rows.
    fn palette_more(&self, s: &'static str, more: bool) -> Option<(&'static str, commands::Msg)> {
        let label = if self.search_expanded.contains(s) { tr("Show less") } else { tr("Show more") };
        more.then_some((label, commands::Msg::ToggleSearchExpanded(s)))
    }

    /// The card of the command palette.
    fn palette_frame<'a>(&self, body: Column<'a, UiMessage>) -> El<'a> {
        let p = &self.pal;
        container(body)
            .width(602)
            .height(360)
            .style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 4.0, 84.0))
            .into()
    }

    /// The input row of the command palette: the query, with the search icon and the Commands or Chat button, or
    /// with the If no reply select of Remind Me and Send Later.
    fn palette_inputs(&self) -> El<'_> {
        let p = &self.pal;
        let placeholder = match (self.cmd_bar_commands_mode, self.search_room.as_deref()) {
            _ if self.cmd_submenu.is_some() => self.cmd_submenu.as_ref().map(|m| m.placeholder()).unwrap_or_default(),
            (true, _) => tr("Search commands").into(),
            (false, Some(_)) => tr("Search this chat").into(),
            (false, None) => tr("Search chats").into(),
        };
        let input = text_input(&placeholder, &self.command_palette_query)
            .id("cmd-palette-input")
            .on_input(|a0| UiMessage::CommandBar(commands::Msg::Query(a0)))
            .size(rem(13.0))
            .padding([6, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let kbd = |label: &'static str| {
            container(semi(label, 13.0).color(p.sub))
                .center_x(Shrink)
                .center_y(20)
                .padding([0, 2])
                .style(fill_box(Color { a: 0.1, ..p.strong }, 4.0))
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
        inputs.spacing(8).align_y(Center).padding(Padding::new(6.0).left(13.0).right(8.0)).into()
    }

    /// The commands mode of the command palette: the heading of a submenu, the commands, and the note of Snooze,
    /// Remind Me and Send Later.
    fn view_palette_commands<'a>(&'a self, st: &BeeperStore, inputs: El<'a>) -> El<'a> {
        let p = &self.pal;
        let schedule = matches!(self.cmd_submenu, Some(commands::SubMenu::Schedule(_)));
        let cmds = self.commands(st, self.command_palette_query.trim());
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
        let snooze = matches!(self.cmd_submenu, Some(commands::SubMenu::Snooze(_)));
        let noted = schedule || snooze || matches!(self.cmd_submenu, Some(commands::SubMenu::Remind(_)));
        let footer = noted.then(|| {
            let note = match (schedule, self.cmd_no_reply) {
                _ if snooze => tr("Snoozed chats leave your inbox and stay quiet until the time you pick").into(),
                (true, _) => trf(
                    "Send Later messages can only send if %s app is running (and your computer is awake)",
                    &[&"Beeper"],
                ),
                (_, true) => tr("Remind Me marks this chat as new at the scheduled time, if there is no reply.").into(),
                (_, false) => tr("Remind Me marks this chat as new at the scheduled time").into(),
            };
            let line = container(Space::new()).width(Fill).height(2).style(fill_box(p.strong.scale_alpha(0.1), 0.0));
            let note = container(txt(note, 13.0).color(p.sub)).center_y(44).padding([8, 13]);
            column![line, container(note).width(Fill).style(fill_box(p.chip, iced::border::bottom(18)))]
        });
        let body = column![].push(header).push(inputs).push(self.view_commands(st, &cmds));
        self.palette_frame(body.push(footer))
    }

    /// The hint of the command palette over an open chat: the hotkey that searches the chat, or all the chats.
    fn palette_hint(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        let c =
            self.pane().selected_chat_id.as_deref().and_then(|cid| st.resolve_chat_idx(cid).map(|i| &st.chats[i]))?;
        let key = |name| prefs::hotkey_label(st.settings.hotkey_keys(name).split(", ").next().unwrap_or(""));
        let label = match (self.search_room.as_deref(), c.chat_type == "group") {
            (Some(_), _) => trf("Press %s to search all of your conversations", &[&key("Search")]),
            (None, true) => trf("Press %1$s to search your conversation in %2$s", &[&key("Search: Room"), &c.title]),
            (None, false) => trf("Press %1$s to search your conversation with %2$s", &[&key("Search: Room"), &c.title]),
        };
        let hint = row![tinted(&DROP_ARROW, 20.0, p.sub), txt(label, 13.0).color(p.text)];
        Some(hint.spacing(16).align_y(Center).padding([8, 24]).into())
    }

    /// The filter chips of the search, of which a search in a chat disables the first.
    fn palette_chips(&self) -> El<'_> {
        let p = &self.pal;
        let f = &self.search_filters;
        let room = self.search_room.as_deref();
        Row::with_children(SEARCH_CHIPS.iter().enumerate().map(|(i, &placeholder)| {
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
        .padding(Padding::new(6.0).left(13.0).right(13.0))
        .into()
    }

    /// The heading of a section of the search results, with its action on the right.
    fn palette_heading(
        &self,
        label: &'static str,
        action: Option<(&'static str, commands::Msg)>,
    ) -> iced::widget::Row<'_, UiMessage> {
        let p = &self.pal;
        let title = match label {
            "Chats" | "Messages" | "Media" => trx(label, "command bar category"),
            _ => tr(label),
        };
        row![txt(title, 12.0).color(p.sub), Space::new().width(Fill)]
            .push(action.map(|(text, msg)| {
                button(semi(text, 11.0).color(p.accent))
                    .padding(0)
                    .style(ghost(p, 4.0))
                    .on_press(UiMessage::CommandBar(msg))
            }))
            .align_y(Center)
            .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
    }

    /// Desktop's `SelectMessageCommand`: a found message in its bubble, with `needle` marked, under its chat and time.
    fn palette_message(&self, st: &BeeperStore, m: &Message, needle: &str) -> El<'_> {
        let p = &self.pal;
        let c = st.chat(&m.chat_id);
        let mine = m.is_sender == Some(true);
        let label = match (c.chat_type == "group", mine) {
            (false, _) => c.title.clone(),
            (true, true) => format!("{} • {}", c.title, chat_network(c)),
            (true, false) => {
                format!("{} in {} • {}", sender_label(c, m), c.title, chat_network(c))
            }
        };
        let h24 = st.settings.pref("timeFormat") == 1;
        let head = row![
            txt(label, 11.0).line_height(Pixels(rem(7.0))).color(p.text),
            Space::new().width(Fill),
            txt(list_ts(&m.timestamp, h24), 11.0).line_height(Pixels(rem(7.0))).color(p.text)
        ];
        let (bg, fg) = if mine { (p.sent, p.sent_fg) } else { (p.received, p.strong) };
        let body = clamped(m.text.as_deref().expect("text message"), 554.0, 3, 13.0, Weight::Normal);
        let lower = body.to_ascii_lowercase();
        let (mut spans, mut at): (Vec<iced::widget::text::Span<()>>, _) = (Vec::new(), 0);
        for (k, hit) in lower.match_indices(needle) {
            spans.push(span(body[at..k].to_string()).size(rem(13.0)).color(fg));
            let mark = span(body[k..k + hit.len()].to_string()).size(rem(13.0));
            spans.push(mark.color(hex(0x202124)).background(hex(0xf6ce46)));
            at = k + hit.len();
        }
        spans.push(span(body[at..].to_string()).size(rem(13.0)).color(fg));
        let bubble = rich_text(spans)
            .line_height(iced::widget::text::LineHeight::Absolute(rem(16.0).into()))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        let bubble = container(container(bubble).padding([8, 11]).style(fill_box(bg, 17.0)))
            .width(Fill)
            .align_x(if mine { Right } else { iced::Left });
        button(column![head, bubble].spacing(6))
            .padding([4, 13])
            .width(Fill)
            .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
            .on_press(UiMessage::CommandBar(commands::Msg::JumpTo(c.id.clone(), m.id.clone())))
            .into()
    }

    /// The Media section of the search results: rows of five tiles with the image, link or file of each message.
    fn palette_media(&self, st: &BeeperStore, media: &[Message]) -> Column<'_, UiMessage> {
        let mut grid = Column::new().spacing(6.5).padding(Padding::new(0.0).left(13.0).right(13.0).bottom(4.0));
        let mut tiles = media.iter().map(|m| self.palette_media_tile(st, m)).peekable();
        while tiles.peek().is_some() {
            grid = grid.push(Row::with_children(tiles.by_ref().take(5)).spacing(6.5));
        }
        column![self.palette_heading("Media", None), grid]
    }

    /// A tile of the Media section, with the avatar of its chat, which jumps to its message.
    fn palette_media_tile(&self, st: &BeeperStore, m: &Message) -> El<'_> {
        let p = &self.pal;
        let c = st.chat(&m.chat_id);
        let card = |title: String, sub: El<'static>| -> El<'static> {
            let body =
                column![clamp(&title, 94.0, 3, 10.0, Weight::Normal).line_height(Pixels(rem(12.0))).color(p.text), sub];
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
                let title = crate::links::shown_title(l);
                let sub =
                    row![ic(icon::LINK, 12.0).color(p.text), ell(&l.url, 77.0, 10.0, Weight::Semibold).color(p.text)];
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
        let face =
            container(self.chat_avatar(c, 24.0, 0.0)).padding(2).style(bordered_box(CLEAR, Color::WHITE, 2.0, 14.0));
        let tile = stack![container(body).width(110).height(110).style(fill_box(p.chip, 8.0))]
            .push(container(face).padding(6));
        button(tile)
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
            .on_press(UiMessage::CommandBar(commands::Msg::JumpTo(c.id.clone(), m.id.clone())))
            .into()
    }

    /// Desktop's `Eqt`: no message matches the search, with "Show partial matches" while they are hidden.
    fn palette_empty(&self) -> Column<'_, UiMessage> {
        let p = &self.pal;
        let desc = format!(
            "{}\n{}",
            tr("Only messages loaded in Beeper can be searched."),
            tr("For more results, scroll through older conversations.")
        );
        let show = (!self.search_partial).then(|| {
            button(semi(tr("Show partial matches"), 13.0).color(p.accent))
                .padding(0)
                .style(ghost(p, 4.0))
                .on_press(UiMessage::CommandBar(commands::Msg::TogglePartialMatches))
        });
        column![
            container(semi(tr("No messages matching those terms"), 16.0).color(p.strong)).center(Fill),
            txt(desc, 13.0).color(p.strong).align_x(Center).width(Fill),
        ]
        .push(show.map(|b| container(b).center_x(Fill)))
        .padding(Padding::new(0.0).left(44.0).right(44.0).bottom(64.0))
    }

    /// A chat of the search results, which opens on a click and has its menu on a right click; Desktop highlights
    /// the first result (`first`).
    fn palette_chat(&self, c: &Chat, first: bool) -> El<'_> {
        let p = &self.pal;
        let title = crate::chat_search_title(p, c);
        let line = row![
            self.chat_avatar(c, 28.0, 0.0),
            title,
            Space::new().width(Fill),
            svg(brand_svg(brand_key(&c.account_id))).width(16).height(16)
        ]
        .spacing(8)
        .align_y(Center);
        let bg = if first { p.chip } else { CLEAR };
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
        .padding([0, 6])
        .into()
    }
}
