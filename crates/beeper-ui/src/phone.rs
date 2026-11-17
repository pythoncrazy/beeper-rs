//! The phone field of Desktop's bridge login form: react-international-phone 4.5.0's `PhoneInput` (`Yf`) with
//! `disableDialCodeAndPrefix` and `showDisabledDialCodeAndPrefix`. It has a country selector with the flag (`qf`),
//! its dropdown of countries (`zf`), the "+<dial code>" preview (`Kf`) and the national number with the country's
//! mask, and it guesses the country from the digits (`Vf`). Its value is the `phone`: `+`, the dial code and the
//! digits. `assets/countries.json` is the library's country list.
use crate::emoji::EMOJI_FONT;
use crate::style::{bar, btn_style, ell, scroll_style, txt, CLEAR, GUTTER};
use crate::{BeeperGuiApp, El, UiMessage};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::font::Weight;
use iced::keyboard::{key::Named, Key};
use iced::widget::{button, container, row, scrollable, text_input, tooltip, Column};
use iced::{
    border, Border, Center, Color, Element, Event, Fill, Length, Padding, Point, Rectangle, Renderer, Shadow, Size,
    Theme, Vector,
};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// The mask symbol (`Ka`) and the mask of a country without a format (`defaultMask`).
const MASK_CHAR: char = '.';
const DEFAULT_MASK: &str = "............";
/// `Hf`: letters typed in the dropdown within a second of each other search together.
const SEARCH_RESET: Duration = Duration::from_millis(1000);
/// The dropdown's rows (`--react-international-phone-dropdown-item-height`), its `padding: 4px 0`, its `width` and
/// its `max-height` (the countries always fill it).
const ITEM: f32 = 28.0;
const LIST_PAD: f32 = 4.0;
const LIST_WIDTH: f32 = 300.0;
const LIST_HEIGHT: f32 = 200.0;
/// The dropdown's `top: var(--react-international-phone-dropdown-top, 44px)` from the selector.
const LIST_TOP: f32 = 44.0;
/// The library's own colors of the dropdown, which come after Beeper's in the cascade: `white`, `#222`, `gray`,
/// `whitesmoke` and `rgba(0, 0, 0, .25)`.
const LIST_BG: Color = Color::WHITE;
const LIST_TEXT: Color = Color::from_rgb(34.0 / 255.0, 34.0 / 255.0, 34.0 / 255.0);
const LIST_DIAL: Color = Color::from_rgb(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0);
const LIST_SELECTED: Color = Color::from_rgb(245.0 / 255.0, 245.0 / 255.0, 245.0 / 255.0);
const LIST_SHADOW: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.25);

/// A country of the library's list: `[name, iso2, dialCode, format?, priority?, areaCodes?]`.
pub(crate) struct Country {
    pub name: String,
    pub iso2: String,
    pub dial: String,
    format: Format,
    priority: i64,
    area_codes: Option<Vec<String>>,
}

/// The `format` of a country: none, one mask, or masks for the national numbers that match a pattern.
enum Format {
    Default,
    Mask(String),
    Rules(Vec<(Pattern, String)>, Option<String>),
}

/// A `/pattern/` key of a format, a JavaScript regular expression. `regex` has no lookahead, so the one pattern with
/// a trailing `(?!x)` matches as `a` and not `ax`.
struct Pattern {
    yes: regex::Regex,
    not: Option<regex::Regex>,
}

impl Pattern {
    fn new(src: &str) -> Self {
        let re = |s: &str| regex::Regex::new(s).expect("a country format pattern compiles");
        match src.split_once("(?!") {
            Some((a, b)) => {
                let b = b.strip_suffix(')').expect("a lookahead at the end of the pattern");
                Pattern { yes: re(a), not: Some(re(&format!("{a}{b}"))) }
            }
            None => Pattern { yes: re(src), not: None },
        }
    }

    fn is_match(&self, s: &str) -> bool {
        self.yes.is_match(s) && !self.not.as_ref().is_some_and(|n| n.is_match(s))
    }
}

