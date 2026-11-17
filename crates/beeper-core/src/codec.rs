use std::fmt::Write;

pub fn url_encode(input: &str) -> String {
    input.bytes().fold(String::with_capacity(input.len()), |mut s, b| {
        match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            true => s.push(b as char),
            false => write!(s, "%{b:02X}").unwrap(),
        }
        s
    })
}

pub fn url_decode(input: &str) -> Option<String> {
    let (mut out, mut bytes) = (Vec::with_capacity(input.len()), input.as_bytes());
    while let [b, rest @ ..] = bytes {
        bytes = rest;
        out.push(match b {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(..2)?).ok()?;
                bytes = &bytes[2..];
                u8::from_str_radix(hex, 16).ok()?
            }
            b'+' => b' ',
            &b => b,
        });
    }
    String::from_utf8(out).ok()
}

/// A part of a message with one style, from the Matrix HTML subset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
    /// Desktop's `data-mx-spoiler`: hidden until a click shows it.
    pub spoiler: bool,
    pub underline: bool,
    /// The `data-mx-color` (or `data-mx-bg-color`, which Desktop also applies as the text color) of a span, or a
    /// `<font color>`.
    pub color: Option<String>,
}

/// A paragraph or a list item, with its list marker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub marker: Option<String>,
    pub runs: Vec<Run>,
    /// How many blockquotes hold it.
    pub quote: u8,
    /// The level of a heading, `h1` to `h6`; 0 for other blocks.
    pub heading: u8,
}

/// Splits Matrix HTML into blocks of styled runs. Reply and profile fallbacks are removed.
pub fn html_blocks(html: &str) -> Vec<Block> {
    let (mut blocks, mut lists, mut rest) = (vec![Block::default()], Vec::<Option<u32>>::new(), html);
    let (mut quote, mut heading) = (0u8, 0u8);
    let mut stack = vec![(Run::default(), false, false)];
    let push = |b: &mut Block, style: &Run, text: &str| match b.runs.last_mut() {
        _ if text.is_empty() => {}
        Some(r) if Run { text: String::new(), ..r.clone() } == *style => r.text.push_str(text),
        _ => b.runs.push(Run { text: text.into(), ..style.clone() }),
    };
    loop {
        let (text, tail) = rest.split_once('<').unwrap_or((rest, ""));
        let (style, hidden, pre) = stack.last().expect("the root style stays").clone();
        let text = if pre { entities(text) } else { entities(&text.replace('\n', "")) };
        let b = blocks.last_mut().expect("there is a block");
        if !hidden {
            push(b, &style, &text);
        }
        let Some((tag, tail)) = tail.split_once('>') else { break };
        rest = tail;
        let (close, tag) = tag.strip_prefix('/').map_or((false, tag), |t| (true, t));
        let name = tag.split([' ', '\t', '\n', '/']).next().expect("split yields a part").to_ascii_lowercase();
        let level = match name.as_bytes() {
            [b'h', n @ b'1'..=b'6'] => n - b'0',
            _ => 0,
        };
        let block = ["p", "div", "li", "ol", "ul", "blockquote", "pre", "tr"].contains(&name.as_str()) || level > 0;
        if block && !hidden && (!b.runs.is_empty() || b.marker.is_some()) {
            blocks.push(Block::default());
        }
        match (close, name.as_str()) {
            (false, "blockquote") => quote = quote.saturating_add(1),
            (true, "blockquote") => quote = quote.saturating_sub(1),
            _ if level > 0 => heading = if close { 0 } else { level },
            _ => {}
        }
        let b = blocks.last_mut().expect("there is a block");
        if b.runs.is_empty() {
            (b.quote, b.heading) = (quote, heading);
        }
        match (close, name.as_str()) {
            (false, "ol") => lists.push(Some(0)),
            (false, "ul") => lists.push(None),
            (true, "ol" | "ul") => drop(lists.pop()),
            (false, "li") => {
                b.marker = Some(match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{n}.")
                    }
                    _ => "•".into(),
                })
            }
            (false, "br" | "hr") if !hidden => push(b, &style, "\n"),
            (false, "img") if !hidden => push(b, &style, &attr(tag, "alt").unwrap_or_default()),
            _ => {}
        }
        if close {
            if stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        if matches!(name.as_str(), "br" | "img" | "hr") || tag.ends_with('/') {
            continue;
        }
        let mut s = style;
        match name.as_str() {
            "b" | "strong" => s.bold = true,
            "i" | "em" => s.italic = true,
            "code" | "pre" => s.code = true,
            "del" | "s" | "strike" => s.strike = true,
            "a" => s.link = attr(tag, "href"),
            "u" | "ins" => s.underline = true,
            "span" if tag.contains("data-mx-spoiler") => s.spoiler = true,
            "font" => s.color = attr(tag, "color").or(s.color),
            _ => {}
        }
        if name == "span" || name == "font" {
            s.color = attr(tag, "data-mx-bg-color").or_else(|| attr(tag, "data-mx-color")).or(s.color);
        }
        let drop = name == "mx-reply" || tag.contains("data-mx-profile-fallback");
        stack.push((s, hidden || drop, pre || name == "pre"));
    }
    for b in &mut blocks {
        if let Some(r) = b.runs.last_mut().filter(|r| r.text.ends_with('\n')) {
            r.text.pop();
        }
        b.runs.retain(|r| !r.text.is_empty());
    }
    blocks.retain(|b| !b.runs.is_empty());
    blocks
}

