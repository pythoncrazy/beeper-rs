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
}

/// A paragraph or a list item, with its list marker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub marker: Option<String>,
    pub runs: Vec<Run>,
}

/// Splits Matrix HTML into blocks of styled runs. Reply and profile fallbacks are removed.
pub fn html_blocks(html: &str) -> Vec<Block> {
    let (mut blocks, mut lists, mut rest) = (vec![Block::default()], Vec::<Option<u32>>::new(), html);
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
        let block =
            ["p", "div", "li", "ol", "ul", "blockquote", "pre", "tr", "h1", "h2", "h3"].contains(&name.as_str());
        if block && !hidden && (!b.runs.is_empty() || b.marker.is_some()) {
            blocks.push(Block::default());
        }
        let b = blocks.last_mut().expect("there is a block");
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
            _ => {}
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

pub fn html_text(html: &str) -> String {
    let line =
        |b: Block| b.marker.map(|m| m + " ").into_iter().chain(b.runs.into_iter().map(|r| r.text)).collect::<String>();
    html_blocks(html).into_iter().map(line).collect::<Vec<_>>().join("\n")
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
        assert_eq!(blocks[2], Block { marker: Some("2.".into()), runs: vec![run("two")] });
        assert_eq!(html_text(html), "Invite to team & PR\nx<y\n1. one\n2. two\n\nend");
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