pub(crate) static COUNTRIES: LazyLock<Vec<Country>> = LazyLock::new(|| {
    let all: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../assets/countries.json")).expect("the country list is JSON");
    let s = |v: &serde_json::Value| v.as_str().expect("a country string").to_string();
    all.iter()
        .map(|c| {
            let format = match &c[3] {
                serde_json::Value::String(m) => Format::Mask(m.clone()),
                serde_json::Value::Object(o) => Format::Rules(
                    o.iter()
                        .filter(|(k, _)| *k != "default")
                        .map(|(k, v)| (Pattern::new(k.trim_matches('/')), s(v)))
                        .collect(),
                    o.get("default").map(s),
                ),
                _ => Format::Default,
            };
            Country {
                name: s(&c[0]),
                iso2: s(&c[1]),
                dial: s(&c[2]),
                format,
                priority: c[4].as_i64().unwrap_or(0),
                area_codes: c[5].as_array().map(|a| a.iter().map(s).collect()),
            }
        })
        .collect()
});

/// Electron's `app.getLocaleCountryCode()` on Linux: the region of the `LC_TIME` locale that `setlocale` takes from
/// `LC_ALL`, `LC_TIME` or `LANG`, before any `.` and after the `_`, in upper case if it has two letters.
static LOCALE_COUNTRY: LazyLock<String> = LazyLock::new(|| {
    let locale = ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    locale_country(&locale.unwrap_or_default())
});

fn locale_country(locale: &str) -> String {
    let locale = locale.split('.').next().unwrap_or_default();
    let region = locale.split_once('_').map_or("", |(_, r)| r);
    if region.len() == 2 {
        region.to_ascii_uppercase()
    } else {
        String::new()
    }
}

/// The index of the country with the ISO code.
fn index(iso2: &str) -> Option<usize> {
    COUNTRIES.iter().position(|c| c.iso2 == iso2)
}

/// Desktop's `defaultCountry`: the locale's country, or the United States.
pub(crate) fn default_country() -> usize {
    index(&LOCALE_COUNTRY.to_lowercase()).or_else(|| index("us")).expect("the United States in the country list")
}

/// Desktop's `_r` (awesome-phonenumber's `isValid`): libphonenumber takes the number as valid, in the locale's region
/// when it has no `+`.
pub(crate) fn is_valid(phone: &str) -> bool {
    let region = LOCALE_COUNTRY.parse::<phonenumber::country::Id>().ok();
    phonenumber::parse(region, phone).is_ok_and(|n| phonenumber::is_valid(&n))
}

/// `Nr`: the digits of a text.
fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// `Uf`: the mask of the country for the phone.
fn mask<'c>(phone: &str, c: &'c Country) -> &'c str {
    match &c.format {
        Format::Default => DEFAULT_MASK,
        Format::Mask(m) => m,
        // A format without a default mask is an error, which takes the default mask.
        Format::Rules(_, None) => DEFAULT_MASK,
        Format::Rules(rules, Some(default)) => {
            let national = digits(&phone.replacen(c.dial.as_str(), "", 1));
            rules.iter().find(|(p, _)| p.is_match(&national)).map_or(default, |(_, m)| m)
        }
    }
}

/// `_f`: the digits in the mask. Without `trim` the mask's characters after the last digit stay, up to the next
/// digit's place. The digits past the mask are dropped.
fn apply_mask(value: &str, mask: &str, trim: bool) -> String {
    let mut digits = value.chars();
    let mut out = String::new();
    let mut left = value.chars().count();
    for m in mask.chars() {
        if left == 0 {
            if !trim && m != MASK_CHAR {
                out.push(m);
                continue;
            }
            break;
        }
        if m == MASK_CHAR {
            out.extend(digits.next());
            left -= 1;
        } else {
            out.push(m);
        }
    }
    out
}

/// `Cf` with `disableDialCodeAndPrefix`: the national number of the phone, after the dial code, in the mask.
fn national(phone: &str, mask: &str, dial: &str, trim_end: bool) -> String {
    let done = |s: String| if trim_end { s.trim().to_string() } else { s };
    if phone.is_empty() {
        return String::new();
    }
    let all = digits(phone);
    let rest = all.get(dial.len()..).unwrap_or("");
    done(apply_mask(rest, mask, trim_end || rest.is_empty()))
}