/// The plain text of Matrix HTML, with a spoiler as one "█" per character (Desktop's `ep`).
pub fn html_text(html: &str) -> String {
    let text = |r: Run| match r.spoiler {
        true => "█".repeat(match r.text.trim().chars().count() {
            0 => 3,
            n => n,
        }),
        false => r.text,
    };
    let line = |b: Block| b.marker.map(|m| m + " ").into_iter().chain(b.runs.into_iter().map(text)).collect::<String>();
    html_blocks(html).into_iter().map(line).collect::<Vec<_>>().join("\n")
}

/// The text with the five HTML special characters as entities, for text and attribute values.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Converts composer Markdown to Matrix HTML like Desktop: GFM, a line break is a `<br>`, typed HTML stays text,
/// and one paragraph is not wrapped. Gives `None` when the text has no formatting.
pub fn markdown_html(md: &str) -> Option<String> {
    use pulldown_cmark::{html, CowStr, Event, Options, Parser};
    let render = |events: &mut dyn Iterator<Item = Event>| {
        let mut out = String::new();
        html::push_html(&mut out, events);
        out
    };
    let parser = Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    let rich = render(&mut parser.map(|e| match e {
        Event::SoftBreak => Event::HardBreak,
        Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
        e => e,
    }));
    let rich = rich.trim_end();
    let rich =
        rich.strip_prefix("<p>").and_then(|r| r.strip_suffix("</p>")).filter(|r| !r.contains("<p>")).unwrap_or(rich);
    let plain = render(&mut md.split('\n').enumerate().flat_map(|(i, line)| {
        (i > 0).then_some(Event::HardBreak).into_iter().chain([Event::Text(CowStr::Borrowed(line))])
    }));
    (rich != plain.trim_end()).then(|| rich.to_owned())
}

pub fn markdown_links(md: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(md.len()), md);
    while let Some((label, url_end)) = rest.find('[').and_then(|i| {
        let close = i + rest[i..].find("](")?;
        Some((i..close, close + 2 + rest[close + 2..].find(')')?))
    }) {
        out.push_str(&rest[..label.start]);
        out.push_str(&rest[label.start + 1..label.end]);
        rest = &rest[url_end + 1..];
    }
    out + rest
}

/// The lowercase `property` or `name` and the `content` of each `<meta>` tag of a web page, then its `<title>`.
pub fn page_meta(html: &str) -> Vec<(String, String)> {
    let lower = html.to_ascii_lowercase();
    let mut tags: Vec<(String, String)> = lower
        .match_indices("<meta")
        .filter_map(|(i, _)| {
            let tag = &html[i..i + html[i..].find('>')?];
            let key = attr(tag, "property").or_else(|| attr(tag, "name"))?;
            Some((key.to_ascii_lowercase(), attr(tag, "content")?))
        })
        .collect();
    let title = lower.find("<title").and_then(|i| {
        let start = i + html[i..].find('>')? + 1;
        Some(entities(html[start..start + lower[start..].find("</title")?].trim()))
    });
    tags.extend(title.map(|t| ("title".to_string(), t)));
    tags
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let (_, v) = tag.split_once(&format!("{name}="))?;
    let quote = v.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    Some(entities(v[1..].split(quote).next().expect("split yields a part")))
}

fn entities(s: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(s.len()), s);
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let ch = end.and_then(|e| match &rest[1..e] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            n => match n.strip_prefix("#x").or(n.strip_prefix("#X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => n.strip_prefix('#').and_then(|d| d.parse().ok()),
            }
            .and_then(char::from_u32),
        });
        match (ch, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

pub const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode_with(data: &[u8], url: bool) -> String {
    let sym = |v: u32| match (B64[v as usize & 63], url) {
        (b'+', true) => '-',
        (b'/', true) => '_',
        (c, _) => c as char,
    };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(sym(n >> (18 - 6 * i)));
        }
        if !url {
            out.push_str(&"=="[..3 - chunk.len()]);
        }
    }
    out
}

pub fn base64_encode_standard(data: &[u8]) -> String {
    base64_encode_with(data, false)
}

