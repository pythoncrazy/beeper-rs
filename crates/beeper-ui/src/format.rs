//! Desktop's composer formatting menu (`BubbleMenu`): Markdown marks on the selection, sent as Matrix HTML.
use crate::popover::above;
use crate::style::{bordered_box, btn_style, carets, fill_box, semi, shadow_box, txt, CLEAR};
use crate::{spell, BeeperGuiApp, El, UiMessage};
use iced::widget::text_editor::{Content, Cursor, Position};
use iced::widget::{button, column, container, row, svg, text_input, tooltip, Row, Space};
use iced::{Border, Center, Color, Fill, Padding, Pixels, Point, Vector};
use std::ops::Range;

pub const LINK: usize = 3;
pub const LABELS: [&str; 9] = [
    "Bold",
    "Italic",
    "Strikethrough",
    "Add Link",
    "Bullet List",
    "Numbered List",
    "Blockquote",
    "Inline Code",
    "Code Block",
];
pub const GROUPS: [u8; 9] = [1, 1, 1, 1, 2, 2, 3, 3, 3];
pub const HOTKEYS: [&[&str]; 9] =
    [&["Ctrl", "B"], &["Ctrl", "I"], &["Ctrl", "Shift", "X"], &[], &[], &[], &[], &[], &[]];
const MARKS: [(&str, &str); 9] =
    [("**", "**"), ("_", "_"), ("~~", "~~"), ("[", "]("), ("", ""), ("", ""), ("", ""), ("`", "`"), ("```\n", "\n```")];