/// `Vf`: the country of the phone's digits, and whether its whole dial code matched. A number of an area code goes to
/// its country, a dial code to the country of the lowest `priority`, and the current country stays when it fits.
fn guess(phone: &str, current: Option<usize>) -> (Option<usize>, bool) {
    let r = digits(phone);
    if r.is_empty() {
        return (None, false);
    }
    let num = |d: &str| d.parse::<u64>().unwrap_or(0);
    let mut best: (Option<usize>, bool) = (None, false);
    let set = |best: &mut (Option<usize>, bool), i: usize, full: bool| {
        let old = best.0.map(|b| &COUNTRIES[b]);
        let same = old.is_some_and(|o| o.dial == COUNTRIES[i].dial);
        let first = COUNTRIES[i].priority < old.map_or(0, |o| o.priority);
        if !same || first {
            *best = (Some(i), full);
        }
    };
    for (i, c) in COUNTRIES.iter().enumerate() {
        if r.starts_with(&c.dial) {
            let higher = best.0.is_none_or(|b| num(&c.dial) >= num(&COUNTRIES[b].dial));
            if let Some(areas) = &c.area_codes {
                let rest = &r[c.dial.len()..];
                if areas.iter().any(|a| rest.starts_with(a.as_str())) {
                    return (Some(i), true);
                }
            }
            if higher || c.dial == r || !best.1 {
                set(&mut best, i, true);
            }
        }
        if !best.1
            && r.len() < c.dial.len()
            && c.dial.starts_with(&r)
            && best.0.is_none_or(|b| num(&c.dial) <= num(&COUNTRIES[b].dial))
        {
            set(&mut best, i, false);
        }
    }
    let Some(now) = current else { return best };
    let c = &COUNTRIES[now];
    let rest = r.get(c.dial.len()..).unwrap_or("");
    let fits = c.area_codes.as_ref().is_some_and(|a| a.iter().any(|a| a.starts_with(rest)));
    if let (Some(b), true) = (best.0, best.1) {
        if b != now && COUNTRIES[b].dial == c.dial && (c.area_codes.is_none() || fits) {
            best = (Some(now), true);
        }
    }
    best
}

/// `Li`: the phone, the input's text and the country of a typed value in the country. A value without `+` is the
/// national number.
fn derive(value: &str, country: usize, trim_end: bool) -> (String, String, usize) {
    let c = &COUNTRIES[country];
    let phone = if value.starts_with('+') { value.to_string() } else { format!("+{}{value}", c.dial) };
    let (guessed, full) = guess(&phone, Some(country));
    let g = &COUNTRIES[guessed.unwrap_or(country)];
    let input = national(&phone, mask(&phone, g), &g.dial, trim_end);
    let country = if full { guessed.unwrap_or(country) } else { country };
    (format!("+{}", digits(&format!("{}{input}", COUNTRIES[country].dial))), input, country)
}

/// `Mf`: the cursor after an edit, at the same digit of the formatted text.
fn cursor(at: usize, before: &str, after: &str, formatted: &str, backward: bool) -> usize {
    let (after, formatted): (Vec<char>, Vec<char>) = (after.chars().collect(), formatted.chars().collect());
    let digit = |c: Option<&char>| c.is_some_and(char::is_ascii_digit);
    if before.is_empty() {
        return formatted.len();
    }
    let Some(last) = (0..at.min(after.len())).rev().find(|&h| digit(after.get(h))) else {
        return (0..after.len()).find(|&h| digit(formatted.get(h))).unwrap_or(after.len());
    };
    let before_digits = after[..last].iter().filter(|c| c.is_ascii_digit()).count();
    let (mut c, mut d) = (0, 0);
    for ch in &formatted {
        c += 1;
        d += usize::from(ch.is_ascii_digit());
        if d > before_digits {
            break;
        }
    }
    if !backward {
        while c < formatted.len() && !digit(formatted.get(c)) {
            c += 1;
        }
    }
    c
}

/// An edit of the input from its text before and after: the characters before the edit, and the ones it put there
/// (the browser's `selectionStart` after the edit is their end, and its `data` is them).
fn edit_of(before: &str, after: &str) -> (usize, String) {
    let (before, after): (Vec<char>, Vec<char>) = (before.chars().collect(), after.chars().collect());
    let same = before.iter().zip(&after).take_while(|(a, b)| a == b).count();
    let tail = before[same..].iter().rev().zip(after[same..].iter().rev()).take_while(|(a, b)| a == b).count();
    (same, after[same..after.len() - tail].iter().collect())
}

