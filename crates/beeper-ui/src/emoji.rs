use crate::style::{med, mix, txt, CLEAR};
use crate::{BeeperGuiApp, El, UiMessage};
use iced::gradient::Linear;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, operation, row, scrollable, stack, svg, text_input, Space};
use iced::{border, Background, Border, Center, Color, Degrees, Fill, Gradient, Padding, Shadow, Task, Vector};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

const PER_LINE: usize = 9;
const CELL: f32 = 36.0;
const HEAD: f32 = 30.0;
const GRID_H: f32 = 278.0;
pub(crate) const EMOJI_FAMILY: &str = "Twemoji Mozilla";
const EMOJI_FONT: iced::Font = iced::Font::with_name(EMOJI_FAMILY);
const SEARCH_INPUT: &str = "emoji-search";
const GRID: &str = "emoji-grid";
const MODIFIERS: [char; 5] = ['\u{1F3FB}', '\u{1F3FC}', '\u{1F3FD}', '\u{1F3FE}', '\u{1F3FF}'];
const SKINS: [(&str, u32); 6] = [
    ("Default", 0xffc93a),
    ("Light", 0xffdab7),
    ("Medium-Light", 0xe7b98f),
    ("Medium", 0xc88c61),
    ("Medium-Dark", 0xa46134),
    ("Dark", 0x5d4437),
];
const CATEGORIES: [(&str, &str); 9] = [
    ("frequent", "Frequently used"),
    ("people", "Smileys & People"),
    ("nature", "Animals & Nature"),
    ("foods", "Food & Drink"),
    ("activity", "Activity"),
    ("places", "Travel & Places"),
    ("objects", "Objects"),
    ("symbols", "Symbols"),
    ("flags", "Flags"),
];

pub struct Emoji {
    pub native: &'static str,
    pub name: &'static str,
    search: String,
    /// 0 has no skin tones, 1 puts the modifier after the first code point, 2 also after the last.
    skin: u8,
}

impl Emoji {
    fn id(&self) -> &str {
        self.search[1..].split(',').next().expect("emoji search starts with the id")
    }

    /// The emoji in skin tone `tone`, where 0 is the default yellow.
    pub fn skinned(&self, tone: u8) -> String {
        if self.skin == 0 || tone == 0 {
            return self.native.into();
        }
        let m = MODIFIERS[usize::from(tone - 1)];
        let mut cs = self.native.chars();
        let first = cs.next().expect("emoji is not empty");
        let rest = cs.as_str().strip_prefix('\u{FE0F}').unwrap_or(cs.as_str());
        let tail = if self.skin == 2 { Some(m) } else { None };
        std::iter::once(first).chain([m]).chain(rest.chars()).chain(tail).collect()
    }
}

struct Data {
    emojis: Vec<Emoji>,
    categories: Vec<Range<usize>>,
    ids: HashMap<&'static str, usize>,
    /// Natives without skin tones and variation selectors, as Desktop matches recent emoji.
    natives: HashMap<String, usize>,
}

fn bare(s: &str) -> String {
    s.chars().filter(|c| !MODIFIERS.contains(c) && !matches!(c, '\u{FE0E}' | '\u{FE0F}')).collect()
}

static DATA: LazyLock<Data> = LazyLock::new(|| {
    let mut d = Data { emojis: Vec::new(), categories: Vec::new(), ids: HashMap::new(), natives: HashMap::new() };
    for line in include_str!("../assets/emoji.tsv").lines() {
        if !line.contains('\t') {
            let id = line;
            assert_eq!(id, CATEGORIES[d.categories.len() + 1].0, "emoji.tsv category order");
            d.categories.push(d.emojis.len()..d.emojis.len());
            continue;
        }
        let [native, name, search, skin] = line.split('\t').collect::<Vec<_>>()[..] else {
            panic!("emoji.tsv row: {line}")
        };
        let mut e = Emoji { native, name, search: format!(",{search}"), skin: skin.parse().expect("emoji skin kind") };
        let tones = if e.skin == 0 { 0..1 } else { 0..6 };
        for t in tones {
            let s = e.skinned(t);
            e.search = format!("{},{s}", e.search);
            d.natives.entry(bare(&s)).or_insert(d.emojis.len());
        }
        d.ids.insert(search.split(',').next().expect("emoji search starts with the id"), d.emojis.len());
        d.emojis.push(e);
        d.categories.last_mut().expect("emoji.tsv starts with a category").end += 1;
    }
    d
});

