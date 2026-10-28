//! Numbers in the language of the process (a decimal comma and КБ in Russian, a point and KB
//! in English) and colors that only appear in a terminal.

use std::io::IsTerminal;
use std::time::Duration;

use tsubamedo_core::lang::english;
use tsubamedo_core::tr;

/// `1.5` as the language writes it: `1,5` in Russian.
fn decimal(text: String) -> String {
    if english() { text } else { text.replace('.', ",") }
}

pub fn size(bytes: u64) -> String {
    let units: [&str; 4] = if english() { ["KB", "MB", "GB", "TB"] } else { ["КБ", "МБ", "ГБ", "ТБ"] };
    if bytes < 1024 {
        return tr!("{bytes} Б", "{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    let text = if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
    format!("{} {}", decimal(text), units[unit])
}

pub fn duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        tr!("{} мс", "{} ms", decimal(format!("{ms:.1}")))
    } else if ms < 1000.0 {
        tr!("{ms:.0} мс", "{ms:.0} ms")
    } else {
        tr!("{} с", "{} s", decimal(format!("{:.2}", ms / 1000.0)))
    }
}

/// The form of the word for the number: Russian has three (1 заголовок, 2 заголовка,
/// 5 заголовков), English two (1 header, 5 headers).
pub fn plural<'a>(n: usize, ru: [&'a str; 3], en: [&'a str; 2]) -> &'a str {
    if english() {
        return if n == 1 { en[0] } else { en[1] };
    }
    let [one, few, many] = ru;
    match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    }
}

pub struct Paint(bool);

impl Paint {
    pub fn stdout() -> Self {
        Paint(std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }

    pub fn stderr() -> Self {
        Paint(std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.0 { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap("31", text)
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    pub fn yellow(&self, text: &str) -> String {
        self.wrap("33", text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    pub fn status(&self, status: u16, text: &str) -> String {
        match status {
            200..=299 => self.wrap("1;32", text),
            300..=399 => self.wrap("1;36", text),
            400..=499 => self.wrap("1;33", text),
            _ => self.wrap("1;31", text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_formats() {
        assert_eq!(size(512), "512 Б");
        assert_eq!(size(12_600), "12 КБ");
        assert_eq!(size(1_500), "1,5 КБ");
        assert_eq!(size(150 << 20), "150 МБ");
        assert_eq!(duration(Duration::from_micros(400)), "0,4 мс");
        assert_eq!(duration(Duration::from_millis(172)), "172 мс");
        assert_eq!(duration(Duration::from_millis(1234)), "1,23 с");
        assert_eq!(plural(22, ["изменение", "изменения", "изменений"], ["change", "changes"]), "изменения");
    }

    #[test]
    fn english_formats() {
        tsubamedo_core::lang::in_language(true, || {
            assert_eq!(size(512), "512 B");
            assert_eq!(size(1_500), "1.5 KB");
            assert_eq!(size(150 << 20), "150 MB");
            assert_eq!(duration(Duration::from_micros(400)), "0.4 ms");
            assert_eq!(duration(Duration::from_millis(1234)), "1.23 s");
            assert_eq!(plural(1, ["изменение", "изменения", "изменений"], ["change", "changes"]), "change");
            assert_eq!(plural(21, ["изменение", "изменения", "изменений"], ["change", "changes"]), "changes");
        });
    }
}
