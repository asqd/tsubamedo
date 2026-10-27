//! Russian number formats and colors that only appear in a terminal.

use std::io::IsTerminal;
use std::time::Duration;

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["КБ", "МБ", "ГБ", "ТБ"];
    if bytes < 1024 {
        return format!("{bytes} Б");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let text = if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
    format!("{} {}", text.replace('.', ","), UNITS[unit])
}

pub fn duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{} мс", format!("{ms:.1}").replace('.', ","))
    } else if ms < 1000.0 {
        format!("{ms:.0} мс")
    } else {
        format!("{} с", format!("{:.2}", ms / 1000.0).replace('.', ","))
    }
}

/// The Russian form for the number: 1 заголовок, 2 заголовка, 5 заголовков.
pub fn plural<'a>(n: usize, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
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
    }
}