/// emoji-mart's `SearchIndex.search`: each word narrows the pool, and the last word ranks it.
fn search(q: &str) -> Vec<usize> {
    let mut q = q.to_lowercase();
    let dash =
        q.char_indices().zip(q.chars().skip(1)).find(|((_, c), n)| (c.is_alphanumeric() || *c == '_') && *n == '-');
    if let Some(((i, c), _)) = dash {
        q.replace_range(i + c.len_utf8()..i + c.len_utf8() + 1, " ");
    }
    let mut words: Vec<&str> = Vec::new();
    for w in q.split(|c: char| c.is_whitespace() || c == '|' || c == ',').filter(|w| !w.is_empty()) {
        if !words.contains(&w) {
            words.push(w);
        }
    }
    let mut pool: Vec<(usize, usize)> = (0..DATA.emojis.len()).map(|i| (i, 0)).collect();
    for w in &words {
        let needle = format!(",{w}");
        pool = pool
            .into_iter()
            .filter_map(|(i, _)| {
                let e = &DATA.emojis[i];
                let at = e.search.find(&needle)?;
                Some((i, if e.id() == *w { 0 } else { at + 1 }))
            })
            .collect();
    }
    let collate = |s: &str| s.chars().map(|c| if c == '_' { '\0' } else { c }).collect::<String>();
    pool.sort_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| collate(DATA.emojis[a.0].id()).cmp(&collate(DATA.emojis[b.0].id())))
    });
    pool.into_iter().take(90).map(|(i, _)| i).collect()
}

#[derive(Debug, Clone)]
pub enum EmojiMsg {
    Toggle,
    /// The pointer enters (`true`) or leaves the emoji button (0) or the picker (1).
    Pointer(u8, bool),
    Close(u32),
    Query(String),
    Hover(usize),
    Unhover(usize),
    Pick(usize),
    Jump(usize),
    Scrolled(f32),
    SkinMenu,
    Skin(u8),
}

#[derive(Default)]
pub struct Picker {
    query: String,
    hover: Option<usize>,
    skins_open: bool,
    scroll: f32,
}

struct Colors {
    bg: Color,
    fg: Color,
    accent: Color,
    input: Color,
    border: Color,
}

impl BeeperGuiApp {
    fn emoji_colors(&self) -> Colors {
        match self.dark {
            true => Colors {
                bg: Color::from_rgb8(21, 22, 23),
                fg: Color::from_rgb8(222, 222, 221),
                accent: Color::from_rgb8(58, 130, 247),
                input: Color::BLACK,
                border: Color::from_rgba(1.0, 1.0, 1.0, 0.1),
            },
            false => Colors {
                bg: Color::WHITE,
                fg: Color::from_rgb8(34, 36, 39),
                accent: Color::from_rgb8(34, 102, 237),
                input: Color::WHITE,
                border: Color::from_rgba(0.0, 0.0, 0.0, 0.05),
            },
        }
    }

    /// The picker sections: frequently used (when there are any), then each category, or the search results.
    fn emoji_sections(&self, pk: &Picker) -> Vec<(&'static str, Vec<usize>)> {
        if !pk.query.trim().is_empty() {
            return vec![("Search Results", search(&pk.query))];
        }
        let frequent = self.frequent_emoji();
        let frequent = (!frequent.is_empty()).then_some((CATEGORIES[0].1, frequent));
        let cats = DATA.categories.iter().zip(&CATEGORIES[1..]).map(|(r, (_, title))| (*title, r.clone().collect()));
        frequent.into_iter().chain(cats).collect()
    }

