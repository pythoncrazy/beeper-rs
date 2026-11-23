//! Desktop's GIF and sticker picker: the GIF search, the sticker packs, and the picked media for the composer.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    Query(String),
    /// Desktop's GIF search 500 ms after the last key, for the query of that time.
    Search(String),
    /// The GIFs of a query.
    /// A page of GIFs for a query: the first page (false) or the next one, and the position of the page after it.
    Loaded(String, bool, Result<(Vec<KlipyGif>, String), String>),
    /// The GIF carousel scrolled: its offset, its content width and its width.
    Scrolled(f32, f32, f32),
    PickGif(KlipyGif),
    Downloaded(Result<PendingMedia, String>),
    /// Desktop's `imagePackStore.loadStickerPacks` of the open chat.
    StickerPacks(Result<Vec<StickerPack>, String>),
    /// A tab of the sticker picker: Recent (`None`) or a pack.
    StickerTab(Option<String>),
    PickSticker(Sticker),
    StickerLoaded(Result<PendingMedia, String>),
}

impl BeeperGuiApp {
    pub(crate) fn gifs(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Query(q) => {
                (self.gif_query, self.gif_loading, self.gif_error) = (q.clone(), true, None);
                self.gif_next.clear();
                if q.trim().is_empty() {
                    return self.update(UiMessage::Gifs(Msg::Search(q)));
                }
                let wait = async { tokio::time::sleep(std::time::Duration::from_millis(500)).await };
                return Task::perform(wait, move |()| UiMessage::Gifs(Msg::Search(q.clone())));
            }
            Msg::Search(q) if q == self.gif_query => {
                let reset = operation::scroll_to("gif-carousel", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                return Task::batch([reset, gif_page(q, String::new())]);
            }
            Msg::Search(_) => {}
            Msg::Loaded(q, ..) if q != self.gif_query => {}
            Msg::Loaded(_, more, r) => {
                self.gif_loading = false;
                match r {
                    // Desktop's `uniqBy(existingGifs.concat(page), "id")`.
                    Ok((page, next)) => {
                        if !more {
                            self.gifs.clear();
                        }
                        for g in page {
                            if self.gifs.iter().all(|x| x.id != g.id) {
                                self.gifs.push(g);
                            }
                        }
                        (self.gif_next, self.gif_error) = (next, None);
                        return self.load_media();
                    }
                    Err(e) => {
                        if !more {
                            self.gifs.clear();
                        }
                        self.gif_error = Some(e);
                    }
                }
            }
            // Desktop loads the next page when the carousel scrolls to 100 px from its end.
            Msg::Scrolled(x, content, width) => {
                self.gif_view = (x, width);
                if x >= content - width - 100.0 && !self.gif_next.is_empty() && !self.gif_loading {
                    self.gif_loading = true;
                    return gif_page(self.gif_query.clone(), self.gif_next.clone());
                }
            }
            Msg::PickGif(g) => {
                self.pane.picker = None;
                return self.download_gif(g);
            }
            Msg::StickerPacks(r) => self.pane.stickers = Some(r),
            Msg::StickerTab(tab) => self.pane.sticker_tab = Some(tab),
            Msg::PickSticker(s) => return self.pick_sticker(s),
            Msg::StickerLoaded(r) => self.sticker_loaded(r),
            Msg::Downloaded(Ok(_)) if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            Msg::Downloaded(r) => match r {
                Ok(gif) => {
                    if !self.store.read().settings.on("autoSendGifs") {
                        self.pane.attachments.push(gif);
                        return Task::none();
                    }
                    // Desktop's `onSendAttachment`: only the GIF goes now. The composer keeps the rest.
                    let pane = &mut self.pane;
                    let text = std::mem::replace(&mut pane.composer, composer_with(""));
                    let files = std::mem::replace(&mut pane.attachments, vec![gif]);
                    let links = std::mem::take(&mut pane.composer_links);
                    let (later, reply) = (pane.send_later.take(), pane.reply_to_message_id.clone());
                    self.submit_composer(false);
                    let pane = &mut self.pane;
                    (pane.composer, pane.attachments, pane.composer_links) = (text, files, links);
                    (pane.send_later, pane.reply_to_message_id) = (later, reply);
                }
                Err(e) => {
                    let text = trf("An error occurred while downloading the GIF: %s", &[&e]);
                    self.fail(Toast::new(text));
                }
            },
        }
        self.settle()
    }
}

/// The width of a GIF of the carousel, 215 px high like Desktop's `tinymp4` videos.
pub(crate) fn gif_width(g: &KlipyGif) -> f32 {
    let (w, h) = g.preview_size;
    (215.0 * w as f32 / h.max(1) as f32).round().max(48.0)
}

/// Loads the page of GIFs at `pos` for the query `q`.
pub(crate) fn gif_page(q: String, pos: String) -> Task<UiMessage> {
    let more = !pos.is_empty();
    Task::perform(beeper_core::cloud::klipy_gifs(q.clone(), pos), move |r| {
        UiMessage::Gifs(gifs::Msg::Loaded(q.clone(), more, r))
    })
}

