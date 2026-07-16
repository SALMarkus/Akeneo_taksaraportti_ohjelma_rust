//! Päivämääräapurit. Vastaa Python-version `define_the_next_*`-funktioita.

use chrono::{Datelike, Local, NaiveDate};

/// Seuraava taksan voimaantulopäivä: kuukauden 15. päivä tai seuraavan
/// kuukauden 1. päivä.
fn next_1st_or_15th_day() -> NaiveDate {
    let today = Local::now().date_naive();
    if today.day() >= 15 {
        // Python: next_month = (month % 12) + 1; year = year + (month // 12)
        let next_month = (today.month() % 12) + 1;
        let year = today.year() + (today.month() / 12) as i32;
        NaiveDate::from_ymd_opt(year, next_month, 1).expect("kelvollinen päivämäärä")
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month(), 15).expect("kelvollinen päivämäärä")
    }
}

/// Muoto YYYYMMDD, käytetään tiedostonimissä.
pub fn next_1st_or_15th_day_compact() -> String {
    next_1st_or_15th_day().format("%Y%m%d").to_string()
}

/// Muoto DD.MM.YYYY, käytetään PDF:n sivuotsikossa.
pub fn next_1st_or_15th_day_fi() -> String {
    next_1st_or_15th_day().format("%d.%m.%Y").to_string()
}

/// Python-versiossa määritelty mutta käyttämätön funktio. Säilytetty
/// käännöksen kattavuuden vuoksi.
#[allow(dead_code)]
pub fn current_timestamp_for_filename() -> String {
    Local::now().format("%Y_%m_%d_%H_%M_%S").to_string()
}