pub static ICONS: std::sync::LazyLock<[iced::widget::svg::Handle; 9]> = std::sync::LazyLock::new(|| {
    [
        (15, "nonzero", "M5.10505 12C4.70805 12 4.4236 11.912 4.25171 11.736C4.0839 11.5559 4 11.2715 4 10.8827V4.11733C4 3.72033 4.08595 3.43588 4.25784 3.26398C4.43383 3.08799 4.71623 3 5.10505 3C6.42741 3 8.25591 3 9.02852 3C10.1373 3 11.0539 3.98153 11.0539 5.1846C11.0539 6.08501 10.6037 6.81855 9.70327 7.23602C10.8657 7.44851 11.5176 8.62787 11.5176 9.48128C11.5176 10.5125 10.9902 12 9.27734 12C8.77742 12 6.42626 12 5.10505 12ZM8.37891 8.00341H5.8V10.631H8.37891C8.9 10.631 9.6296 10.1211 9.6296 9.29877C9.6296 8.47643 8.9 8.00341 8.37891 8.00341ZM5.8 4.36903V6.69577H8.17969C8.53906 6.69577 9.27734 6.35939 9.27734 5.50002C9.27734 4.64064 8.48047 4.36903 8.17969 4.36903H5.8Z"),
        (15, "evenodd", "M5.67494 3.50017C5.67494 3.25164 5.87641 3.05017 6.12494 3.05017H10.6249C10.8735 3.05017 11.0749 3.25164 11.0749 3.50017C11.0749 3.7487 10.8735 3.95017 10.6249 3.95017H9.00587L7.2309 11.05H8.87493C9.12345 11.05 9.32493 11.2515 9.32493 11.5C9.32493 11.7486 9.12345 11.95 8.87493 11.95H4.37493C4.1264 11.95 3.92493 11.7486 3.92493 11.5C3.92493 11.2515 4.1264 11.05 4.37493 11.05H5.99397L7.76894 3.95017H6.12494C5.87641 3.95017 5.67494 3.7487 5.67494 3.50017Z"),
        (24, "nonzero", "M6.85 7.08C6.85 4.37 9.45 3 12.24 3c1.64 0 3 .49 3.9 1.28.77.65 1.46 1.73 1.46 3.24h-3.01c0-.31-.05-.59-.15-.85-.29-.86-1.2-1.28-2.25-1.28-1.86 0-2.34 1.02-2.34 1.7 0 .48.25.88.74 1.21.38.25.77.48 1.41.7H7.39c-.21-.34-.54-.89-.54-1.92zM21 12v-2H3v2h9.62c1.15.45 1.96.75 1.96 1.97 0 1-.81 1.67-2.28 1.67-1.54 0-2.93-.54-2.93-2.51H6.4c0 .55.08 1.13.24 1.58.81 2.29 3.29 3.3 5.67 3.3 2.27 0 5.3-.89 5.3-4.05 0-.3-.01-1.16-.48-1.94H21V12z"),
        (15, "evenodd", "M8.51194 3.00541C9.18829 2.54594 10.0435 2.53694 10.6788 2.95419C10.8231 3.04893 10.9771 3.1993 11.389 3.61119C11.8009 4.02307 11.9513 4.17714 12.046 4.32141C12.4633 4.95675 12.4543 5.81192 11.9948 6.48827C11.8899 6.64264 11.7276 6.80811 11.3006 7.23511L10.6819 7.85383C10.4867 8.04909 10.4867 8.36567 10.6819 8.56093C10.8772 8.7562 11.1938 8.7562 11.389 8.56093L12.0077 7.94221L12.0507 7.89929C12.4203 7.52976 12.6568 7.2933 12.822 7.0502C13.4972 6.05623 13.5321 4.76252 12.8819 3.77248C12.7233 3.53102 12.4922 3.30001 12.1408 2.94871L12.0961 2.90408L12.0515 2.85942C11.7002 2.508 11.4692 2.27689 11.2277 2.11832C10.2377 1.46813 8.94398 1.50299 7.95001 2.17822C7.70691 2.34336 7.47044 2.57991 7.1009 2.94955L7.058 2.99247L6.43928 3.61119C6.24401 3.80645 6.24401 4.12303 6.43928 4.31829C6.63454 4.51355 6.95112 4.51355 7.14638 4.31829L7.7651 3.69957C8.1921 3.27257 8.35757 3.11027 8.51194 3.00541ZM4.31796 7.14672C4.51322 6.95146 4.51322 6.63487 4.31796 6.43961C4.12269 6.24435 3.80611 6.24435 3.61085 6.43961L2.99213 7.05833L2.94922 7.10124C2.57957 7.47077 2.34303 7.70724 2.17788 7.95035C1.50265 8.94432 1.4678 10.238 2.11799 11.2281C2.27656 11.4695 2.50766 11.7005 2.8591 12.0518L2.90374 12.0965L2.94837 12.1411C3.29967 12.4925 3.53068 12.7237 3.77214 12.8822C4.76219 13.5324 6.05589 13.4976 7.04986 12.8223C7.29296 12.6572 7.52943 12.4206 7.89896 12.051L7.89897 12.051L7.94188 12.0081L8.5606 11.3894C8.75586 11.1941 8.75586 10.8775 8.5606 10.6823C8.36533 10.487 8.04875 10.487 7.85349 10.6823L7.23477 11.301C6.80777 11.728 6.6423 11.8903 6.48794 11.9951C5.81158 12.4546 4.95642 12.4636 4.32107 12.0464C4.17681 11.9516 4.02274 11.8012 3.61085 11.3894C3.19896 10.9775 3.0486 10.8234 2.95385 10.6791C2.53661 10.0438 2.54561 9.18863 3.00507 8.51227C3.10993 8.35791 3.27224 8.19244 3.69924 7.76544L4.31796 7.14672ZM9.62172 6.08558C9.81698 5.89032 9.81698 5.57373 9.62172 5.37847C9.42646 5.18321 9.10988 5.18321 8.91461 5.37847L5.37908 8.91401C5.18382 9.10927 5.18382 9.42585 5.37908 9.62111C5.57434 9.81637 5.89092 9.81637 6.08619 9.62111L9.62172 6.08558Z"),
        (15, "evenodd", "M1.5 5.25C1.91421 5.25 2.25 4.91421 2.25 4.5C2.25 4.08579 1.91421 3.75 1.5 3.75C1.08579 3.75 0.75 4.08579 0.75 4.5C0.75 4.91421 1.08579 5.25 1.5 5.25ZM4 4.5C4 4.22386 4.22386 4 4.5 4H13.5C13.7761 4 14 4.22386 14 4.5C14 4.77614 13.7761 5 13.5 5H4.5C4.22386 5 4 4.77614 4 4.5ZM4.5 7C4.22386 7 4 7.22386 4 7.5C4 7.77614 4.22386 8 4.5 8H13.5C13.7761 8 14 7.77614 14 7.5C14 7.22386 13.7761 7 13.5 7H4.5ZM4.5 10C4.22386 10 4 10.2239 4 10.5C4 10.7761 4.22386 11 4.5 11H13.5C13.7761 11 14 10.7761 14 10.5C14 10.2239 13.7761 10 13.5 10H4.5ZM2.25 7.5C2.25 7.91421 1.91421 8.25 1.5 8.25C1.08579 8.25 0.75 7.91421 0.75 7.5C0.75 7.08579 1.08579 6.75 1.5 6.75C1.91421 6.75 2.25 7.08579 2.25 7.5ZM1.5 11.25C1.91421 11.25 2.25 10.9142 2.25 10.5C2.25 10.0858 1.91421 9.75 1.5 9.75C1.08579 9.75 0.75 10.0858 0.75 10.5C0.75 10.9142 1.08579 11.25 1.5 11.25Z"),
        (24, "nonzero", "M3.604 3.089A.75.75 0 0 1 4 3.75V8.5h.75a.75.75 0 0 1 0 1.5h-3a.75.75 0 1 1 0-1.5h.75V5.151l-.334.223a.75.75 0 0 1-.832-1.248l1.5-1a.75.75 0 0 1 .77-.037ZM8.75 5.5a.75.75 0 0 0 0 1.5h11.5a.75.75 0 0 0 0-1.5H8.75Zm0 6a.75.75 0 0 0 0 1.5h11.5a.75.75 0 0 0 0-1.5H8.75Zm0 6a.75.75 0 0 0 0 1.5h11.5a.75.75 0 0 0 0-1.5H8.75ZM5.5 15.75c0-.704-.271-1.286-.72-1.686a2.302 2.302 0 0 0-1.53-.564c-.535 0-1.094.178-1.53.565-.449.399-.72.982-.72 1.685a.75.75 0 0 0 1.5 0c0-.296.104-.464.217-.564A.805.805 0 0 1 3.25 15c.215 0 .406.072.533.185.113.101.217.268.217.565 0 .332-.069.48-.21.657-.092.113-.216.24-.403.419l-.147.14c-.152.144-.33.313-.52.504l-1.5 1.5a.75.75 0 0 0-.22.53v.25c0 .414.336.75.75.75H5A.75.75 0 0 0 5 19H3.31l.47-.47c.176-.176.333-.324.48-.465l.165-.156a5.98 5.98 0 0 0 .536-.566c.358-.447.539-.925.539-1.593Z"),
        (15, "evenodd", "M9.42503 3.44136C10.0561 3.23654 10.7837 3.2402 11.3792 3.54623C12.7532 4.25224 13.3477 6.07191 12.7946 8C12.5465 8.8649 12.1102 9.70472 11.1861 10.5524C10.262 11.4 8.98034 11.9 8.38571 11.9C8.17269 11.9 8 11.7321 8 11.525C8 11.3179 8.17644 11.15 8.38571 11.15C9.06497 11.15 9.67189 10.7804 10.3906 10.236C10.9406 9.8193 11.3701 9.28633 11.608 8.82191C12.0628 7.93367 12.0782 6.68174 11.3433 6.34901C10.9904 6.73455 10.5295 6.95946 9.97725 6.95946C8.7773 6.95946 8.0701 5.99412 8.10051 5.12009C8.12957 4.28474 8.66032 3.68954 9.42503 3.44136ZM3.42503 3.44136C4.05614 3.23654 4.78366 3.2402 5.37923 3.54623C6.7532 4.25224 7.34766 6.07191 6.79462 8C6.54654 8.8649 6.11019 9.70472 5.1861 10.5524C4.26201 11.4 2.98034 11.9 2.38571 11.9C2.17269 11.9 2 11.7321 2 11.525C2 11.3179 2.17644 11.15 2.38571 11.15C3.06497 11.15 3.67189 10.7804 4.39058 10.236C4.94065 9.8193 5.37014 9.28633 5.60797 8.82191C6.06282 7.93367 6.07821 6.68174 5.3433 6.34901C4.99037 6.73455 4.52948 6.95946 3.97725 6.95946C2.7773 6.95946 2.0701 5.99412 2.10051 5.12009C2.12957 4.28474 2.66032 3.68954 3.42503 3.44136Z"),
        (15, "evenodd", "M9.96424 2.68571C10.0668 2.42931 9.94209 2.13833 9.6857 2.03577C9.4293 1.93322 9.13832 2.05792 9.03576 2.31432L5.03576 12.3143C4.9332 12.5707 5.05791 12.8617 5.3143 12.9642C5.5707 13.0668 5.86168 12.9421 5.96424 12.6857L9.96424 2.68571ZM3.85355 5.14646C4.04882 5.34172 4.04882 5.6583 3.85355 5.85356L2.20711 7.50001L3.85355 9.14646C4.04882 9.34172 4.04882 9.6583 3.85355 9.85356C3.65829 10.0488 3.34171 10.0488 3.14645 9.85356L1.14645 7.85356C0.951184 7.6583 0.951184 7.34172 1.14645 7.14646L3.14645 5.14646C3.34171 4.9512 3.65829 4.9512 3.85355 5.14646ZM11.1464 5.14646C11.3417 4.9512 11.6583 4.9512 11.8536 5.14646L13.8536 7.14646C14.0488 7.34172 14.0488 7.6583 13.8536 7.85356L11.8536 9.85356C11.6583 10.0488 11.3417 10.0488 11.1464 9.85356C10.9512 9.6583 10.9512 9.34172 11.1464 9.14646L12.7929 7.50001L11.1464 5.85356C10.9512 5.6583 10.9512 5.34172 11.1464 5.14646Z"),
        (256, "nonzero", "M58.34,101.66l-32-32a8,8,0,0,1,0-11.32l32-32A8,8,0,0,1,69.66,37.66L43.31,64,69.66,90.34a8,8,0,0,1-11.32,11.32Zm40,0a8,8,0,0,0,11.32,0l32-32a8,8,0,0,0,0-11.32l-32-32A8,8,0,0,0,98.34,37.66L124.69,64,98.34,90.34A8,8,0,0,0,98.34,101.66ZM200,40H176a8,8,0,0,0,0,16h24V200H56V136a8,8,0,0,0-16,0v64a16,16,0,0,0,16,16H200a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Z"),
    ]
    .map(|(view, rule, d)| {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view} {view}"><path fill="currentColor" fill-rule="{rule}" d="{d}"/></svg>"#
        );
        iced::widget::svg::Handle::from_memory(svg.into_bytes())
    })
});

