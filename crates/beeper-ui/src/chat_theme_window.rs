//! Desktop's chat theme window: the wallpapers and bubble colors to pick from, and the colors they give the timeline.

use super::*;
use iced::widget::column;

impl BeeperGuiApp {
    /// Desktop's `--on-wallpaper-fg` of `chat` outside of a reply chain and of a thread pane, when its chat theme has
    /// a wallpaper.
    pub(crate) fn wallpaper_fg(&self, st: &BeeperStore, chat: &Chat, thread: bool) -> Option<Color> {
        let wallpaper = chat_theme::resolve(&st.settings.chat_appearance, Some(&chat.id), self.dark, &st.config.data_dir).0;
        wallpaper.filter(|_| !thread && self.pane().thread_root.is_none()).map(|(_, fg)| fg)
    }

    /// The color, text color and metadata color of sent bubbles in `chat`, or in the default chat theme.
    pub(crate) fn sent_colors(&self, st: &BeeperStore, chat: Option<&str>) -> (Color, Color, Color) {
        let p = &self.pal;
        match chat_theme::resolve(&st.settings.chat_appearance, chat, self.dark, &st.config.data_dir).1 {
            Some((bg, fg)) => (bg, fg, fg.scale_alpha(p.sent_meta.a)),
            None => (p.sent, p.sent_fg, p.sent_meta),
        }
    }

    /// Desktop's `removeOrphanedWallpapers`: deletes the photos in `chat_wallpapers` that no chat theme uses.
    pub(crate) fn prune_wallpapers(&self) {
        let st = self.store.read();
        let entries = match std::fs::read_dir(chat_theme::photo_dir(&st.config.data_dir)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => panic!("read the chat wallpapers: {e}"),
        };
        let used: HashSet<&str> = st.settings.chat_appearance.photos().collect();
        for entry in entries {
            let path = entry.expect("a chat wallpaper entry").path();
            if path.file_name().and_then(|n| n.to_str()).is_none_or(|n| !used.contains(n)) {
                std::fs::remove_file(&path).expect("remove an unused chat wallpaper");
            }
        }
    }