/// The flag of a country: its two regional indicator letters, which Twemoji draws like Desktop's twemoji images.
fn flag(iso2: &str) -> String {
    iso2.chars().filter_map(|c| char::from_u32(0x1f1e6 + (c as u32).wrapping_sub('a' as u32))).collect()
}

/// The state of a phone field.
pub(crate) struct Phone {
    pub country: usize,
    /// The input's text: the national number in the mask.
    pub input: String,
    /// The value of the field (Desktop's `phone`).
    phone: String,
    pub open: bool,
    /// The dropdown's highlighted country (`w`), the one it last scrolled to (`h`) and its scroll.
    focus: usize,
    shown: usize,
    scroll: f32,
    /// The letters typed in the dropdown, and when the last came.
    search: String,
    searched: Option<Instant>,
}

/// What the dropdown does with a key.
pub(crate) enum KeyOutcome {
    Ignored,
    Moved(Option<f32>),
    Picked(usize),
    Closed,
}

/// A change of the phone field.
#[derive(Clone, Debug)]
pub enum PhoneMsg {
    /// Typing or deleting in the input, with its text after the edit.
    Input(String),
    /// A paste into the input, with its text after the paste.
    Paste(String),
    /// The selector button.
    Toggle,
    Pick(usize),
    /// A press outside the dropdown and the selector: the dropdown's `onBlur`.
    Blur,
    Scrolled(f32),
}

impl Phone {
    /// The field with the value in `country`, and the field's value: empty until it changes, as in Desktop, where the
    /// form's reset of its values comes after the field's first `onChange`.
    pub fn new(value: &str, country: usize) -> (Self, String) {
        let (phone, input, country) = derive(value, country, false);
        let mut p = Phone {
            country,
            input,
            phone: phone.clone(),
            open: false,
            focus: country,
            shown: usize::MAX,
            scroll: 0.0,
            search: String::new(),
            searched: None,
        };
        p.show(country);
        (p, if value.is_empty() { String::new() } else { phone })
    }

    /// The value of the field: Desktop's `phone`.
    pub fn phone(&self) -> &str {
        &self.phone
    }

    /// Desktop's effect on a new `value`: the field takes it, in its country.
    pub fn set(&mut self, value: &str) -> String {
        if value != self.phone {
            let (phone, input, country) = derive(value, self.country, false);
            self.change(phone, input, country);
        }
        self.phone.clone()
    }

    fn change(&mut self, phone: String, input: String, country: usize) {
        (self.phone, self.input) = (phone, input);
        if country != self.country {
            self.country = country;
            self.reset_focus();
        }
    }

    /// `Nf` for a typed edit of the input: a typed character that is not a digit does nothing, unless it is the `+`
    /// of an empty input. Returns the value and the cursor.
    pub fn typed(&mut self, text: &str) -> (String, usize) {
        let (same, inserted) = edit_of(&self.input, text);
        if !inserted.is_empty() && !inserted.chars().all(|c| c.is_ascii_digit()) && text != "+" {
            return (self.phone.clone(), same);
        }
        self.edit(text, same + inserted.chars().count(), inserted.is_empty())
    }

    /// `Nf` for a paste: a pasted number with `+` and a dial code goes to its country.
    pub fn pasted(&mut self, text: &str) -> (String, usize) {
        let (same, inserted) = edit_of(&self.input, text);
        self.edit(text, same + inserted.chars().count(), false)
    }

    fn edit(&mut self, text: &str, at: usize, backward: bool) -> (String, usize) {
        let (phone, input, country) = derive(text, self.country, backward);
        let at = cursor(at, &self.input, text, &input, backward);
        self.change(phone, input, country);
        (self.phone.clone(), at)
    }

    /// The selector button: opens or closes the dropdown. An open dropdown shows at its last scroll.
    pub fn toggle(&mut self) -> Option<f32> {
        self.open = !self.open;
        if self.open {
            return Some(self.scroll);
        }
        self.reset_focus();
        None
    }

    /// `onBlur` and Escape: closes the dropdown.
    pub fn close(&mut self) {
        if std::mem::take(&mut self.open) {
            self.reset_focus();
        }
    }

