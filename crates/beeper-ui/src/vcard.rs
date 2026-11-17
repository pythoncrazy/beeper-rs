//! vCard contact cards: the name of a shared contact, and the parts that its card shows.

use super::*;

/// Desktop's contact name from a vCard file name, without a leading `123.ext.` and the `.vcf`.
pub(crate) fn contact_name(f: &str) -> String {
    let rest = f.split_once('.').filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let rest = rest
        .and_then(|(_, r)| r.split_once('.'))
        .filter(|(w, _)| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    rest.map_or(f, |(_, r)| r).replacen(".vcf", "", 1)
}

/// The parts of a vCard that Desktop's contact card shows.
#[derive(Debug, Default, PartialEq, Clone)]
pub struct VCard {
    pub(crate) name: String,
    pub(crate) org: Option<String>,
    pub(crate) phones: Vec<String>,
    pub(crate) emails: Vec<String>,
}

/// Desktop's `parseVCardAsContact`: unfolds the lines, drops the parameters and groups, and splits each value at the
/// unescaped `;`, or else at the unescaped `,`. The name is `FN`, or else the given and family names of `N`.
pub(crate) fn parse_vcard(text: &str) -> VCard {
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        match lines.last_mut().filter(|_| l.starts_with([' ', '\t'])) {
            Some(last) => last.push_str(l.trim()),
            None => lines.push(l.to_string()),
        }
    }
    let bare = |v: &str, c: char| v.starts_with(c) || v.as_bytes().windows(2).any(|w| w[1] == c as u8 && w[0] != b'\\');
    let split = |v: &str| -> Vec<String> {
        let v = v.replace("\\n", "\n");
        let Some(c) = [';', ','].into_iter().find(|&c| bare(&v, c)) else {
            return vec![v.replace("\\,", ",").replace("\\;", ";")];
        };
        let other = if c == ';' { ',' } else { ';' };
        let v = v.replace(&format!("\\{other}"), &other.to_string()).replace(&format!("\\{c}"), "\0");
        v.split(c).map(|p| p.replace('\0', &c.to_string())).collect()
    };
    let mut fields = HashMap::<String, Vec<Vec<String>>>::new();
    for l in &lines {
        let (key, value) = l.split_once(':').unwrap_or((l, ""));
        let key = key.split(';').next().expect("split gives one part");
        fields.entry(key.split_once('.').map_or(key, |(_, k)| k).to_lowercase()).or_default().push(split(value));
    }
    let first = |k: &str| fields.get(k).and_then(|v| v[0].iter().find(|s| !s.is_empty()).cloned());
    let all = |k: &str| fields.get(k).into_iter().flatten().flatten().filter(|s| !s.is_empty()).cloned().collect();
    let n = fields.get("n").map(|n| n[0].clone()).unwrap_or_default();
    let given = [n.get(1), n.first()].into_iter().flatten().filter(|s| !s.is_empty()).cloned();
    let name = first("fn").unwrap_or_else(|| given.collect::<Vec<_>>().join(" "));
    VCard { name, org: first("org"), phones: all("tel"), emails: all("email") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcards_parse_like_desktop() {
        let card =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Doe;Jo;;;\r\nORG:Acme\\, Inc.\r\nitem1.TEL;type=CELL:+1555\r\n 0100\r\n\
            TEL:+15550100200\r\nEMAIL:jo@x.y\r\nEND:VCARD\r\n";
        let want = VCard {
            name: "Jo Doe".into(),
            org: Some("Acme, Inc.".into()),
            phones: vec!["+15550100".into(), "+15550100200".into()],
            emails: vec!["jo@x.y".into()],
        };
        assert_eq!(parse_vcard(card), want, "Desktop's `parseVCardAsContact`");
        assert_eq!(parse_vcard("FN:Ann\nN:B;C").name, "Ann");
    }
}