    pub(crate) fn view_chat_theme(&self, chat: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let title = container(semi(tr("Chat Theme"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let card = scrollable(self.chat_theme_picker(&st, Some(chat))).direction(bar()).style(scroll_style(p.scroll));
        let card = container(card).height(Fill).style(fill_box(p.card, 12.0));
        container(column![title, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's `ChatAppearancePicker`: the chat theme of `chat`, or the default chat theme.
    pub(crate) fn chat_theme_picker(&self, st: &BeeperStore, chat: Option<&str>) -> El<'_> {
        let p = &self.pal;
        let a = &st.settings.chat_appearance;
        let stored = a.theme(chat);
        let wallpaper = chat_theme::resolve(a, chat, self.dark, &st.config.data_dir).0;
        let (sent, sent_fg, _) = self.sent_colors(st, chat);
        let id = chat.map(str::to_string);
        let ring = |el: El<'static>, on: bool, radius: f32, msg: UiMessage| -> El<'static> {
            let edge = if on { p.accent } else { CLEAR };
            let el = container(el).padding(4).style(bordered_box(CLEAR, edge, 2.0, radius + 4.0));
            mouse_area(el).on_press(msg).into()
        };
        let bubble = |text: &'static str, bg: Color, fg: Color| {
            // `ChatAppearancePicker`'s bubbles are 13 px, its labels and caption 12 px, and its tiles 11 px.
            container(crate::style::fixed(tr(text), 13.0, Weight::Normal).color(fg)).padding([8, 12]).style(fill_box(bg, 17.0))
        };
        let lines = column![
            bubble("Pick a wallpaper and bubble color", p.received, p.strong),
            container(bubble("Only you see your chat theme", sent, sent_fg)).align_right(Fill),
        ]
        .spacing(8);
        let preview: El<'_> = match wallpaper.clone() {
            Some((paint, _)) => stack![chat_theme::fill(paint, Size::new(f32::INFINITY, 106.0), 12.0), container(lines).padding(16)].into(),
            None => container(lines).padding(16).style(fill_box(p.card, 12.0)).into(),
        };
        let preview = container(preview).height(106 + 2).padding(1).style(bordered_box(CLEAR, p.border_neutrals, 1.0, 12.0));
        let label = |text: &'static str| crate::style::fixed(tr(text), 12.0, Weight::Medium).color(p.sub);
        let default = matches!(stored.bubble, None | Some(Bubble::Default));
        let none = if chat.is_some() { Some(Bubble::Default) } else { None };
        let sub = p.sub;
        let cross = svg(chat_theme::NO_BUBBLE.clone()).width(26).height(26).style(move |_, _| svg::Style { color: Some(sub) });
        let mut swatches = vec![ring(
            container(cross).padding(1).style(bordered_box(CLEAR, p.border_neutrals, 1.0, 14.0)).into(),
            default,
            14.0,
            UiMessage::ChatTheme(chat_theme::Msg::Bubble(id.clone(), none)),
        )];
        swatches.extend(chat_theme::BUBBLES.iter().map(|&(bid, c)| {
            let on = stored.bubble == Some(Bubble::Swatch { id: bid.into() });
            let dot = container(Space::new()).width(28).height(28).style(fill_box(hex(c), 14.0));
            ring(dot.into(), on, 14.0, UiMessage::ChatTheme(chat_theme::Msg::Bubble(id.clone(), Some(Bubble::Swatch { id: bid.into() }))))
        }));
        let tile = |fill: El<'static>, w: Wallpaper| {
            let on = stored.wallpaper.as_ref().unwrap_or(&Wallpaper::None) == &w;
            let fill = container(fill).width(Fill).height(100).padding(1).style(bordered_box(CLEAR, p.border_neutrals, 1.0, 10.0));
            ring(fill.into(), on, 10.0, UiMessage::ChatTheme(chat_theme::Msg::Wallpaper(id.clone(), w)))
        };
        let blank = |label: &'static str| -> El<'static> {
            container(crate::style::fixed(tr(label), 11.0, Weight::Normal).color(p.sub)).center(Fill).style(fill_box(p.card, 10.0)).into()
        };
        let photo = matches!(stored.wallpaper, Some(Wallpaper::Photo { .. }));
        let photo_fill = match wallpaper.filter(|_| photo) {
            Some((paint, _)) => chat_theme::fill(paint, Size::new(60.0, 98.0), 9.0),
            None => blank("Photo…"),
        };
        let mut tiles = vec![tile(blank("None"), Wallpaper::None)];
        let photo_fill = container(photo_fill).width(Fill).height(100).padding(1);
        let photo_fill = photo_fill.style(bordered_box(CLEAR, p.border_neutrals, 1.0, 10.0));
        tiles.push(ring(photo_fill.into(), photo, 10.0, UiMessage::ChatTheme(chat_theme::Msg::PickWallpaper(id.clone()))));
        let papers = chat_theme::COLORS.iter().map(|c| Wallpaper::Color { id: c.0.into() });
        for w in papers.chain(chat_theme::GRADIENTS.iter().map(|g| Wallpaper::Gradient { id: g.0.into() })) {
            let (paint, _) = chat_theme::paint(&w, self.dark, &st.config.data_dir).expect("a known wallpaper");
            tiles.push(tile(chat_theme::fill(paint, Size::new(60.0, 98.0), 9.0), w));
        }
        let footer = chat.map(|c| {
            let reset = a
                .per_chat
                .contains_key(c)
                .then(|| pill_btn(p, tr("Use app default"), UiMessage::ChatTheme(chat_theme::Msg::Clear(c.into()))));
            row![crate::style::fixed(tr("Only you see your chat themes."), 12.0, Weight::Normal).color(p.sub), Space::new().width(Fill)]
                .push(reset)
                .spacing(8)
                .align_y(Center)
        });
        column![
            preview,
            label("Bubble color"),
            Row::with_children(swatches).wrap(),
            label("Wallpaper"),
            Grid::with_children(tiles).fluid(74).spacing(2).height(grid::Sizing::EvenlyDistribute(Shrink)),
        ]
        .push(footer)
        .spacing(12)
        .padding(16)
        .into()
    }
}