pub fn offset(text: &str, p: Position) -> usize {
    text.split('\n').take(p.line).map(|l| l.len() + 1).sum::<usize>() + p.column
}

/// The composer selection as a byte range of its text.
pub fn selection(c: &Content) -> Option<Range<usize>> {
    let sel = c.selection().filter(|s| !s.is_empty())?;
    let text = c.text();
    let Cursor { position, selection } = c.cursor();
    let (a, b) = (offset(&text, position), offset(&text, selection.expect("a selection has an anchor")));
    let (lo, hi) = (a.min(b), a.max(b));
    let start = text.match_indices(sel.as_str()).map(|(i, _)| i).find(|&i| i <= lo && i + sel.len() >= hi);
    let start = start.expect("the selected text covers both cursors");
    Some(start..start + sel.len())
}

pub fn position(text: &str, at: usize) -> Position {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    Position { line: text[..at].matches('\n').count(), column: at - line_start }
}

/// A composer with `text` and the selection `sel`; an empty selection is the caret.
pub fn content(text: &str, sel: Range<usize>) -> Content {
    let mut c = Content::with_text(text);
    let anchor = (!sel.is_empty()).then(|| position(text, sel.start));
    c.move_to(Cursor { position: position(text, sel.end), selection: anchor });
    c
}