pub fn base64_encode_url_nopad(data: &[u8]) -> String {
    base64_encode_with(data, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_html_formats_like_desktop() {
        for plain in ["hi", "a & b < c", "two\nlines", "5 * 3 = 15", ""] {
            assert_eq!(markdown_html(plain), None, "{plain:?}");
        }
        for (md, html) in [
            ("**b** _i_ ~~s~~ `c`", "<strong>b</strong> <em>i</em> <del>s</del> <code>c</code>"),
            ("[x](https://x.test)", "<a href=\"https://x.test\">x</a>"),
            ("**a**\nb", "<strong>a</strong><br />\nb"),
            ("<b>no</b> **yes**", "&lt;b&gt;no&lt;/b&gt; <strong>yes</strong>"),
            ("- a\n- b", "<ul>\n<li>a</li>\n<li>b</li>\n</ul>"),
            ("> q", "<blockquote>\n<p>q</p>\n</blockquote>"),
        ] {
            assert_eq!(markdown_html(md).as_deref(), Some(html), "{md:?}");
        }
    }

    /// Desktop's `data-mx-spoiler`: a run of its own in the timeline, and "█" per character in plain text.
    #[test]
    fn spoilers_like_desktop() {
        let html = "the end: <span data-mx-spoiler>he dies</span>!";
        let runs = &html_blocks(html)[0].runs;
        assert_eq!(
            runs.iter().map(|r| (r.text.as_str(), r.spoiler)).collect::<Vec<_>>(),
            [("the end: ", false), ("he dies", true), ("!", false)]
        );
        assert_eq!(html_text(html), "the end: ███████!");
        assert_eq!(html_text("<span data-mx-spoiler> </span>"), "███", "an empty spoiler is three blocks");
    }

    #[test]
    fn html_blocks_keep_styles_and_drop_fallbacks() {
        let html = "<mx-reply><blockquote>old</blockquote></mx-reply><strong data-mx-profile-fallback>Bot: </strong>\
                    Invite to <strong>team</strong> &amp; <a href=\"https://x.test/?a=1&amp;b=2\">PR</a><br>\n<code>x&lt;y</code>\
                    <ol><li>one</li><li>two</li></ol><p><br>end<br></p>";
        let blocks = html_blocks(html);
        let run = |text: &str| Run { text: text.into(), ..Run::default() };
        assert_eq!(blocks[0].runs[1], Run { bold: true, ..run("team") });
        assert_eq!(blocks[0].runs[3], Run { link: Some("https://x.test/?a=1&b=2".into()), ..run("PR") });
        assert_eq!(blocks[0].runs[5], Run { code: true, ..run("x<y") });
        assert_eq!(blocks[2], Block { marker: Some("2.".into()), runs: vec![run("two")], ..Block::default() });
        assert_eq!(html_text(html), "Invite to team & PR\nx<y\n1. one\n2. two\n\nend");
    }

    /// Desktop's blockquotes, headings, `<u>` and `data-mx-color` (with `data-mx-bg-color` as the text color too).
    #[test]
    fn html_blocks_keep_quotes_headings_and_colors() {
        let html = "<h2>Title</h2><blockquote><p>quoted</p><blockquote>deeper</blockquote></blockquote>after \
                    <u>under</u> <span data-mx-color=\"#ff0000\">red</span> <font color=\"blue\">blue</font> \
                    <span data-mx-color=\"#ff0000\" data-mx-bg-color=\"#00ff00\">green</span>";
        let blocks = html_blocks(html);
        let shape: Vec<(u8, u8, String)> =
            blocks.iter().map(|b| (b.quote, b.heading, b.runs.iter().map(|r| r.text.as_str()).collect())).collect();
        assert_eq!(shape[..3], [(0, 2, "Title".into()), (1, 0, "quoted".into()), (2, 0, "deeper".into())]);
        let runs = &blocks[3].runs;
        let color = |text: &str| runs.iter().find(|r| r.text == text).and_then(|r| r.color.clone());
        assert!(runs.iter().any(|r| r.text == "under" && r.underline));
        assert_eq!(
            (color("red"), color("blue"), color("green")),
            (Some("#ff0000".into()), Some("blue".into()), Some("#00ff00".into()))
        );
        assert_eq!(blocks[3].quote, 0, "the quote ends");
    }

    #[test]
    fn markdown_links_keep_the_labels() {
        assert_eq!(markdown_links("[https://x.io](https://x.io) see [a](b) [c] d"), "https://x.io see a [c] d");
    }

    #[test]
    fn page_meta_reads_open_graph() {
        let html = "<head><TITLE> A &amp; B </TITLE><meta property=\"og:Title\" content=\"T\">\
                    <META content='D' name=\"description\"/><meta charset=\"utf-8\"></head>";
        let tags = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(page_meta(html), vec![tags("og:title", "T"), tags("description", "D"), tags("title", "A & B")]);
    }
}