    /// `setCountry`: the picked country with an empty number.
    pub fn pick(&mut self, i: usize) -> String {
        self.open = false;
        self.show(i);
        self.country = i;
        self.input.clear();
        self.phone = format!("+{}", COUNTRIES[i].dial);
        self.phone.clone()
    }

    pub fn scrolled(&mut self, y: f32) {
        self.scroll = y;
    }

    /// `L`: the dropdown highlights the country again when it last scrolled to another.
    fn reset_focus(&mut self) {
        if self.shown != self.country {
            self.show(self.country);
        }
    }

    /// `A` and `X`: highlights the country of the dropdown and scrolls it into view, like Desktop's `Df`.
    fn show(&mut self, i: usize) {
        self.focus = i;
        if self.shown == i {
            return;
        }
        self.shown = i;
        let top = LIST_PAD + ITEM * i as f32 - self.scroll;
        let below = LIST_HEIGHT - (top + ITEM);
        let max = LIST_PAD * 2.0 + ITEM * COUNTRIES.len() as f32 - LIST_HEIGHT;
        if top < 0.0 || below < 0.0 {
            self.scroll = if top.abs() < below.abs() { self.scroll + top } else { self.scroll - below }.clamp(0.0, max);
        }
    }

    fn moved(&mut self, i: usize) -> KeyOutcome {
        let was = self.scroll;
        self.show(i);
        KeyOutcome::Moved((self.scroll != was).then_some(self.scroll))
    }

    /// The dropdown's `onKeyDown` for a named key.
    pub fn key(&mut self, key: &Key) -> KeyOutcome {
        let last = COUNTRIES.len() - 1;
        match key {
            Key::Named(Named::Enter) => KeyOutcome::Picked(self.focus),
            Key::Named(Named::Escape) => {
                self.close();
                KeyOutcome::Closed
            }
            Key::Named(Named::ArrowUp) => self.moved(self.focus.saturating_sub(1)),
            Key::Named(Named::ArrowDown) => self.moved((self.focus + 1).min(last)),
            Key::Named(Named::PageUp) => self.moved(0),
            Key::Named(Named::PageDown) => self.moved(last),
            _ => KeyOutcome::Ignored,
        }
    }

    /// The dropdown's search: the letters typed within a second of each other go to the first country whose name
    /// starts with them.
    pub fn typed_letters(&mut self, text: &str, now: Instant) -> KeyOutcome {
        let stale = self.searched.is_some_and(|t| now.duration_since(t) > SEARCH_RESET);
        if stale {
            self.search.clear();
        }
        self.search.push_str(&text.to_lowercase());
        self.searched = Some(now);
        match COUNTRIES.iter().position(|c| c.name.to_lowercase().starts_with(&self.search)) {
            Some(i) => self.moved(i),
            None => KeyOutcome::Moved(None),
        }
    }
}

/// The ID of the input of a phone field, and of its dropdown's list.
pub(crate) fn input_id(window: iced::window::Id, field: &str) -> String {
    format!("phone-{window:?}-{field}")
}

pub(crate) fn list_id(window: iced::window::Id, field: &str) -> String {
    format!("phone-list-{window:?}-{field}")
}

