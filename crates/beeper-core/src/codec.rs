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
}