fn lines(text: &str, r: &Range<usize>) -> Range<usize> {
    let start = text[..r.start].rfind('\n').map_or(0, |i| i + 1);
    start..r.end + text[r.end..].find('\n').unwrap_or(text.len() - r.end)
}

fn unprefixed(i: usize, line: &str) -> Option<&str> {
    match i {
        4 => line.strip_prefix("- "),
        5 => line.split_once(". ").filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).map(|l| l.1),
        _ => line.strip_prefix("> ").or_else(|| line.strip_prefix('>')),
    }
}

/// Whether button `i` is on for the selection `r`, like Desktop's highlighted buttons.
pub fn active(i: usize, text: &str, r: &Range<usize>) -> bool {
    match i {
        4..=6 => text[lines(text, r)].split('\n').all(|l| unprefixed(i, l).is_some()),
        _ => text[..r.start].ends_with(MARKS[i].0) && text[r.end..].starts_with(MARKS[i].1),
    }
}

/// Toggles button `i` on the selection `r`; gives the new text and selection.
pub fn toggle(i: usize, text: &str, r: Range<usize>) -> (String, Range<usize>) {
    let on = active(i, text, &r);
    if (4..=6).contains(&i) {
        let l = lines(text, &r);
        let edit = |(n, line): (usize, &str)| match on {
            true => unprefixed(i, line).expect("active lines have the prefix").to_owned(),
            false => format!("{}{line}", [String::from("- "), format!("{}. ", n + 1), "> ".into()][i - 4]),
        };
        let body = text[l.clone()].split('\n').enumerate().map(edit).collect::<Vec<_>>().join("\n");
        return (format!("{}{body}{}", &text[..l.start], &text[l.end..]), l.start..l.start + body.len());
    }
    let (open, close) = MARKS[i];
    match on {
        true => {
            let end = if i == LINK { text[r.end..].find(')').expect("a link ends with )") + 1 } else { close.len() };
            let out = format!("{}{}{}", &text[..r.start - open.len()], &text[r.clone()], &text[r.end + end..]);
            (out, r.start - open.len()..r.end - open.len())
        }
        false => {
            assert_ne!(i, LINK, "the link dialog adds links");
            let out = format!("{}{open}{}{close}{}", &text[..r.start], &text[r.clone()], &text[r.end..]);
            (out, r.start + open.len()..r.end + open.len())
        }
    }
}