impl BeeperGuiApp {
    /// Desktop's `.subview-add-account .login-user-input` phone field: a grid of the selector, 8 px and the input.
    /// The selector is a 32 px `--color-bg-fill-primary` button with the 14 px flag 9 px in and the dial code 27 px in,
    /// and the input is the login form's field.
    pub(crate) fn view_phone<'a>(
        &'a self,
        phone: &'a Phone,
        ids: (String, String),
        placeholder: &str,
        go: impl Fn(PhoneMsg) -> UiMessage + Clone + 'a,
        submit: UiMessage,
    ) -> El<'a> {
        let p = &self.pal;
        let c = &COUNTRIES[phone.country];
        let flag = |c: &Country, size: f32| -> El<'a> {
            container(txt(flag(&c.iso2), size).line_height(1.0).font(EMOJI_FONT)).center(size).into()
        };
        let preview = txt(format!("+{}", c.dial), 12.0).line_height(iced::Pixels(15.0)).color(p.fg);
        let selector = button(container(row![flag(c, 14.0), preview].spacing(4).align_y(Center)).center_y(32))
            .padding(Padding::ZERO.left(9).right(8))
            .style(btn_style([p.fill_primary; 3], [p.fg; 3], 10.0))
            .on_press(go(PhoneMsg::Toggle));
        // The selector's `title`, which a press on it does not show over the dropdown.
        let selector: El<'a> = match phone.open {
            true => selector.into(),
            false => self.tip(selector.into(), c.name.as_str(), vec![], tooltip::Position::Bottom).into(),
        };
        let (fill, fg, hint, active, accent) = (p.fill_primary, p.fg, p.tertiary, p.input_active, p.accent);
        let (typed, pasted) = (go.clone(), go.clone());
        let input = text_input(placeholder, &phone.input)
            .id(ids.0)
            .on_input(move |v| typed(PhoneMsg::Input(v)))
            .on_paste(move |v| pasted(PhoneMsg::Paste(v)))
            .on_submit(submit)
            .size(12)
            .line_height(iced::Pixels(15.0))
            .padding(Padding { top: 8.5, bottom: 8.5, left: 10.0, right: 13.0 })
            .width(Fill)
            .style(move |_, status| text_input::Style {
                background: fill.into(),
                border: Border {
                    color: if matches!(status, text_input::Status::Focused { .. }) { active } else { CLEAR },
                    width: 1.0,
                    radius: 10.0.into(),
                },
                icon: hint,
                placeholder: hint,
                value: fg,
                selection: accent.scale_alpha(0.45),
            });
        let field = row![selector, input].spacing(8).align_y(Center);
        let on_blur = go(PhoneMsg::Blur);
        let list = phone.open.then(|| self.phone_list(phone, ids.1, go));
        Anchored { content: field.into(), popup: list, on_blur }.into()
    }

    /// `zf`: the countries, each with its flag, its name and its dial code, the selected and the highlighted ones on
    /// `whitesmoke`.
    fn phone_list<'a>(&'a self, phone: &'a Phone, id: String, go: impl Fn(PhoneMsg) -> UiMessage + 'a) -> El<'a> {
        let items = COUNTRIES.iter().enumerate().map(|(i, c)| -> El<'a> {
            let dial = format!("+{}", c.dial);
            let room = LIST_WIDTH - GUTTER - 16.0 - 16.0 - 8.0 - 8.0;
            let room = room - crate::style::text_width(&dial, 14.0, Weight::Normal);
            let name = ell(&c.name, room, 14.0, Weight::Normal).color(LIST_TEXT);
            let flag = container(txt(flag(&c.iso2), 16.0).line_height(1.0).font(EMOJI_FONT)).center(16);
            let content = row![flag, name, txt(dial, 14.0).color(LIST_DIAL)].spacing(8).align_y(Center);
            let bg = if i == phone.country || i == phone.focus { LIST_SELECTED } else { CLEAR };
            button(container(content).center_y(ITEM).padding([0, 8]))
                .width(Fill)
                .padding(0)
                .style(btn_style([bg, LIST_SELECTED, LIST_SELECTED], [LIST_TEXT; 3], 0.0))
                .on_press(go(PhoneMsg::Pick(i)))
                .into()
        });
        let list = Column::with_children(items).padding([LIST_PAD, 0.0]);
        let list = scrollable(list)
            .id(id)
            .direction(bar())
            .style(scroll_style(self.pal.scroll))
            .on_scroll(move |v| go(PhoneMsg::Scrolled(v.absolute_offset().y)))
            .height(Fill);
        container(list)
            .width(LIST_WIDTH)
            .height(LIST_HEIGHT)
            .style(|_| container::Style {
                background: Some(LIST_BG.into()),
                border: border::rounded(0),
                shadow: Shadow { color: LIST_SHADOW, offset: Vector::new(2.0, 2.0), blur_radius: 16.0 },
                ..Default::default()
            })
            .into()
    }
}

/// The phone field with its dropdown over the window, `top: 44px` below the selector (the field's first child). A
/// press outside the dropdown and the selector publishes `on_blur` and goes on to what is under it, like a blur.
struct Anchored<'a> {
    content: El<'a>,
    popup: Option<El<'a>>,
    on_blur: UiMessage,
}

