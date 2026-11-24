//! Desktop's `MergedChatEditor` (`Hft`) in its `CreateMergedChatBrowserWindow` (`D3`): the intro, the chats to merge,
//! and the name, photo and default chat of a new or an edited merged chat.

use super::*;
use crate::dialogs::LABEL_INTRO_FACES;
use crate::merged::chat_brand;
use crate::prelude::*;
use iced::widget::{column, operation};

/// Desktop's `windowFeatures` of the merged chat editor.
pub(crate) const MERGE_SIZE: Size = Size::new(340.0, 570.0);
/// Desktop's `Q`: the most chats of a section of the chat list.
const SECTION: usize = 8;
/// Desktop's `lte`, the "Learn more" page of merged chats.
pub(crate) const MERGE_HELP: &str = "https://help.beeper.com/en_US/quick-references/how-to-create-and-manage-merged-chats";

/// A step of the editor (`f`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MergeStep {
    Intro,
    Pick,
    Details,
}

/// Desktop's photo choice of a merged chat: the photo it has, its initials, or the photo of a member chat.
#[derive(Debug, Clone, PartialEq)]
pub enum Photo {
    Keep,
    Auto,
    Member(String),
}

pub struct MergeDraft {
    pub id: window::Id,
    /// The edited merged chat, or none for a new one.
    pub union: Option<String>,
    pub step: MergeStep,
    pub search: String,
    /// The chats to merge, in the order they were picked.
    pub chats: Vec<String>,
    pub title: String,
    /// The user typed the name (`I`); until then it follows the first chat.
    pub titled: bool,
    pub photo: Photo,
    pub default: Option<String>,
    /// The menu of the "Default chat" select is open.
    pub default_menu: bool,
    pub saving: bool,
}

#[derive(Debug, Clone)]
pub enum MergeEdit {
    Start,
    Search(String),
    Toggle(String),
    Continue,
    Back,
    Title(String),
    Photo(Photo),
    DefaultMenu(bool),
    Default(String),
    Submit,
    Upgrade,
}

/// Desktop's `uS` and `isMergeable`: a direct chat that is not hidden, read-only or merged already.
pub(crate) fn mergeable(c: &Chat) -> bool {
    c.chat_type == "single" && c.merge.is_none() && c.merged_into_chat_id.is_none() && !c.is_read_only
}

/// The other person of a direct chat (`Ui`).
pub(crate) fn other(c: &Chat) -> Option<&User> {
    c.participants.items.iter().find(|u| u.is_self != Some(true))
}

/// Desktop's `mi`: the phone number, email and full name of two words or more of the other person, which find the
/// chats with the same person.
pub(crate) fn merge_keys(c: &Chat) -> Vec<String> {
    let Some(u) = other(c) else { return Vec::new() };
    let phone = u.phone_number.as_deref().map(|n| n.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect());
    let email = u.email.as_deref().map(|e| e.trim().to_lowercase());
    let name = u.full_name.as_deref().map(|n| n.split_whitespace().collect::<Vec<_>>()).filter(|w| w.len() >= 2);
    let name = name.map(|w| w.join(" ").to_lowercase());
    let keys = [phone.map(|p: String| format!("tel:{p}")), email.map(|e| format!("mailto:{e}"))];
    keys.into_iter().chain([name.map(|n| format!("name:{n}"))]).flatten().collect()
}

/// Desktop's `Tde`: the username, phone number, email or user ID of the other person of a direct chat.
fn identifier(c: &Chat) -> Option<String> {
    let u = other(c).filter(|_| c.chat_type == "single")?;
    handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone()).or(Some(u.id.clone()))
}

/// Desktop's `oj`: the photo of a chat, else of its other person, as an `mxc://` URL that a merged chat can take.
fn member_photo(c: &Chat) -> Option<&str> {
    c.img_url.as_deref().or_else(|| other(c)?.img_url.as_deref()).filter(|u| u.starts_with("mxc://"))
}

impl MergeDraft {
    fn ready(&self) -> bool {
        self.chats.len() >= 2
    }
}