/// Replaces the selection `r` with a Markdown link; the caret goes after it like Desktop.
pub fn link(text: &str, r: Range<usize>, url: &str, label: &str) -> (String, Range<usize>) {
    let md = format!("[{}]({url})", if label.is_empty() { url } else { label });
    let end = r.start + md.len();
    (format!("{}{md}{}", &text[..r.start], &text[r.end..]), end..end)
}

impl BeeperGuiApp {
    /// The composer with Desktop's bubble menu or link dialog centered above the selection, 8px over its first line.
    pub(crate) fn with_formatting<'a>(&'a self, editor: El<'a>, formats: u16) -> El<'a> {
        let text = self.pane().composer.text();
        let r = selection(&self.pane().composer).filter(|_| formats != 0);
        let popup = r.clone().map(|r| match &self.pane().link_dialog {
            Some((url, label)) => self.view_link_dialog(url, label),
            None => self.view_bubble(&text, &r, formats),
        });
        let r = r.unwrap_or_default();
        let caret = offset(&text, self.pane().composer.cursor().position);
        let typos = spell::misspelled(&text, caret);
        let text: std::rc::Rc<str> = text.into();
        let at = |text: &str, r: &Range<usize>| [r.start, r.end].map(|o| position(text, o)).map(|p| (p.line, p.column));
        let shown = text.clone();
        above(editor, popup, move |size| {
            let &[(x0, row), (x1, _)] = &carets(&text, 13.0, size.width - 4.0, at(&text, &r))[..] else {
                unreachable!()
            };
            Vector::new(2.0 + (x0 + x1) / 2.0, 9.0 + row as f32 * 13.0 * 1.3 - 8.0)
        })
        .squiggles(move |size| {
            let ends = carets(&shown, 13.0, size.width - 4.0, typos.iter().flat_map(|r| at(&shown, r)));
            let words = ends.chunks(2).filter(|w| w[0].1 == w[1].1);
            words
                .map(|w| (Point::new(2.0 + w[0].0, 9.0 + (w[0].1 as f32 + 1.0) * 13.0 * 1.3 - 3.0), w[1].0 - w[0].0))
                .collect()
        })
        .into()
    }