impl Widget<UiMessage, Theme, Renderer> for Anchored<'_> {
    fn children(&self) -> Vec<widget::Tree> {
        std::iter::once(&self.content).chain(&self.popup).map(widget::Tree::new).collect()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        let all: Vec<_> = std::iter::once(&self.content).chain(&self.popup).map(|e| e.as_widget()).collect();
        tree.diff_children(&all);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, UiMessage>,
        viewport: &Rectangle,
    ) {
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, UiMessage, Theme, Renderer>> {
        let (content, rest) = tree.children.split_first_mut().expect("the field has a tree");
        let selector = layout.children().next().map_or(layout.bounds(), |l| l.bounds()) + translation;
        let inner = self.content.as_widget_mut().overlay(content, layout, renderer, viewport, translation);
        let popup = self.popup.as_mut().map(|popup| {
            let tree = rest.first_mut().expect("the dropdown has a tree");
            let at = selector.position() + Vector::new(0.0, LIST_TOP);
            let on_blur = self.on_blur.clone();
            overlay::Element::new(Box::new(Dropdown { popup, tree, at, selector, on_blur }))
        });
        let all: Vec<_> = inner.into_iter().chain(popup).collect();
        (!all.is_empty()).then(|| overlay::Group::with_children(all).overlay())
    }
}

struct Dropdown<'a, 'b> {
    popup: &'b mut El<'a>,
    tree: &'b mut widget::Tree,
    at: Point,
    selector: Rectangle,
    on_blur: UiMessage,
}

impl overlay::Overlay<UiMessage, Theme, Renderer> for Dropdown<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.popup.as_widget_mut().layout(self.tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        node.move_to(self.at)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.popup.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &layout.bounds());
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.popup.as_widget_mut().operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, UiMessage>,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
            if !cursor.is_over(layout.bounds()) && !cursor.is_over(self.selector) {
                shell.publish(self.on_blur.clone());
            }
        }
        let viewport = layout.bounds();
        self.popup.as_widget_mut().update(self.tree, event, layout, cursor, renderer, clipboard, shell, &viewport);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.popup.as_widget().mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }
}

