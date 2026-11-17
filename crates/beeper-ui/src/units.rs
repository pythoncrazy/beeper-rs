//! Sizes and counts as Desktop shows them.

pub(crate) fn human_size(n: u64) -> String {
    let i = (n.max(1) as f64).log(1024.0).floor() as i32;
    let v = format!("{:.1}", n as f64 / 1024f64.powi(i));
    format!("{} {}", v.trim_end_matches(".0"), ["B", "KB", "MB", "GB", "TB"][i as usize])
}

/// `Intl.NumberFormat` with `notation: "compact"`, like Desktop's message counts.
pub(crate) fn compact(n: u64) -> String {
    let Some((d, unit)) = [(1e9, "B"), (1e6, "M"), (1e3, "K")].into_iter().find(|&(d, _)| n as f64 >= d) else {
        return n.to_string();
    };
    let v = n as f64 / d;
    let v = if v < 10.0 { format!("{v:.1}").trim_end_matches(".0").to_string() } else { format!("{v:.0}") };
    format!("{v}{unit}")
}

/// `toLocaleString` of a count, with thousands separators.
pub(crate) fn grouped(n: u64) -> String {
    let s = n.to_string();
    let digits: Vec<String> = s.as_bytes().rchunks(3).rev().map(|c| String::from_utf8_lossy(c).into()).collect();
    digits.join(",")
}