impl BeeperGuiApp {
    /// Desktop's `D3.open`: the editor of the merged chat `union`, or of a new one of `chats`. A new merged chat starts
    /// with the intro the first time, or when the plan allows no more.
    pub(crate) fn open_merge(&mut self, union: Option<String>, chats: Vec<String>) -> Task<UiMessage> {
        self.list.chat_menu = None;
        self.show_command_palette = false;
        if let Some(d) = &self.merge_window {
            return self.raise(d.id);
        }
        let st = self.store.read();
        let intro = union.is_none() && (!st.settings.merge_intro_seen || !st.can_create_union());
        let edited = union.as_deref().and_then(|u| st.resolve_chat_idx(u));
        let (chats, title, photo, default) = match edited {
            Some(i) => {
                let c = &st.chats[i];
                let rooms = c.merge.as_ref().map(|m| m.chat_ids.clone()).unwrap_or_default();
                let photo = if c.img_url.is_some() { Photo::Keep } else { Photo::Auto };
                (rooms, c.title.clone(), photo, st.default_member(i).map(|m| st.chats[m].id.clone()))
            }
            None => {
                let title = chats.first().and_then(|c| st.find_chat(c)).map(|c| c.title.clone()).unwrap_or_default();
                (chats, title, Photo::Auto, None)
            }
        };
        drop(st);
        if intro {
            self.patch_settings(|s| s.merge_intro_seen = true);
        }
        let (id, open) = self.open_resizable_child(MERGE_SIZE);
        let step = if intro { MergeStep::Intro } else { MergeStep::Pick };
        let titled = union.is_some();
        self.merge_window = Some(MergeDraft {
            id,
            union,
            step,
            search: String::new(),
            chats,
            title,
            titled,
            photo,
            default,
            default_menu: false,
            saving: false,
        });
        if intro {
            open
        } else {
            open.chain(operation::focus("merge-search"))
        }
    }