    fn view_bubble(&self, text: &str, r: &Range<usize>, formats: u16) -> El<'_> {
        let p = &self.pal;
        let (mut items, mut group) = (Row::new().spacing(4).align_y(Center), None);
        for i in (0..LABELS.len()).filter(|i| formats & 1 << i != 0) {
            if group.is_some_and(|g| g != GROUPS[i]) {
                items = items.push(container(Space::new()).width(1).height(16).style(fill_box(p.prefs_border, 0.0)));
            }
            group = Some(GROUPS[i]);
            let on = active(i, text, r);
            let label = if i == LINK && on { "Remove Link" } else { LABELS[i] };
            let (bg, fg) = if on { (p.accent, Color::WHITE) } else { (CLEAR, p.strong) };
            let icon = svg(ICONS[i].clone()).width(16).height(16).style(move |_, _| svg::Style { color: Some(fg) });
            let b = button(icon).padding(4).style(btn_style([bg; 3], [fg; 3], 8.0)).on_press(UiMessage::Format(i));
            items =
                items.push(self.tip(b.into(), label, HOTKEYS[i].to_vec(), tooltip::Position::Top).padding(0).gap(6));
        }
        container(items).padding(4).style(bordered_box(p.input, p.prefs_border, 1.0, 8.0)).into()
    }

    fn view_link_dialog(&self, url: &str, label: &str) -> El<'_> {
        let p = &self.pal;
        let save = (!url.trim().is_empty()).then_some(UiMessage::SaveLink);
        let input = |id: &'static str, hint: &'static str, value: &str| {
            text_input(hint, value)
                .id(id)
                .size(10)
                .line_height(Pixels(12.0))
                .padding([6, 8])
                .on_submit_maybe(save.clone())
                .style(move |_, _| text_input::Style {
                    background: p.chip.into(),
                    border: Border { color: p.surface_dim, width: 1.0, radius: 8.0.into() },
                    icon: p.sub,
                    placeholder: p.on_secondary,
                    value: p.strong,
                    selection: p.accent.scale_alpha(0.45),
                })
        };
        let field = |name: &'static str, input| {
            column![txt(name, 11.0).line_height(Pixels(14.0)).color(p.sub), input].spacing(2)
        };
        let (u, l) = (url.to_owned(), label.to_owned());
        let url_in = input("link-url", "https://example.com", url)
            .on_input(move |u| UiMessage::LinkDialog(Some((u, l.clone()))));
        let text_in = input("link-text", "Link text (optional)", label)
            .on_input(move |l| UiMessage::LinkDialog(Some((u.clone(), l))));
        let btn = |name: &'static str, bg: Color, fg: Color, msg: Option<UiMessage>| {
            button(semi(name, 10.0).line_height(Pixels(12.0)))
                .padding([4, 10])
                .style(btn_style([bg; 3], [fg; 3], 8.0))
                .on_press_maybe(msg)
        };
        let buttons = row![
            Space::new().width(Fill),
            btn("Cancel", CLEAR, p.strong, Some(UiMessage::LinkDialog(None))),
            btn("Save", p.accent, Color::WHITE, save.clone())
        ];
        let body =
            column![field("URL", url_in), field("Text", text_in), buttons.spacing(6).padding(Padding::ZERO.top(2.0))];
        container(body.spacing(6)).padding(8).width(220).style(shadow_box(p.card, CLEAR, 8.0, 20.0)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_markdown_like_desktop_marks() {
        let (t, r) = toggle(0, "say hi now", 4..6);
        assert_eq!((t.as_str(), r.clone()), ("say **hi** now", 6..8));
        assert!(active(0, &t, &r) && !active(1, &t, &r));
        assert_eq!(toggle(0, &t, r), ("say hi now".into(), 4..6));
        assert_eq!(toggle(8, "a", 0..1), ("```\na\n```".into(), 4..5));
        assert_eq!(toggle(0, "", 0..0), ("****".into(), 2..2));
        let (t, r) = toggle(5, "x\ny\nz", 1..3);
        assert_eq!((t.as_str(), r.clone()), ("1. x\n2. y\nz", 0..9));
        assert_eq!(toggle(5, &t, r), ("x\ny\nz".into(), 0..3));
        let (t, r) = link("see this", 4..8, "https://x.test", "this");
        assert_eq!((t.as_str(), r), ("see [this](https://x.test)", 26..26));
        assert!(active(LINK, &t, &(5..9)));
        assert_eq!(toggle(LINK, &t, 5..9), ("see this".into(), 4..8));
    }

    #[test]
    fn selection_round_trips_through_the_editor() {
        let c = content("one\ntwo three", 8..13);
        assert_eq!((selection(&c), c.selection().as_deref()), (Some(8..13), Some("three")));
    }
}
