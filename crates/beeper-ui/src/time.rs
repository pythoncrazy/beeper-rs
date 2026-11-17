//! Dates and times as Desktop shows them: the clock, the day labels of the timeline and the chat list, and timers.

use super::*;

pub(crate) const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

pub(crate) fn iso_millis(iso: &str) -> i64 {
    millis_from_rfc3339(iso).unwrap_or_else(|| panic!("timestamp {iso} is not RFC 3339"))
}

pub fn format_short_ts(iso: &str, twenty_four: bool) -> String {
    clock(iso_millis(iso), twenty_four)
}

/// date-fns `h:mm a` or `HH:mm`, which Desktop formats in English.
pub(crate) fn clock(ms: i64, twenty_four: bool) -> String {
    let (f, hm) = clock_format(ms, twenty_four);
    i18n::en_date(f, (0, 1, 1), hm)
}

/// Desktop's `Cp`: the time in the app language.
pub(crate) fn local_clock(ms: i64, twenty_four: bool) -> String {
    let (f, hm) = clock_format(ms, twenty_four);
    i18n::date(f, (0, 1, 1), hm)
}

pub(crate) fn clock_format(ms: i64, twenty_four: bool) -> (i18n::Date, (i64, i64)) {
    let mins = local_millis(ms).div_euclid(60_000).rem_euclid(1440);
    let f = match (twenty_four, mins < 720) {
        (true, _) => i18n::Date::Clock24,
        (false, true) => i18n::Date::Am,
        (false, false) => i18n::Date::Pm,
    };
    (f, (mins / 60, mins % 60))
}

/// The en-US `Date.toLocaleString()` of a timestamp, like "10/16/2026, 10:40:56 PM".
pub(crate) fn locale_string(iso: &str) -> String {
    let ms = local_millis(iso_millis(iso));
    let (y, mo, d) = civil_from_days(ms.div_euclid(86_400_000));
    let s = ms.div_euclid(1000).rem_euclid(86_400);
    let (h, ampm) = (s / 3600, ["AM", "PM"][usize::from(s >= 43_200)]);
    format!("{mo}/{d}/{y}, {}:{:02}:{:02} {ampm}", (h + 11) % 12 + 1, s / 60 % 60, s % 60)
}

pub(crate) fn day_num(iso: &str) -> i64 {
    day_of(iso_millis(iso))
}

pub(crate) fn day_of(ms: i64) -> i64 {
    local_millis(ms).div_euclid(86_400_000)
}

pub(crate) fn today() -> i64 {
    day_of(now_unix_millis())
}

pub(crate) fn day_label(iso: &str) -> String {
    let (d, today) = (day_num(iso), today());
    let wd = WEEKDAYS[(d + 4).rem_euclid(7) as usize];
    match today - d {
        0 => tr("Today").into(),
        1 => tr("Yesterday").into(),
        2..=6 => tr(wd).into(),
        _ => {
            let ((y, m, dd), ty) = (civil_from_days(d), civil_from_days(today).0);
            let year = if y == ty { String::new() } else { format!(", {y}") };
            format!("{}, {} {dd}{year}", &wd[..3], i18n::en_short_month(m))
        }
    }
}

pub(crate) fn list_ts(iso: &str, h24: bool) -> String {
    const DAY: i64 = 86_400_000;
    let d = day_num(iso);
    let age = now_unix_millis() - millis_from_rfc3339(iso).expect("RFC 3339 timestamp");
    match today() - d {
        0 => local_clock(iso_millis(iso), h24),
        1 => tr("Yesterday").into(),
        _ if age < 7 * DAY => tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]).into(),
        _ if age < 365 * DAY => i18n::date(i18n::Date::MonthDay, civil_from_days(d), (0, 0)),
        _ => i18n::date(i18n::Date::Medium, civil_from_days(d), (0, 0)),
    }
}

/// Desktop's `KJ`: the timer, or "Off".
pub(crate) fn timer_label(ms: u64) -> String {
    if ms == 0 {
        trx("Off", "disappearing messages timer option").into()
    } else {
        timer_distance(ms)
    }
}