impl BeeperGuiApp {
    /// Desktop's `GifPicker` keys: Down selects the first GIF, Left and Right move the selection, Up goes back to the
    /// search, Enter sends the selected GIF, and Escape closes the picker and focuses the composer.
    pub(crate) fn gif_key(&mut self, key: &Key) -> Option<Task<UiMessage>> {
        let n = self.gifs.len();
        let select = |app: &mut Self, i: usize| {
            app.gif_sel = Some(i);
            // Desktop's `scrollIntoView` of the selected GIF, to the nearest edge.
            let x = 14.0 + app.gifs[..i].iter().map(|g| gif_width(g) + 4.0).sum::<f32>();
            let (at, width) = app.gif_view;
            let to = match () {
                _ if x < at => x,
                _ if x + gif_width(&app.gifs[i]) > at + width => x + gif_width(&app.gifs[i]) - width,
                _ => return Task::none(),
            };
            operation::scroll_to("gif-carousel", operation::AbsoluteOffset { x: to, y: 0.0 })
        };
        Some(match (key.as_ref(), self.gif_sel) {
            (Key::Named(Named::ArrowUp), _) => {
                self.gif_sel = None;
                Task::batch([operation::focus("gif-search-input"), operation::move_cursor_to_end("gif-search-input")])
            }
            (Key::Named(Named::ArrowDown), None) if n > 0 => select(self, 0),
            (Key::Named(Named::ArrowLeft), Some(i)) => select(self, i.saturating_sub(1).min(n - 1)),
            (Key::Named(Named::ArrowRight), Some(i)) => select(self, (i + 1).min(n - 1)),
            (Key::Named(Named::Enter), Some(i)) if i < n => {
                self.update(UiMessage::Gifs(gifs::Msg::PickGif(self.gifs[i].clone())))
            }
            (Key::Named(Named::Escape), _) => {
                self.pane.picker = None;
                operation::focus("composer-input")
            }
            _ => return None,
        })
    }

    /// Desktop's `downloadGif`: downloads the format of the GIF that the open chat takes. An MP4 is measured like a
    /// picked video.
    pub(crate) fn download_gif(&self, g: KlipyGif) -> Task<UiMessage> {
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let Some((url, size, mp4)) = portal::gif_format(&g, &chat.file) else { return Task::none() };
        let (url, name) = (url.to_string(), format!("{}.{}", g.id, if mp4 { "mp4" } else { "gif" }));
        let load = async move {
            let bytes = beeper_core::cloud::http_bytes(&url).await?;
            let load = move || match mp4 {
                true => {
                    let path = std::env::temp_dir().join(format!("beeper-rs-{}-{name}", std::process::id()));
                    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                    let (f, frames) = (portal::load_file(&path), portal::video_frames(&path, true));
                    std::fs::remove_file(&path).expect("remove the downloaded GIF");
                    Ok(PendingMedia { is_gif: true, size, frames: frames?, ..f? })
                }
                false => {
                    let frames = decode_media(bytes.clone())?;
                    let mime = "image/gif".into();
                    Ok(PendingMedia { bytes, name, mime, is_gif: true, size, frames, duration: None, sticker: None })
                }
            };
            tokio::task::spawn_blocking(load).await.expect("GIF load thread")
        };
        Task::perform(load, |a0| UiMessage::Gifs(gifs::Msg::Downloaded(a0)))
    }

    /// Desktop's `loadStickerPacks` when the sticker picker opens: the packs of the open chat.
    pub(crate) fn load_sticker_packs(&mut self) -> Task<UiMessage> {
        (self.pane.stickers, self.pane.sticker_tab) = (None, None);
        let room = self.pane.selected_chat_id.clone().expect("an open chat");
        Task::perform(beeper_core::matrix::sticker_packs(self.store.clone(), room), |a0| {
            UiMessage::Gifs(gifs::Msg::StickerPacks(a0))
        })
    }

    /// Desktop's `onSelect` of the sticker picker: an animated sticker needs a chat that takes it, and the image
    /// comes as an attachment named by its first shortcode.
    pub(crate) fn pick_sticker(&mut self, s: Sticker) -> Task<UiMessage> {
        use beeper_core::media::{animated, takes_sticker};
        self.pane.picker = None;
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let mime = s.info["mimetype"].as_str().map(str::to_string);
        if mime.as_deref().is_some_and(|m| animated(m) && !takes_sticker(&chat.file, m)) {
            drop(st);
            self.status_banner = tr("This animated sticker can't be sent in this chat.").into();
            return Task::none();
        }
        let fetch = beeper_core::matrix::media(st.matrix.clone(), s.url.clone());
        drop(st);
        let load = async move {
            let bytes = fetch.await?;
            let load = move || {
                let reader = ::image::ImageReader::new(std::io::Cursor::new(&bytes));
                let reader = reader.with_guessed_format().expect("in memory");
                let sniffed = reader.format().map(|f| f.to_mime_type().to_string());
                let dims = reader.into_dimensions().map_err(|e| e.to_string())?;
                let mime = mime.or(sniffed).ok_or("The sticker has no image type")?;
                let ext =
                    mime.split_once('/').map(|(_, t)| t.replace("x-tgsticker", "tgs").replace("lottie+json", "json"));
                let code = Some(s.shortcodes[0].as_str()).filter(|c| !c.is_empty()).unwrap_or("sticker");
                let name = format!("{code}.{}", ext.unwrap_or_else(|| "webp".into()));
                let size = match (s.info["w"].as_u64(), s.info["h"].as_u64()) {
                    (Some(w), Some(h)) => (w as u32, h as u32),
                    _ => dims,
                };
                let frames = decode_media(bytes.clone())?;
                let is_gif = mime == "image/gif";
                Ok(PendingMedia { bytes, name, mime, is_gif, size, frames, duration: None, sticker: Some(Box::new(s)) })
            };
            tokio::task::spawn_blocking(load).await.expect("sticker load thread")
        };
        Task::perform(load, |a0| UiMessage::Gifs(gifs::Msg::StickerLoaded(a0)))
    }

