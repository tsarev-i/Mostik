//! Message time formatting in the user's time zone, honouring the language and clock format.

use std::cell::Cell;

use js_sys::Date;
use mostik_core::Timestamp;
use wasm_bindgen::JsValue;

use crate::i18n::Lang;

thread_local! {
    static FORMAT: Cell<(Lang, bool)> = const { Cell::new((Lang::Ru, true)) };
}

/// Language and 24-hour format for all functions of the module; called when the preferences change.
pub fn configure(lang: Lang, clock24: bool) {
    FORMAT.with(|f| f.set((lang, clock24)));
}

fn format() -> (Lang, bool) {
    FORMAT.with(Cell::get)
}

const MONTHS_RU: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];
const WEEKDAYS_RU: [&str; 7] = [
    "воскресенье",
    "понедельник",
    "вторник",
    "среда",
    "четверг",
    "пятница",
    "суббота",
];
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS_EN: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn date(ts: Timestamp) -> Date {
    Date::new(&JsValue::from_f64(ts as f64 / 1000.0))
}

/// `10:42` or `10:42 AM`.
pub fn hm(ts: Timestamp) -> String {
    let d = date(ts);
    let (h, m) = (d.get_hours(), d.get_minutes());
    if format().1 {
        format!("{h:02}:{m:02}")
    } else {
        let suffix = if h < 12 { "AM" } else { "PM" };
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12}:{m:02} {suffix}")
    }
}

/// Calendar day key for the feed separators.
pub fn day_key(ts: Timestamp) -> (u32, u32, u32) {
    let d = date(ts);
    (d.get_full_year(), d.get_month(), d.get_date())
}

enum Day {
    Today,
    Yesterday,
    Other,
}

fn which_day(ts: Timestamp) -> Day {
    let now = Date::new_0();
    let today = (now.get_full_year(), now.get_month(), now.get_date());
    let y = Date::new(&JsValue::from_f64(now.get_time() - 86_400_000.0));
    let yesterday = (y.get_full_year(), y.get_month(), y.get_date());
    match day_key(ts) {
        k if k == today => Day::Today,
        k if k == yesterday => Day::Yesterday,
        _ => Day::Other,
    }
}

/// "Today", "Yesterday", "Monday, November 9th" (and the Russian equivalents).
pub fn day_label(ts: Timestamp) -> String {
    let d = date(ts);
    let lang = format().0;
    let tr = crate::i18n::for_lang(lang);
    match which_day(ts) {
        Day::Today => tr.today.to_owned(),
        Day::Yesterday => tr.yesterday.to_owned(),
        Day::Other => match lang {
            Lang::Ru => format!(
                "{}, {} {}",
                WEEKDAYS_RU[d.get_day() as usize],
                d.get_date(),
                MONTHS_RU[d.get_month() as usize]
            ),
            Lang::En => format!(
                "{}, {} {}{}",
                WEEKDAYS_EN[d.get_day() as usize],
                MONTHS_EN[d.get_month() as usize],
                d.get_date(),
                ordinal(d.get_date())
            ),
        },
    }
}

fn ordinal(n: u32) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// "today at 10:41" (and the Russian equivalent).
pub fn relative(ts: Timestamp) -> String {
    let lang = format().0;
    let day = match (which_day(ts), lang) {
        (Day::Today, Lang::Ru) => "сегодня".to_owned(),
        (Day::Yesterday, Lang::Ru) => "вчера".to_owned(),
        (Day::Today, Lang::En) => "today".to_owned(),
        (Day::Yesterday, Lang::En) => "yesterday".to_owned(),
        (Day::Other, _) => day_label(ts),
    };
    match lang {
        Lang::Ru => format!("{day} в {}", hm(ts)),
        Lang::En => format!("{day} at {}", hm(ts)),
    }
}

/// Short date for lists: the time today, "Yesterday", otherwise "Nov 9" (or the Russian short form).
pub fn short_date(ts: Timestamp) -> String {
    let d = date(ts);
    let lang = format().0;
    match which_day(ts) {
        Day::Today => hm(ts),
        Day::Yesterday => crate::i18n::for_lang(lang).yesterday.to_owned(),
        Day::Other => match lang {
            Lang::Ru => format!(
                "{} {}",
                d.get_date(),
                &MONTHS_RU[d.get_month() as usize].chars().take(3).collect::<String>()
            ),
            Lang::En => format!("{} {}", &MONTHS_EN[d.get_month() as usize][..3], d.get_date()),
        },
    }
}