    /// Desktop's top 27 recent emoji, merged by emoji and sorted by use count.
    fn frequent_emoji(&self) -> Vec<usize> {
        let st = self.store.read().unwrap();
        let mut counts: Vec<(usize, u64)> = Vec::new();
        for (key, n) in &st.recent_emoji {
            let id = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic() || c == '_');
            let found = if id { DATA.ids.get(key.as_str()) } else { DATA.natives.get(&bare(key)) };
            let Some(&i) = found else { continue };
            match counts.iter_mut().find(|(j, _)| *j == i) {
                Some((_, c)) => *c += n,
                None => counts.push((i, *n)),
            }
        }
        counts.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        counts.into_iter().take(27).map(|(i, _)| i).collect()
    }

    pub(crate) fn update_emoji(&mut self, m: EmojiMsg) -> Task<UiMessage> {
        let on_hover = self.store.read().unwrap().settings.on("emojiPickerOnHover");
        match m {
            EmojiMsg::Toggle if on_hover => return Task::none(),
            EmojiMsg::Toggle if self.emoji.is_some() => self.emoji = None,
            EmojiMsg::Toggle => return self.open_emoji(),
            EmojiMsg::Pointer(area, inside) => {
                self.emoji_hovered[usize::from(area)] = inside;
                self.emoji_gen += 1;
                match inside {
                    true if on_hover && self.emoji.is_none() => return self.open_emoji(),
                    false if on_hover => {
                        let g = self.emoji_gen;
                        let wait = tokio::time::sleep(std::time::Duration::from_millis(200));
                        return Task::perform(wait, move |_| UiMessage::Emoji(EmojiMsg::Close(g)));
                    }
                    _ => {}
                }
            }
            EmojiMsg::Close(g) if g == self.emoji_gen && self.emoji_hovered == [false; 2] => self.emoji = None,
            EmojiMsg::Close(_) => {}
            EmojiMsg::Pick(i) => {
                let tone = self.store.read().unwrap().skin_tone;
                let text = format!("{}{}", self.composer_text, DATA.emojis[i].skinned(tone));
                return self.update(UiMessage::ComposerChanged(text));
            }
            EmojiMsg::Skin(t) => {
                let client = {
                    let mut st = self.store.write().unwrap();
                    st.skin_tone = t;
                    st.matrix.clone()
                };
                self.emoji.as_mut().expect("the picker is open").skins_open = false;
                if let Some(c) = client {
                    let content = serde_json::json!({ "tone": t });
                    let write = beeper_core::matrix::set_global(c, "com.beeper.emoji_skin_tone", content);
                    return Task::perform(write, UiMessage::Pushed);
                }
            }
            EmojiMsg::Jump(k) => {
                let pk = self.emoji.as_mut().expect("the picker is open");
                pk.query.clear();
                let pk = self.emoji.as_ref().expect("the picker is open");
                let y = self.emoji_sections(pk).iter().take(k).map(|(_, v)| section_height(v.len())).sum();
                return operation::scroll_to(GRID, operation::AbsoluteOffset { x: 0.0, y });
            }
            m => {
                let pk = self.emoji.as_mut().expect("the picker is open");
                match m {
                    EmojiMsg::Query(q) => {
                        pk.query = q;
                        return operation::scroll_to(GRID, operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                    }
                    EmojiMsg::Hover(i) => pk.hover = Some(i),
                    EmojiMsg::Unhover(i) if pk.hover == Some(i) => pk.hover = None,
                    EmojiMsg::Scrolled(y) => pk.scroll = y,
                    EmojiMsg::SkinMenu => pk.skins_open = !pk.skins_open,
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn open_emoji(&mut self) -> Task<UiMessage> {
        self.emoji = Some(Picker::default());
        self.emoji_hovered[1] = false;
        operation::focus(SEARCH_INPUT)
    }

    pub(crate) fn view_emoji_picker(&self) -> Option<El<'_>> {
        let pk = self.emoji.as_ref()?;
        let c = self.emoji_colors();
        let tone = self.store.read().unwrap().skin_tone;
        let sections = self.emoji_sections(pk);
        let tops: Vec<f32> = sections
            .iter()
            .scan(0.0, |y, (_, v)| {
                let top = *y;
                *y += section_height(v.len());
                Some(top)
            })
            .collect();
        let current = tops.iter().rposition(|&t| t <= pk.scroll + 1.0).expect("the first section starts at 0");
        let searching = !pk.query.trim().is_empty();
        let frequent = usize::from(!self.frequent_emoji().is_empty());
        let nav = row((1 - frequent..CATEGORIES.len()).enumerate().map(|(k, cat)| {
            let on = !searching && k == current;
            let (fg, accent, dim) = (c.fg, c.accent, mix(c.bg, c.fg, 0.65));
            let glyph = svg(CATEGORY_ICONS[cat].clone()).width(18).height(18).style(move |_, s| svg::Style {
                color: Some(match (on, s) {
                    (true, _) => accent,
                    (false, svg::Status::Hovered) => fg,
                    (false, svg::Status::Idle) => dim,
                }),
            });
            button(container(glyph).center_x(CELL).center_y(42))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Emoji(EmojiMsg::Jump(k)))
                .into()
        }))
        .padding(Padding::ZERO.left(12.0));
        let bar = (!searching).then(|| {
            let bar = container(Space::new().width(CELL).height(3)).style(move |_| container::Style {
                background: Some(c.accent.into()),
                border: border::rounded(border::top(3)),
                ..Default::default()
            });
            container(bar).padding(Padding::ZERO.top(39.0).left(12.0 + CELL * current as f32))
        });
        let nav = stack![nav].push(bar).height(42);
        let fade = |down: bool, border: Color| {
            let g = Linear::new(Degrees(if down { 180.0 } else { 0.0 })).add_stop(0.0, border).add_stop(1.0, CLEAR);
            container(Space::new().width(Fill).height(2))
                .style(move |_| container::Style { background: Some(Gradient::Linear(g).into()), ..Default::default() })
        };
        let (fg, accent, input_bg, edge) = (c.fg, c.accent, c.input, c.border);
        let input = text_input("Search", &pk.query)
            .id(SEARCH_INPUT)
            .on_input(|q| UiMessage::Emoji(EmojiMsg::Query(q)))
            .size(14)
            .padding(Padding::new(10.0).left(30.8).right(28.0))
            .style(move |_, s| {
                let focused = matches!(s, text_input::Status::Focused { .. });
                text_input::Style {
                    background: Background::Color(if focused { input_bg } else { edge }),
                    border: Border { color: if focused { accent } else { CLEAR }, width: 1.0, radius: 10.0.into() },
                    icon: fg,
                    placeholder: fg.scale_alpha(0.6),
                    value: fg,
                    selection: accent.scale_alpha(0.45),
                }
            });
        let faint = mix(c.input, c.fg, 0.7);
        let icon =
            |h: &svg::Handle| svg(h.clone()).width(14).height(14).style(move |_, _| svg::Style { color: Some(faint) });
        let loupe = container(icon(&SEARCH_ICONS[0])).center_y(Fill).padding(Padding::ZERO.left(9.8));
        let clear = (!pk.query.is_empty()).then(|| {
            let x = button(icon(&SEARCH_ICONS[1])).padding(0).style(|_, _| button::Style::default());
            container(x.on_press(UiMessage::Emoji(EmojiMsg::Query(String::new()))))
                .center_y(Fill)
                .align_right(Fill)
                .padding(Padding::ZERO.right(9.8))
        });
        let search = container(stack![input, loupe].push(clear).height(37))
            .padding(Padding::new(0.0).left(12.0).right(12.0).top(10.0));
        let search = stack![search, fade(true, edge)];
        let (mut items, mut y, mut skipped): (Vec<El<'static>>, f32, f32) = (Vec::new(), 0.0, 0.0);
        let header = |title: &'static str| {
            container(med(title, 15.0).color(fg)).padding(6).width(Fill).height(HEAD).style(move |_| container::Style {
                background: Some(c.bg.scale_alpha(0.9).into()),
                ..Default::default()
            })
        };
        let mut push = |el: El<'static>, h: f32| {
            if y + h > pk.scroll - CELL && y < pk.scroll + GRID_H + CELL {
                if skipped > 0.0 {
                    items.push(Space::new().height(skipped).into());
                    skipped = 0.0;
                }
                items.push(el);
            } else {
                skipped += h;
            }
            y += h;
        };
        for (title, list) in &sections {
            push(header(title).into(), HEAD);
            for line in list.chunks(PER_LINE) {
                push(row(line.iter().map(|&i| self.emoji_cell(i, tone, edge))).height(CELL).into(), CELL);
            }
        }
        let last = sections.last().map_or(0.0, |(_, v)| section_height(v.len()));
        items.push(Space::new().height(skipped + (GRID_H + 1.0 - last).max(0.0)).into());
        let col = column(items);
        let grid = scrollable(container(col).padding(Padding::ZERO.left(12.0)).width(12.0 + CELL * PER_LINE as f32))
            .id(GRID)
            .on_scroll(|v| UiMessage::Emoji(EmojiMsg::Scrolled(v.absolute_offset().y)))
            .direction(Direction::Vertical(Scrollbar::new().width(8).scroller_width(8).margin(4)))
            .style(move |_, s| {
                let thumb = match s {
                    scrollable::Status::Active { .. } => CLEAR,
                    scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. }
                    | scrollable::Status::Dragged { .. } => edge.scale_alpha(2.0),
                    scrollable::Status::Hovered { .. } => edge,
                };
                crate::style::rail_style(CLEAR, thumb)
            })
            .height(GRID_H);
        let sticky = sections.get(current).filter(|_| pk.scroll > 0.0).map(|(title, _)| {
            let next = tops.get(current + 1).map_or(f32::MAX, |&t| t - pk.scroll);
            let shift = (next - HEAD).min(0.0);
            container(header(title)).padding(Padding::ZERO.left(12.0).top(shift.max(-HEAD))).height(HEAD).clip(true)
        });
        let grid = stack![grid].push(sticky).push(container(fade(false, edge)).align_bottom(GRID_H));
        let col = column![nav, search, grid, self.emoji_preview(pk, &sections, tone, &c)];
        let body = container(col).width(352).height(435).style(move |_| container::Style {
            background: Some(c.bg.into()),
            border: border::rounded(10),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.14),
                offset: Vector::new(0.3, 0.5),
                blur_radius: 2.7,
            },
            ..Default::default()
        });
        let skins = pk.skins_open.then(|| self.emoji_skin_menu(tone, &c));
        let layered = stack![body].push(skins);
        Some(
            mouse_area(layered)
                .on_enter(UiMessage::Emoji(EmojiMsg::Pointer(1, true)))
                .on_exit(UiMessage::Emoji(EmojiMsg::Pointer(1, false)))
                .into(),
        )
    }

    fn emoji_cell(&self, i: usize, tone: u8, hover: Color) -> El<'static> {
        let glyph = txt(DATA.emojis[i].skinned(tone), 24.0).line_height(1.0).font(EMOJI_FONT);
        let b = button(container(glyph).center(CELL)).padding(0).style(move |_, s| button::Style {
            background: matches!(s, button::Status::Hovered | button::Status::Pressed).then(|| hover.into()),
            border: border::rounded(CELL / 2.0),
            ..Default::default()
        });
        mouse_area(b.on_press(UiMessage::Emoji(EmojiMsg::Pick(i))))
            .on_enter(UiMessage::Emoji(EmojiMsg::Hover(i)))
            .on_exit(UiMessage::Emoji(EmojiMsg::Unhover(i)))
            .into()
    }

    fn emoji_preview(&self, pk: &Picker, sections: &[(&str, Vec<usize>)], tone: u8, c: &Colors) -> El<'static> {
        let none = sections.iter().all(|(_, v)| v.is_empty());
        let shown = pk.hover.map(|i| {
            (DATA.emojis[i].skinned(tone), DATA.emojis[i].name.to_string(), format!(":{}:", DATA.emojis[i].id()))
        });
        let (glyph, text): (String, El<'static>) = match (shown, none) {
            (Some((g, name, code)), _) => {
                (g, column![txt(name, 16.5).color(c.fg), txt(code, 13.5).color(c.fg.scale_alpha(0.45))].into())
            }
            (None, true) => (
                "😢".into(),
                column![
                    txt("Oh no!", 16.5).color(c.fg),
                    txt("That emoji couldn’t be found", 13.5).color(c.fg.scale_alpha(0.45))
                ]
                .into(),
            ),
            (None, false) => ("☝️".into(), txt("Pick an emoji…", 21.0).color(c.fg.scale_alpha(0.45)).into()),
        };
        let skin = (pk.hover.is_none() && !none).then(|| {
            let (edge, open) = (c.border, pk.skins_open);
            let b = button(container(skin_dot(tone)).center(22))
                .padding(0)
                .style(move |_, s| button::Style {
                    background: open.then(|| edge.into()),
                    border: Border {
                        color: if matches!(s, button::Status::Hovered) { edge } else { CLEAR },
                        width: 1.0,
                        radius: 12.0.into(),
                    },
                    ..Default::default()
                })
                .on_press(UiMessage::Emoji(EmojiMsg::SkinMenu));
            container(b).center(CELL)
        });
        let emoji = container(txt(glyph, 36.0).line_height(1.0).font(EMOJI_FONT)).center(CELL);
        container(row![emoji, container(text).padding(Padding::ZERO.left(12.0)).width(Fill)].push(skin).align_y(Center))
            .padding(Padding::new(16.0).left(12.0).right(16.0))
            .height(68)
            .into()
    }

    fn emoji_skin_menu(&self, tone: u8, c: &Colors) -> El<'static> {
        let (fg, accent, edge) = (c.fg, c.accent, c.border);
        let widest = SKINS
            .iter()
            .map(|(l, _)| crate::style::text_width(l, 15.0, iced::font::Weight::Normal))
            .fold(0.0, f32::max);
        let options = SKINS.iter().enumerate().map(|(t, (label, _))| {
            let t = t as u8;
            let picked = t == tone;
            button(row![skin_dot(t), txt(*label, 15.0)].spacing(6).align_y(Center))
                .padding(Padding::new(4.0).left(6.0).right(12.0))
                .width(40.0 + widest.ceil())
                .style(move |_, s| {
                    let hot = matches!(s, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: hot.then(|| accent.into()),
                        text_color: if hot { Color::WHITE } else { fg },
                        border: Border { color: if picked { accent } else { CLEAR }, width: 2.0, radius: 6.0.into() },
                        ..Default::default()
                    }
                })
                .on_press(UiMessage::Emoji(EmojiMsg::Skin(t)))
                .into()
        });
        let bg = c.bg.scale_alpha(0.9);
        let menu = container(column(options)).padding(4).style(move |_| container::Style {
            background: Some(bg.into()),
            border: Border { color: edge, width: 1.0, radius: 10.0.into() },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.05),
                offset: Vector::new(1.0, 1.0),
                blur_radius: 5.0,
            },
            ..Default::default()
        });
        container(menu).align_right(352).align_bottom(435).padding(Padding::ZERO.right(19.0).bottom(52.0)).into()
    }
}