    /// Desktop's `StickerPicker`: the stickers of the selected tab in 4 columns, and the tabs of Recent and of each
    /// pack. Without packs and recent stickers it shows "No sticker packs yet.".
    pub(crate) fn view_sticker_picker<'a>(&'a self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let recent = &st.settings.recent_stickers;
        let packs = match &self.pane().stickers {
            Some(Ok(packs)) => &packs[..],
            _ => &[],
        };
        let failed = matches!(self.pane().stickers, Some(Err(_)));
        let tab = self.pane().sticker_tab.clone().or_else(|| match recent.is_empty() {
            false => Some(None),
            true => packs.first().map(|x| Some(x.id.clone())),
        });
        let shown = match &tab {
            Some(None) => &recent[..],
            Some(Some(id)) => packs.iter().find(|x| &x.id == id).map_or(&[][..], |x| &x.stickers[..]),
            None => &[],
        };
        let picture = |url: &str, w: Length, h: f32| -> El<'a> {
            match self.media_at(url) {
                // Desktop's `StickerImage`, an `EnhancedImg` with `isEmoji`.
                Some(Ok(frames)) => anim::gif(frames, self.animate.get(), self.started, move |f| {
                    image(f).width(w).height(h).content_fit(iced::ContentFit::Contain)
                }),
                _ => Space::new().width(w).height(h).into(),
            }
        };
        // Desktop's grid: 4 columns in 393 px, less the padding of 6 and 8 px and the gaps of 8 px.
        let cell = (393.0 - 12.0 - 16.0 - 24.0) / 4.0;
        let body: El<'a> = match (self.pane().stickers.as_ref().is_some_and(|_| packs.is_empty()) || failed)
            && recent.is_empty()
        {
            true => {
                container(txt(tr("No sticker packs yet."), 13.0).color(p.muted)).padding([16, 12]).center_x(Fill).into()
            }
            false => {
                let rows = shown.chunks(4).map(|row| {
                    let cells = row.iter().map(|x| {
                        button(picture(&x.url, Fill, cell))
                            .padding(0)
                            .width(Fill)
                            .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                            .on_press(UiMessage::Gifs(gifs::Msg::PickSticker(x.clone())))
                            .into()
                    });
                    let rest = (row.len()..4).map(|_| Space::new().width(Fill).into());
                    Row::with_children(cells.chain(rest)).spacing(8).into()
                });
                let n = shown.len().div_ceil(4) as f32;
                let h = (n * cell + (n - 1.0).max(0.0) * 8.0 + 16.0).min(232.0);
                let grid = Column::with_children(rows).spacing(8).padding(8);
                scrollable(grid).height(h).width(Fill).style(scroll_style(p.scroll)).into()
            }
        };
        let tab_btn = |icon: El<'a>, on: bool, msg: UiMessage| -> El<'a> {
            let bg = if on { p.received } else { CLEAR };
            button(container(icon).center(32))
                .padding(2)
                .style(btn_style([bg, p.hover, p.selected], [p.muted; 3], 18.0))
                .on_press(msg)
                .into()
        };
        let recent_tab = (!recent.is_empty()).then(|| {
            tab_btn(tinted(&TIMER, 20.0, p.muted), tab == Some(None), UiMessage::Gifs(gifs::Msg::StickerTab(None)))
        });
        let pack_tabs = packs.iter().map(|x| {
            let avatar = x.avatar.as_deref().unwrap_or(&x.stickers[0].url);
            let on = tab.as_ref().is_some_and(|t| t.as_ref() == Some(&x.id));
            tab_btn(
                picture(avatar, Length::Fixed(32.0), 32.0),
                on,
                UiMessage::Gifs(gifs::Msg::StickerTab(Some(x.id.clone()))),
            )
        });
        let tabs = Row::new().push(recent_tab).extend(pack_tabs).spacing(4).padding([4, 6]);
        let rule = container(Space::new().width(Fill).height(1)).style(fill_box(p.border_weak, 0.0));
        let card = column![body, rule, tabs].width(393).padding(6);
        container(container(card).style(shadow_box(p.menu, p.menu_border, 8.0, 16.0))).padding([0, 8]).into()
    }
}