    pub(crate) fn merge_edit(&mut self, edit: MergeEdit) -> Task<UiMessage> {
        let can = self.store.read().can_create_union();
        let st = self.store.read();
        let d = self.merge_window.as_mut().expect("the merged chat editor is open");
        let title_of = |id: &str| st.find_chat(id).map(|c| c.title.clone()).unwrap_or_default();
        match edit {
            MergeEdit::Start => {
                d.step = MergeStep::Pick;
                return operation::focus("merge-search");
            }
            MergeEdit::Search(q) => d.search = q,
            MergeEdit::Toggle(chat) => {
                match d.chats.iter().position(|c| *c == chat) {
                    Some(i) => drop(d.chats.remove(i)),
                    None => d.chats.push(chat),
                }
                if !d.titled {
                    if let Some(first) = d.chats.first() {
                        d.title = title_of(first);
                    }
                }
            }
            MergeEdit::Continue if d.ready() => {
                if d.default.as_ref().is_none_or(|c| !d.chats.contains(c)) {
                    d.default = d.chats.first().cloned();
                }
                if d.photo == Photo::Auto {
                    let photo = d.chats.iter().find(|c| st.find_chat(c).and_then(member_photo).is_some());
                    d.photo = photo.map_or(Photo::Auto, |c| Photo::Member(c.clone()));
                }
                d.step = MergeStep::Details;
                return operation::focus("merge-title");
            }
            MergeEdit::Continue => {}
            MergeEdit::Back => (d.step, d.default_menu) = (MergeStep::Pick, false),
            MergeEdit::Title(t) => (d.title, d.titled) = (t, true),
            MergeEdit::Photo(p) => d.photo = p,
            MergeEdit::DefaultMenu(open) => d.default_menu = open,
            MergeEdit::Default(c) => (d.default, d.default_menu) = (Some(c), false),
            MergeEdit::Upgrade => {
                drop(st);
                return self.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus)));
            }
            MergeEdit::Submit if !d.ready() || d.title.trim().is_empty() || d.saving => {}
            MergeEdit::Submit if d.union.is_none() && !can => {
                drop(st);
                return self.update(UiMessage::Merged(merged::Msg::Edit(MergeEdit::Upgrade)));
            }
            MergeEdit::Submit => {
                drop(st);
                return self.submit_merge();
            }
        }
        Task::none()
    }

    /// Desktop's `Mt`: creates the merged chat and opens it with a toast, or saves the edited one.
    fn submit_merge(&mut self) -> Task<UiMessage> {
        let d = self.merge_window.as_mut().expect("the merged chat editor is open");
        d.saving = true;
        let (window, title, chats, default) = (d.id, d.title.trim().to_string(), d.chats.clone(), d.default.clone());
        let (union, photo) = (d.union.clone(), d.photo.clone());
        let mut st = self.store.write();
        let photo_url = |st: &BeeperStore| match &photo {
            Photo::Member(c) => st.find_chat(c).and_then(member_photo).map(str::to_string),
            _ => None,
        };
        let url = photo_url(&st);
        let Some(union) = union else {
            let n = chats.len();
            let id = st.create_union(&title, chats, default, url);
            drop(st);
            self.select_chat(&id);
            let text = trf("You can now message %1$s across %2$d chats from one conversation.", &[&title, &n]);
            self.show(Toast { strong: true, ..Toast::new(text) });
            return self.close_window(window);
        };
        let had_photo = st.find_chat(&union).is_some_and(|c| c.img_url.is_some());
        st.update_union(&union, &title, chats, default);
        // Desktop's `setUnionAvatarMxc` and `clearUnionAvatar`.
        let content = match photo {
            Photo::Keep => None,
            Photo::Auto => had_photo.then(|| serde_json::json!({})),
            Photo::Member(_) => url.as_ref().map(|u| serde_json::json!({ "url": u })),
        };
        if let Some(content) = content {
            let idx = st.chat_idx(&st.resolve_canonical_chat_id(&union).expect("an edited merged chat"));
            st.update_chat(idx, |c| c.img_url.clone_from(&url));
            let id = st.chats[idx].id.clone();
            st.queue(&id, beeper_core::matrix::Outgoing::State("m.room.avatar".into(), content));
        }
        self.close_window(window)
    }

    /// Desktop's `qe`: the title bar of the editor.
    fn merge_title(d: &MergeDraft) -> String {
        match d.step {
            MergeStep::Intro => tr("Merge Chats").into(),
            MergeStep::Details => tr("Merge Details").into(),
            MergeStep::Pick if d.union.is_some() => tr("Merged Chats").into(),
            MergeStep::Pick => trn("Merge %d Chat", "Merge %d Chats", d.chats.len(), &[&d.chats.len()]),
        }
    }

    pub(crate) fn view_merge_editor<'a>(&'a self, d: &'a MergeDraft) -> El<'a> {
        let p = &self.pal;
        let edit = |e| UiMessage::Merged(merged::Msg::Edit(e));
        let body = match d.step {
            MergeStep::Intro => self.view_merge_intro(d),
            MergeStep::Pick => self.view_merge_pick(d),
            MergeStep::Details => self.view_merge_details(d),
        };
        // Desktop's `vt` buttons: 32 px rounded pills.
        let pill = |label: String, primary: bool, msg: Option<UiMessage>| {
            let (bg, fg) = match (primary, msg.is_some()) {
                (true, true) => (p.accent, Color::WHITE),
                (true, false) => (p.accent.scale_alpha(0.4), Color::WHITE.scale_alpha(0.6)),
                (false, _) => (p.chip, p.strong),
            };
            button(container(med(label, 13.0)).center_x(Fill).center_y(32))
                .padding([0, 14])
                .style(btn_style([bg; 3], [fg; 3], 16.0))
                .on_press_maybe(msg)
        };
        let footer: Option<El> = match d.step {
            MergeStep::Intro => None,
            MergeStep::Pick => {
                let next = d.ready().then(|| edit(MergeEdit::Continue));
                Some(pill(tr("Continue").into(), true, next).width(Fill).into())
            }
            MergeStep::Details => {
                let n = d.chats.len();
                let label = match d.union.is_some() {
                    true => trx("Confirm", "merged chat wizard").into(),
                    false => trn("Merge %d Chat", "Merge %d Chats", n, &[&n]),
                };
                let submit = (!d.title.trim().is_empty() && !d.saving).then(|| edit(MergeEdit::Submit));
                let back = pill(trx("Back", "merged chat wizard").into(), false, Some(edit(MergeEdit::Back)));
                Some(row![back, Space::new().width(Fill), pill(label, true, submit)].into())
            }
        };
        let footer = footer.map(|f| container(f).padding(pad(16.0, 12.0, 10.0, 12.0)));
        let card = container(column![container(body).height(Fill)].push(footer)).height(Fill).clip(true);
        let card = card.style(fill_box(p.card, 14.0));
        let head = container(semi(Self::merge_title(d), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        // Desktop's `outsideFooter` of the intro.
        let outside = (d.step == MergeStep::Intro).then(|| {
            let link = button(txt(tr("Learn more"), 12.0).color(p.accent)).bare(p.accent).on_press(UiMessage::OpenUrl(MERGE_HELP.into()));
            let line = row![txt(tr("You can separate merged chats anytime"), 12.0).color(p.sub), link].spacing(4);
            container(line).center_x(Fill).padding(pad(8.0, 0.0, 2.0, 0.0))
        });
        let window = column![head, card].push(outside);
        container(window).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's intro: the chats or three sample faces with the merged chat badge, the headline, Get Started and the
    /// Beeper Plus upsell.
    fn view_merge_intro<'a>(&'a self, d: &'a MergeDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = |e| UiMessage::Merged(merged::Msg::Edit(e));
        let can = st.can_create_union();
        let picked: Vec<&Chat> = d.chats.iter().filter_map(|c| st.find_chat(c)).take(5).collect();
        let mut faces: Vec<El> = picked.iter().map(|c| self.chat_avatar(c, 48.0, 1.25)).collect();
        if faces.is_empty() {
            faces = LABEL_INTRO_FACES.iter().map(|h| image(h.clone()).width(48).height(48).into()).collect();
        }
        let n = faces.len();
        let faces = faces.into_iter().enumerate().fold(Stack::new(), |s, (i, face)| {
            let ring = container(face).padding(4).style(fill_box(p.card, 28.0));
            s.push(container(ring).padding(Padding::ZERO.left(i as f32 * 33.0)))
        });
        let tile = svg(crate::style::brand_round("merged-chat")).width(26).height(26);
        let tile = container(tile).padding(2).style(fill_box(p.card, 15.0));
        let width = 56.0 + (n - 1) as f32 * 33.0;
        let stack = stack![container(faces).padding(Padding::ZERO.top(15.0)), container(tile).align_right(width + 15.0)];
        let headline = match picked.first().map(|c| c.title.trim()).filter(|t| !t.is_empty()) {
            Some(t) => trf("View all your chats with %s as one conversation", &[&t]),
            None => tr("View chats with the same contacts, across different accounts, as one conversation.").into(),
        };
        let headline = semi(headline, 13.0).color(p.strong).align_x(Center);
        let (label, go) = match can {
            true => (tr("Get Started"), MergeEdit::Start),
            false => (tr("Upgrade"), MergeEdit::Upgrade),
        };
        let go = button(container(med(label, 13.0)).center_y(32))
            .padding([0, 14])
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 16.0))
            .on_press(edit(go));
        let intro = column![stack, headline, go].spacing(16).align_x(Center).padding(26);
        let max = st.features.max_merge_chats();
        let upsell = (max >= 0 && (!st.features.subscribed || !can)).then(|| {
            let sub = match can {
                true => trn("Merge %d chat for free", "Merge %d chats for free", max as usize, &[&max]),
                false => tr("You’ve reached your merged chat limit").into(),
            };
            let text = column![semi(tr("Go unlimited with Beeper Plus"), 12.0).color(p.strong), txt(sub, 12.0).color(p.sub)]
                .spacing(2)
                .width(Fill);
            let action = container(txt(tr("Upgrade"), 10.0).color(Color::WHITE)).padding([1, 6]);
            let action = action.style(fill_box(p.strong, 999.0));
            let card = row![text, action].spacing(10).align_y(Center).padding(12);
            let card = button(card).bare(p.strong).on_press(edit(MergeEdit::Upgrade));
            container(container(card).style(bordered_box(CLEAR, p.prefs_border, 0.5, 10.0))).padding(pad(0.0, 16.0, 16.0, 16.0))
        });
        let plus = container(badge(p, prefs::Badge::Plus)).align_right(Fill).padding(Padding::new(12.0).right(14.0));
        let body = column![container(intro).center(Fill)].push(upsell);
        stack![body, plus].into()
    }

    /// Desktop's `cnt` row of a chat: its avatar with the network badge, its name and identifier, and a checkbox.
    fn merge_row<'a>(&'a self, st: &BeeperStore, c: &Chat, on: bool) -> El<'a> {
        let p = &self.pal;
        let mono = (!st.settings.on("colorizeNetworkIcons")).then_some(p);
        let face = badged(self.chat_avatar(c, 40.0, 1.25), chat_brand(c), 40.0, 14.0, mono, p.card);
        let name = ell(&c.title, 220.0, 14.0, Weight::Medium).color(p.strong);
        let text = column![name].push(identifier(c).map(|i| ell(&i, 220.0, 13.0, Weight::Normal).color(p.sub)));
        let line = row![face, text.spacing(4).width(Fill), container(checkbox(p, on)).padding(Padding::ZERO.right(16.0))];
        let line = line.spacing(10).align_y(Center).padding(pad(10.0, 8.0, 10.0, 16.0));
        button(line).bare(p.strong).width(Fill).on_press(UiMessage::Merged(merged::Msg::Edit(MergeEdit::Toggle(c.id.clone())))).into()
    }

    /// Desktop's pick step: the search row, then the selected chats and the Suggested, Frequently Contacted, Recent
    /// Chats and Chats sections, or the search results.
    fn view_merge_pick<'a>(&'a self, d: &'a MergeDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = |e| UiMessage::Merged(merged::Msg::Edit(e));
        let input = text_input(tr("Search contact"), &d.search)
            .id("merge-search")
            .on_input(move |q| edit(MergeEdit::Search(q)))
            .size(rem(14.0))
            .line_height(lh(18.0))
            .padding([17, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let searching = !d.search.trim().is_empty();
        let clear = searching.then(|| icon_btn(p, icon::CLOSE, 16.0, 28.0, p.sub, edit(MergeEdit::Search(String::new()))));
        let search = row![ic(icon::SEARCH, 16.0).color(p.sub), input].push(clear).spacing(10).align_y(Center);
        let search = container(search).padding(pad(0.0, 8.0, 0.0, 16.0));
        let picked: Vec<&Chat> = d.chats.iter().filter_map(|c| st.find_chat(c)).collect();
        // Desktop's `se`: the mergeable chats, and the member chats of the edited merged chat.
        let pool: Vec<&Chat> = st.chats.iter().filter(|c| mergeable(c) || picked.iter().any(|m| m.id == c.id)).collect();
        let header = |s: &'static str| -> El<'a> { container(txt(tr(s), 12.0).color(p.sub)).padding(pad(8.0, 16.0, 4.0, 16.0)).into() };
        let mut rows: Vec<El<'a>> = Vec::new();
        let taken = |c: &Chat| d.chats.contains(&c.id);
        if searching {
            if !picked.is_empty() {
                rows.push(header("Selected"));
                let chips = picked.iter().map(|c| -> El<'a> {
                    button(self.chat_avatar(c, 40.0, 1.25)).bare(p.strong).on_press(edit(MergeEdit::Toggle(c.id.clone()))).into()
                });
                let strip = iced::widget::Row::with_children(chips).spacing(10).wrap();
                rows.push(container(strip).padding(pad(4.0, 13.0, 8.0, 13.0)).into());
            }
            let q = d.search.trim().to_lowercase();
            let hits: Vec<&Chat> = pool.iter().copied().filter(|c| !taken(c) && c.title.to_lowercase().contains(&q)).collect();
            rows.push(header(if hits.is_empty() { "No matching chats found" } else { "Search results" }));
            rows.extend(hits.into_iter().map(|c| self.merge_row(&st, c, false)));
        } else {
            rows.extend(picked.iter().map(|c| self.merge_row(&st, c, true)));
            // Desktop's `getSuggestedMembersFor`: the chats with the same phone number, email or name.
            let keys: HashSet<String> = picked.iter().flat_map(|c| merge_keys(c)).collect();
            let suggested: Vec<&Chat> = pool
                .iter()
                .copied()
                .filter(|c| !taken(c) && !st.user_blocked(c) && merge_keys(c).iter().any(|k| keys.contains(k)))
                .collect();
            let shown = |c: &Chat, v: &[&Chat]| taken(c) || suggested.iter().chain(v).any(|s| s.id == c.id);
            // Desktop's `frecency` order, which lists chats without any activity too.
            let mut frequent: Vec<&Chat> = pool.iter().copied().filter(|c| !shown(c, &[])).collect();
            frequent.sort_by(|a, b| st.frecency(&b.id).total_cmp(&st.frecency(&a.id)));
            frequent.truncate(SECTION);
            let recent: Vec<&Chat> = st
                .recent_chat_ids
                .iter()
                .filter_map(|id| pool.iter().copied().find(|c| c.id == *id))
                .filter(|c| !shown(c, &frequent))
                .take(SECTION)
                .collect();
            let before: Vec<&Chat> = frequent.iter().chain(&recent).copied().collect();
            let mut rest: Vec<&Chat> = pool.iter().copied().filter(|c| !shown(c, &before)).collect();
            rest.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
            rest.truncate(SECTION);
            for (title, chats) in [("Suggested", suggested), ("Frequently Contacted", frequent), ("Recent Chats", recent), ("Chats", rest)]
            {
                if !chats.is_empty() {
                    rows.push(header(title));
                    rows.extend(chats.into_iter().map(|c| self.merge_row(&st, c, false)));
                }
            }
        }
        let list = scrollable(Column::with_children(rows)).direction(Direction::Vertical(Scrollbar::hidden()));
        column![search, container(list).height(Fill)].into()
    }

    /// The avatar of a photo choice, `size` wide (`Fft`).
    fn merge_photo<'a>(&self, st: &BeeperStore, d: &MergeDraft, photo: &Photo, size: f32) -> El<'a> {
        let existing = d.union.as_deref().and_then(|u| st.find_chat(u)).and_then(|c| c.img_url.as_deref());
        match photo {
            Photo::Keep => self.face(existing, &d.title, size, 1.25),
            Photo::Auto => self.face(None, &d.title, size, 1.25),
            Photo::Member(c) => match st.find_chat(c) {
                Some(c) => self.chat_avatar(c, size, 1.25),
                None => self.face(None, &d.title, size, 1.25),
            },
        }
    }

    /// Desktop's details step: the photo and the name, the photo choices, and the "Default chat" select.
    fn view_merge_details<'a>(&'a self, d: &'a MergeDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = |e| UiMessage::Merged(merged::Msg::Edit(e));
        let members: Vec<&Chat> = d.chats.iter().filter_map(|c| st.find_chat(c)).collect();
        let preview = container(self.merge_photo(&st, d, &d.photo, 88.0)).style(glow_box(CLEAR, p.shadow, 44.0, 4.0, 8.0));
        let name = text_input(tr("Name"), &d.title)
            .id("merge-title")
            .on_input(move |t| edit(MergeEdit::Title(t)))
            .on_submit(edit(MergeEdit::Submit))
            .size(rem(14.0))
            .line_height(lh(18.0))
            .padding([7, 14])
            .align_x(Center)
            .width(200)
            .style(input_style(p.chip, p.strong, p.sub, p.accent, 16.0));
        let center = column![preview, container(name).padding(Padding::ZERO.top(-4.0))].align_x(Center);
        // Desktop's `qft`: the initials, then the member chats with a photo, with a ring around the choice.
        let choices =
            std::iter::once(Photo::Auto).chain(members.iter().filter(|c| member_photo(c).is_some()).map(|c| Photo::Member(c.id.clone())));
        let options = choices.map(|choice| -> El<'a> {
            let on = choice == d.photo;
            let face = self.merge_photo(&st, d, &choice, 40.0);
            let ring = if on { p.accent } else { CLEAR };
            let cell = container(container(face).padding(6)).style(bordered_box(CLEAR, ring, 2.0, 28.0));
            button(container(cell).center(56)).bare(p.strong).on_press(edit(MergeEdit::Photo(choice))).into()
        });
        let options = iced::widget::Row::with_children(options).wrap();
        let options = container(options).center_x(Fill).padding([8, 26]);
        let default = members.iter().find(|c| Some(&c.id) == d.default.as_ref());
        let select = default.map_or_else(|| tr("Select").to_string(), |c| network_name(&c.account_id).to_string());
        let select = row![txt(select, 12.0).color(p.accent), ic(icon::EXPAND_MORE, 14.0).color(p.accent)].spacing(4).align_y(Center);
        let select = button(select)
            .padding([4, 6])
            .style(btn_style([CLEAR, p.chip, p.chip], [p.accent; 3], 20.0))
            .on_press(edit(MergeEdit::DefaultMenu(!d.default_menu)));
        let row = row![semi(tr("Default chat"), 12.0).color(p.strong), Space::new().width(Fill), select].align_y(Center);
        let row = container(row).padding(pad(24.0, 24.0, 30.0, 24.0));
        let body = column![container(center).center(Fill), options, row];
        // Desktop's `st` menu of the member chats, "name · network".
        let menu = d.default_menu.then(|| {
            let items = members.iter().map(|c| {
                let label = format!("{} · {}", c.title, network_name(&c.account_id));
                menu_item(p, None, label, "", Some(edit(MergeEdit::Default(c.id.clone()))))
            });
            let menu = crate::menu_card(p, Column::with_children(items), 240);
            container(menu).align_right(Fill).align_bottom(Fill).padding(pad(0.0, 16.0, 60.0, 0.0))
        });
        stack![body].push(menu).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use beeper_core::matrix::Outgoing;
    use iced_test::simulator::{simulator, Simulator};

    const KAI: &str = "!ig_kai:beeper.com";
    const LENA: &str = "!sg_lena:beeper.com";
    const SAM: &str = "!wa_sam:beeper.com";

    fn edit(app: &mut BeeperGuiApp, e: MergeEdit) {
        let _ = app.update(UiMessage::Merged(merged::Msg::Edit(e)));
    }

    fn view(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
        simulator(app.view_merge_editor(app.merge_window.as_ref().expect("the editor is open")))
    }

    /// Desktop's Merge Chats: the intro the first time, then the chats to merge, then the name and the default chat.
    /// Merging creates the `com.beeper.union` room, opens the merged chat and says so in a toast.
    #[test]
    fn merge_chats_creates_a_merged_chat_like_desktop() {
        let mut app = demo_app("merge-editor-create");
        let _ = app.update(UiMessage::Merged(merged::Msg::Open(None, Vec::new())));
        assert_eq!(app.merge_window.as_ref().map(|d| d.step), Some(MergeStep::Intro), "the intro shows the first time");
        {
            let mut sim = view(&app);
            let headline = "View chats with the same contacts, across different accounts, as one conversation.";
            assert!(sim.find(headline).is_ok() && sim.find("Merge 1 chat for free").is_ok());
            assert!(sim.find("You can separate merged chats anytime").is_ok());
        }
        edit(&mut app, MergeEdit::Start);
        edit(&mut app, MergeEdit::Toggle(KAI.into()));
        assert!(view(&app).find("Merge 1 Chat").is_ok());
        edit(&mut app, MergeEdit::Continue);
        assert_eq!(app.merge_window.as_ref().map(|d| d.step), Some(MergeStep::Pick), "Continue takes two chats");
        edit(&mut app, MergeEdit::Toggle(LENA.into()));
        edit(&mut app, MergeEdit::Continue);
        let d = app.merge_window.as_ref().unwrap();
        assert_eq!((d.step, d.title.as_str(), d.default.as_deref()), (MergeStep::Details, "Kai Nakamura", Some(KAI)));
        {
            let mut sim = view(&app);
            assert!(sim.find("Merge Details").is_ok() && sim.find("Default chat").is_ok());
            assert!(sim.find("Instagram").is_ok() && sim.find("Merge 2 Chats").is_ok());
        }
        edit(&mut app, MergeEdit::DefaultMenu(true));
        assert!(view(&app).find("Lena Fischer · Signal").is_ok(), "the menu names each chat and its network");
        edit(&mut app, MergeEdit::Default(LENA.into()));
        edit(&mut app, MergeEdit::Title("Friends".into()));
        edit(&mut app, MergeEdit::Submit);
        let st = app.store.read();
        let open = app.pane.selected_chat_id.clone().expect("the merged chat opens");
        let c = st.find_chat(&open).expect("the merged chat");
        assert_eq!((c.title.as_str(), st.chat(KAI).merged_into_chat_id.as_deref()), ("Friends", Some(c.id.as_str())));
        assert_eq!(c.merge.as_ref().and_then(|m| m.default_chat_id.as_deref()), Some(LENA));
        let union =
            |o: &Outgoing| matches!(o, Outgoing::CreateUnion { title: Some(t), rooms, .. } if t == "Friends" && rooms == &[KAI, LENA]);
        assert!(st.outbox.iter().any(|(_, chat, o)| *chat == c.id && union(o)), "the union room comes with its children");
        assert_eq!(app.status_banner, "You can now message Friends across 2 chats from one conversation.");
        assert!(st.settings.merge_intro_seen, "the intro shows once");
    }

    /// Desktop's "Frequently Contacted" in the pick step, by the sends and the dwells of each chat: a chat that stays
    /// open 3 seconds counts.
    #[test]
    fn merge_pick_lists_frequently_contacted_chats_like_desktop() {
        let mut app = demo_app("merge-editor-frequent");
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        let _ = app.update(UiMessage::SelectChat(SAM.into()));
        let ago = std::time::Instant::now() - std::time::Duration::from_secs(4);
        app.dwell.as_mut().expect("the open chat").1 = ago;
        let _ = app.update(UiMessage::Noop);
        let _ = app.update(UiMessage::Noop);
        let st = app.store.read();
        assert!(st.frecency(SAM) > 0.6 && st.frecency(SAM) < 0.62, "one dwell of the open chat");
        assert_eq!(st.frecency(KAI), 0.0, "a chat left before 3 seconds does not count");
        drop(st);
        let _ = app.update(UiMessage::Merged(merged::Msg::Open(None, Vec::new())));
        edit(&mut app, MergeEdit::Start);
        let mut sim = view(&app);
        assert!(sim.find("Frequently Contacted").is_ok());
    }

    /// Desktop's Edit of a merged chat from the Manage menu of Chat Info: its chats are picked, and the change of a
    /// chat writes its `m.space.child` state.
    #[test]
    fn manage_edits_a_merged_chat_like_desktop() {
        let mut app = demo_app("merge-editor-edit");
        let rooms = vec![KAI.to_string(), LENA.to_string()];
        let id = app.store.write().create_union("Friends", rooms, None, None);
        app.store.write().outbox.clear();
        let _ = app.update(UiMessage::SelectChat(id.clone()));
        app.show_right_panel = true;
        {
            let mut sim = simulator(app.view_info());
            assert!(sim.find("Merged Chats").is_ok() && sim.find("Default").is_ok());
            let _ = sim.click("Manage").expect("Manage");
            let menu = format!("merged:{id}");
            let opens = |m: &UiMessage| matches!(m, UiMessage::ChatList(chat_list::Msg::OpenMenu(k)) if *k == menu);
            assert!(sim.into_messages().any(|m| opens(&m)), "Manage opens its menu");
        }
        let _ = app.update(UiMessage::Merged(merged::Msg::Open(Some(id.clone()), Vec::new())));
        let d = app.merge_window.as_ref().unwrap();
        assert_eq!((d.step, d.chats.len(), d.title.as_str()), (MergeStep::Pick, 2, "Friends"), "an edit skips the intro");
        assert!(view(&app).find("Merged Chats").is_ok());
        edit(&mut app, MergeEdit::Toggle(LENA.into()));
        edit(&mut app, MergeEdit::Toggle(SAM.into()));
        edit(&mut app, MergeEdit::Continue);
        assert!(view(&app).find("Confirm").is_ok());
        edit(&mut app, MergeEdit::Submit);
        let st = app.store.read();
        assert_eq!(st.chat(SAM).merged_into_chat_id.as_deref(), Some(id.as_str()));
        assert_eq!(st.chat(LENA).merged_into_chat_id, None, "the removed chat is back in the list");
        let changes: Vec<(&str, bool)> = st
            .outbox
            .iter()
            .filter_map(|(_, _, o)| match o {
                Outgoing::LabelChild(room, add) => Some((room.as_str(), *add)),
                _ => None,
            })
            .collect();
        assert_eq!(changes, [(SAM, true), (LENA, false)]);
    }
}