fn section_height(n: usize) -> f32 {
    HEAD + n.div_ceil(PER_LINE) as f32 * CELL
}

fn skin_dot(t: u8) -> El<'static> {
    let fill = crate::style::hex(SKINS[usize::from(t)].1);
    container(Space::new().width(16).height(16))
        .style(move |_| container::Style {
            background: Some(fill.into()),
            border: Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.25), width: 1.0, radius: 8.0.into() },
            ..Default::default()
        })
        .into()
}

static SEARCH_ICONS: LazyLock<[svg::Handle; 2]> = LazyLock::new(|| {
    [
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M12.9 14.32a8 8 0 1 1 1.41-1.41l5.35 5.33-1.42 1.42-5.33-5.34zM8 14A6 6 0 1 0 8 2a6 6 0 0 0 0 12z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M10 8.586L2.929 1.515 1.515 2.929 8.586 10l-7.071 7.071 1.414 1.414L10 11.414l7.071 7.071 1.414-1.414L11.414 10l7.071-7.071-1.414-1.414L10 8.586z"/></svg>"#,
    ]
    .map(|s| svg::Handle::from_memory(s.as_bytes()))
});

static CATEGORY_ICONS: LazyLock<[svg::Handle; 9]> = LazyLock::new(|| {
    [
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M256 512C114.6 512 0 397.4 0 256C0 114.6 114.6 0 256 0C397.4 0 512 114.6 512 256C512 397.4 397.4 512 256 512zM232 256C232 264 236 271.5 242.7 275.1L338.7 339.1C349.7 347.3 364.6 344.3 371.1 333.3C379.3 322.3 376.3 307.4 365.3 300L280 243.2V120C280 106.7 269.3 96 255.1 96C242.7 96 231.1 106.7 231.1 120L232 256z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M0 256C0 114.6 114.6 0 256 0C397.4 0 512 114.6 512 256C512 397.4 397.4 512 256 512C114.6 512 0 397.4 0 256zM256 432C332.1 432 396.2 382 415.2 314.1C419.1 300.4 407.8 288 393.6 288H118.4C104.2 288 92.92 300.4 96.76 314.1C115.8 382 179.9 432 256 432V432zM176.4 160C158.7 160 144.4 174.3 144.4 192C144.4 209.7 158.7 224 176.4 224C194 224 208.4 209.7 208.4 192C208.4 174.3 194 160 176.4 160zM336.4 224C354 224 368.4 209.7 368.4 192C368.4 174.3 354 160 336.4 160C318.7 160 304.4 174.3 304.4 192C304.4 209.7 318.7 224 336.4 224z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 576 512"><path d="M332.7 19.85C334.6 8.395 344.5 0 356.1 0C363.6 0 370.6 3.52 375.1 9.502L392 32H444.1C456.8 32 469.1 37.06 478.1 46.06L496 64H552C565.3 64 576 74.75 576 88V112C576 156.2 540.2 192 496 192H426.7L421.6 222.5L309.6 158.5L332.7 19.85zM448 64C439.2 64 432 71.16 432 80C432 88.84 439.2 96 448 96C456.8 96 464 88.84 464 80C464 71.16 456.8 64 448 64zM416 256.1V480C416 497.7 401.7 512 384 512H352C334.3 512 320 497.7 320 480V364.8C295.1 377.1 268.8 384 240 384C211.2 384 184 377.1 160 364.8V480C160 497.7 145.7 512 128 512H96C78.33 512 64 497.7 64 480V249.8C35.23 238.9 12.64 214.5 4.836 183.3L.9558 167.8C-3.331 150.6 7.094 133.2 24.24 128.1C41.38 124.7 58.76 135.1 63.05 152.2L66.93 167.8C70.49 182 83.29 191.1 97.97 191.1H303.8L416 256.1z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M481.9 270.1C490.9 279.1 496 291.3 496 304C496 316.7 490.9 328.9 481.9 337.9C472.9 346.9 460.7 352 448 352H64C51.27 352 39.06 346.9 30.06 337.9C21.06 328.9 16 316.7 16 304C16 291.3 21.06 279.1 30.06 270.1C39.06 261.1 51.27 256 64 256H448C460.7 256 472.9 261.1 481.9 270.1zM475.3 388.7C478.3 391.7 480 395.8 480 400V416C480 432.1 473.3 449.3 461.3 461.3C449.3 473.3 432.1 480 416 480H96C79.03 480 62.75 473.3 50.75 461.3C38.74 449.3 32 432.1 32 416V400C32 395.8 33.69 391.7 36.69 388.7C39.69 385.7 43.76 384 48 384H464C468.2 384 472.3 385.7 475.3 388.7zM50.39 220.8C45.93 218.6 42.03 215.5 38.97 211.6C35.91 207.7 33.79 203.2 32.75 198.4C31.71 193.5 31.8 188.5 32.99 183.7C54.98 97.02 146.5 32 256 32C365.5 32 457 97.02 479 183.7C480.2 188.5 480.3 193.5 479.2 198.4C478.2 203.2 476.1 207.7 473 211.6C469.1 215.5 466.1 218.6 461.6 220.8C457.2 222.9 452.3 224 447.3 224H64.67C59.73 224 54.84 222.9 50.39 220.8zM372.7 116.7C369.7 119.7 368 123.8 368 128C368 131.2 368.9 134.3 370.7 136.9C372.5 139.5 374.1 141.6 377.9 142.8C380.8 143.1 384 144.3 387.1 143.7C390.2 143.1 393.1 141.6 395.3 139.3C397.6 137.1 399.1 134.2 399.7 131.1C400.3 128 399.1 124.8 398.8 121.9C397.6 118.1 395.5 116.5 392.9 114.7C390.3 112.9 387.2 111.1 384 111.1C379.8 111.1 375.7 113.7 372.7 116.7V116.7zM244.7 84.69C241.7 87.69 240 91.76 240 96C240 99.16 240.9 102.3 242.7 104.9C244.5 107.5 246.1 109.6 249.9 110.8C252.8 111.1 256 112.3 259.1 111.7C262.2 111.1 265.1 109.6 267.3 107.3C269.6 105.1 271.1 102.2 271.7 99.12C272.3 96.02 271.1 92.8 270.8 89.88C269.6 86.95 267.5 84.45 264.9 82.7C262.3 80.94 259.2 79.1 256 79.1C251.8 79.1 247.7 81.69 244.7 84.69V84.69zM116.7 116.7C113.7 119.7 112 123.8 112 128C112 131.2 112.9 134.3 114.7 136.9C116.5 139.5 118.1 141.6 121.9 142.8C124.8 143.1 128 144.3 131.1 143.7C134.2 143.1 137.1 141.6 139.3 139.3C141.6 137.1 143.1 134.2 143.7 131.1C144.3 128 143.1 124.8 142.8 121.9C141.6 118.1 139.5 116.5 136.9 114.7C134.3 112.9 131.2 111.1 128 111.1C123.8 111.1 119.7 113.7 116.7 116.7L116.7 116.7z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M16.17 337.5c0 44.98 7.565 83.54 13.98 107.9C35.22 464.3 50.46 496 174.9 496c9.566 0 19.59-.4707 29.84-1.271L17.33 307.3C16.53 317.6 16.17 327.7 16.17 337.5zM495.8 174.5c0-44.98-7.565-83.53-13.98-107.9c-4.688-17.54-18.34-31.23-36.04-35.95C435.5 27.91 392.9 16 337 16c-9.564 0-19.59 .4707-29.84 1.271l187.5 187.5C495.5 194.4 495.8 184.3 495.8 174.5zM26.77 248.8l236.3 236.3c142-36.1 203.9-150.4 222.2-221.1L248.9 26.87C106.9 62.96 45.07 177.2 26.77 248.8zM256 335.1c0 9.141-7.474 16-16 16c-4.094 0-8.188-1.564-11.31-4.689L164.7 283.3C161.6 280.2 160 276.1 160 271.1c0-8.529 6.865-16 16-16c4.095 0 8.189 1.562 11.31 4.688l64.01 64C254.4 327.8 256 331.9 256 335.1zM304 287.1c0 9.141-7.474 16-16 16c-4.094 0-8.188-1.564-11.31-4.689L212.7 235.3C209.6 232.2 208 228.1 208 223.1c0-9.141 7.473-16 16-16c4.094 0 8.188 1.562 11.31 4.688l64.01 64.01C302.5 279.8 304 283.9 304 287.1zM256 175.1c0-9.141 7.473-16 16-16c4.094 0 8.188 1.562 11.31 4.688l64.01 64.01c3.125 3.125 4.688 7.219 4.688 11.31c0 9.133-7.468 16-16 16c-4.094 0-8.189-1.562-11.31-4.688l-64.01-64.01C257.6 184.2 256 180.1 256 175.1z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M39.61 196.8L74.8 96.29C88.27 57.78 124.6 32 165.4 32H346.6C387.4 32 423.7 57.78 437.2 96.29L472.4 196.8C495.6 206.4 512 229.3 512 256V448C512 465.7 497.7 480 480 480H448C430.3 480 416 465.7 416 448V400H96V448C96 465.7 81.67 480 64 480H32C14.33 480 0 465.7 0 448V256C0 229.3 16.36 206.4 39.61 196.8V196.8zM109.1 192H402.9L376.8 117.4C372.3 104.6 360.2 96 346.6 96H165.4C151.8 96 139.7 104.6 135.2 117.4L109.1 192zM96 256C78.33 256 64 270.3 64 288C64 305.7 78.33 320 96 320C113.7 320 128 305.7 128 288C128 270.3 113.7 256 96 256zM416 320C433.7 320 448 305.7 448 288C448 270.3 433.7 256 416 256C398.3 256 384 270.3 384 288C384 305.7 398.3 320 416 320z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 384 512"><path d="M112.1 454.3c0 6.297 1.816 12.44 5.284 17.69l17.14 25.69c5.25 7.875 17.17 14.28 26.64 14.28h61.67c9.438 0 21.36-6.401 26.61-14.28l17.08-25.68c2.938-4.438 5.348-12.37 5.348-17.7L272 415.1h-160L112.1 454.3zM191.4 .0132C89.44 .3257 16 82.97 16 175.1c0 44.38 16.44 84.84 43.56 115.8c16.53 18.84 42.34 58.23 52.22 91.45c.0313 .25 .0938 .5166 .125 .7823h160.2c.0313-.2656 .0938-.5166 .125-.7823c9.875-33.22 35.69-72.61 52.22-91.45C351.6 260.8 368 220.4 368 175.1C368 78.61 288.9-.2837 191.4 .0132zM192 96.01c-44.13 0-80 35.89-80 79.1C112 184.8 104.8 192 96 192S80 184.8 80 176c0-61.76 50.25-111.1 112-111.1c8.844 0 16 7.159 16 16S200.8 96.01 192 96.01z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M500.3 7.251C507.7 13.33 512 22.41 512 31.1V175.1C512 202.5 483.3 223.1 447.1 223.1C412.7 223.1 383.1 202.5 383.1 175.1C383.1 149.5 412.7 127.1 447.1 127.1V71.03L351.1 90.23V207.1C351.1 234.5 323.3 255.1 287.1 255.1C252.7 255.1 223.1 234.5 223.1 207.1C223.1 181.5 252.7 159.1 287.1 159.1V63.1C287.1 48.74 298.8 35.61 313.7 32.62L473.7 .6198C483.1-1.261 492.9 1.173 500.3 7.251H500.3zM74.66 303.1L86.5 286.2C92.43 277.3 102.4 271.1 113.1 271.1H174.9C185.6 271.1 195.6 277.3 201.5 286.2L213.3 303.1H239.1C266.5 303.1 287.1 325.5 287.1 351.1V463.1C287.1 490.5 266.5 511.1 239.1 511.1H47.1C21.49 511.1-.0019 490.5-.0019 463.1V351.1C-.0019 325.5 21.49 303.1 47.1 303.1H74.66zM143.1 359.1C117.5 359.1 95.1 381.5 95.1 407.1C95.1 434.5 117.5 455.1 143.1 455.1C170.5 455.1 191.1 434.5 191.1 407.1C191.1 381.5 170.5 359.1 143.1 359.1zM440.3 367.1H496C502.7 367.1 508.6 372.1 510.1 378.4C513.3 384.6 511.6 391.7 506.5 396L378.5 508C372.9 512.1 364.6 513.3 358.6 508.9C352.6 504.6 350.3 496.6 353.3 489.7L391.7 399.1H336C329.3 399.1 323.4 395.9 321 389.6C318.7 383.4 320.4 376.3 325.5 371.1L453.5 259.1C459.1 255 467.4 254.7 473.4 259.1C479.4 263.4 481.6 271.4 478.7 278.3L440.3 367.1zM116.7 219.1L19.85 119.2C-8.112 90.26-6.614 42.31 24.85 15.34C51.82-8.137 93.26-3.642 118.2 21.83L128.2 32.32L137.7 21.83C162.7-3.642 203.6-8.137 231.6 15.34C262.6 42.31 264.1 90.26 236.1 119.2L139.7 219.1C133.2 225.6 122.7 225.6 116.7 219.1H116.7z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M64 496C64 504.8 56.75 512 48 512h-32C7.25 512 0 504.8 0 496V32c0-17.75 14.25-32 32-32s32 14.25 32 32V496zM476.3 0c-6.365 0-13.01 1.35-19.34 4.233c-45.69 20.86-79.56 27.94-107.8 27.94c-59.96 0-94.81-31.86-163.9-31.87C160.9 .3055 131.6 4.867 96 15.75v350.5c32-9.984 59.87-14.1 84.85-14.1c73.63 0 124.9 31.78 198.6 31.78c31.91 0 68.02-5.971 111.1-23.09C504.1 355.9 512 344.4 512 332.1V30.73C512 11.1 495.3 0 476.3 0z"/></svg>"#,
    ]
    .map(|s| svg::Handle::from_memory(s.as_bytes()))
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_skins_and_search_match_emoji_mart() {
        assert_eq!(DATA.emojis.len(), 1906);
        let thumbs = DATA.ids["+1"];
        assert_eq!(DATA.emojis[thumbs].skinned(3), "👍🏽");
        assert_eq!(DATA.emojis[DATA.ids["people_holding_hands"]].skinned(1), "🧑🏻‍🤝‍🧑🏻");
        assert_eq!(DATA.natives[&bare("👍🏿")], thumbs);
        assert_eq!(search("thumbs").first(), Some(&thumbs));
        assert!(search("zzzz").is_empty());
    }
}