impl<'a> From<Anchored<'a>> for El<'a> {
    fn from(a: Anchored<'a>) -> Self {
        Element::new(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(iso2: &str) -> usize {
        index(iso2).expect("a listed country")
    }

    #[test]
    fn the_country_list_is_desktops() {
        assert_eq!(COUNTRIES.len(), 214);
        let au = &COUNTRIES[at("au")];
        assert_eq!((au.name.as_str(), au.dial.as_str(), au.area_codes.as_deref()), ("Australia", "61", Some(&[][..])));
        assert_eq!(flag("us"), "🇺🇸");
        assert_eq!(locale_country("en_US.UTF-8"), "US");
        assert_eq!(locale_country("de_AT"), "AT");
        assert_eq!(locale_country("C.UTF-8"), "");
        assert_eq!(locale_country("en_US@euro"), "");
    }

    /// `Uf` and `_f`: the masks of the countries, and their patterns.
    #[test]
    fn numbers_take_the_country_mask() {
        let fmt = |value: &str, iso2: &str| derive(value, at(iso2), false).1;
        assert_eq!(fmt("2015550123", "us"), "(201) 555-0123");
        assert_eq!(fmt("201", "us"), "(201) ");
        assert_eq!(derive("201", at("us"), true).1, "(201");
        assert_eq!(fmt("20155501234567", "us"), "(201) 555-0123", "the digits past the mask are dropped");
        assert_eq!(fmt("412345678", "au"), "412 345 678");
        assert_eq!(fmt("212345678", "au"), "2 1234 5678");
        assert_eq!(fmt("512345678", "au"), "512 345 678");
        assert_eq!(fmt("550123456", "au"), "5 5012 3456", "`^5(?!50)` does not match 550");
        assert_eq!(fmt("612345678", "nl"), "6 12345678");
        assert_eq!(fmt("123456789012", "af"), "123456789012", "the default mask");
    }

    /// `Vf`: the dial code, the area codes and the priority, and the current country when it fits.
    #[test]
    fn the_country_comes_from_the_digits() {
        let country = |phone: &str, now: &str| {
            let (i, full) = guess(phone, Some(at(now)));
            (i.map(|i| COUNTRIES[i].iso2.as_str()), full)
        };
        assert_eq!(country("+442079460958", "us"), (Some("gb"), true));
        assert_eq!(country("+16045550123", "us"), (Some("ca"), true), "604 is a Canadian area code");
        assert_eq!(country("+12015550123", "ca"), (Some("us"), true), "the lowest priority of +1");
        assert_eq!(country("+1", "ca"), (Some("ca"), true), "the current country fits");
        assert_eq!(country("+78121234567", "kz"), (Some("kz"), true));
        assert_eq!(country("+78121234567", "us"), (Some("kz"), true), "Kazakhstan has priority 0");
        assert!(!country("+35", "us").1, "no whole dial code yet");
        assert_eq!(country("", "us"), (None, false));
    }

    /// `Nf`: typing formats the number and keeps the cursor at its digit, a letter does nothing, and a pasted
    /// international number goes to its country.
    #[test]
    fn typing_and_pasting_follow_desktop() {
        let (mut p, value) = Phone::new("", at("us"));
        assert_eq!((value.as_str(), p.input.as_str()), ("", ""));
        assert_eq!(p.typed("2"), ("+12".into(), 2));
        assert_eq!(p.input, "(2");
        for d in "015550123".chars() {
            let next = format!("{}{d}", p.input);
            p.typed(&next);
        }
        assert_eq!(p.input, "(201) 555-0123");
        assert_eq!(p.typed("(201) 555-0123a"), ("+12015550123".into(), 14), "a letter does nothing");
        assert_eq!(p.typed("(201) 555-012"), ("+1201555012".into(), 13));
        assert_eq!(p.input, "(201) 555-012");
        // Deleting the `)` deletes nothing but it, and backward the mask is trimmed.
        p.set("+1201");
        assert_eq!(p.input, "(201) ");
        assert_eq!(p.typed("(201)"), ("+1201".into(), 4));
        assert_eq!(p.input, "(201");
        assert_eq!(p.pasted("+44 20 7946 0958"), ("+442079460958".into(), 11));
        assert_eq!((COUNTRIES[p.country].iso2.as_str(), p.input.as_str()), ("gb", "2079 460958"));
        assert_eq!(p.pick(at("de")), "+49");
        assert_eq!((p.input.as_str(), p.open), ("", false));
    }

    /// `zf`: the dropdown highlights the selected country, moves with the keys, and searches by the typed letters.
    #[test]
    fn the_dropdown_moves_like_desktop() {
        let (mut p, _) = Phone::new("", at("us"));
        let us = at("us");
        // Desktop's list scrolls the selected country into view, at its bottom.
        assert_eq!(p.scroll, LIST_PAD + ITEM * (us + 1) as f32 - LIST_HEIGHT);
        assert_eq!(p.toggle(), Some(p.scroll));
        assert!(matches!(p.key(&Key::Named(Named::ArrowDown)), KeyOutcome::Moved(Some(_))));
        assert_eq!(p.focus, us + 1);
        assert!(matches!(p.key(&Key::Named(Named::PageUp)), KeyOutcome::Moved(Some(y)) if y == LIST_PAD));
        let now = Instant::now();
        assert!(matches!(p.typed_letters("g", now), KeyOutcome::Moved(Some(_))));
        assert_eq!(COUNTRIES[p.focus].name, "Gabon");
        p.typed_letters("e", now + Duration::from_millis(500));
        assert_eq!(COUNTRIES[p.focus].name, "Georgia");
        p.typed_letters("i", now + Duration::from_millis(2000));
        assert_eq!(COUNTRIES[p.focus].name, "Iceland", "a second later the search starts again");
        assert!(matches!(p.key(&Key::Named(Named::Enter)), KeyOutcome::Picked(i) if i == at("is")));
        assert!(matches!(p.key(&Key::Named(Named::Escape)), KeyOutcome::Closed));
        assert_eq!((p.open, p.focus), (false, us), "a closed dropdown highlights the selected country again");
    }

    /// `_r`: libphonenumber's validity of the phone.
    #[test]
    fn phones_validate_with_libphonenumber() {
        assert!(is_valid("+12015550123") && is_valid("+442079460958"));
        assert!(!is_valid("+1") && !is_valid("+15550100") && !is_valid("+112"));
    }
}
