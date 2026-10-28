//! Cookies the servers set, sent back the way browsers do (RFC 6265): by domain and path,
//! `Secure` only over HTTPS (and to this machine), until they expire. A jar per environment:
//! the session keeps it in the app's data folder, the CLI for one run.
//!
//! Cookies without an expiry are kept until the jar is cleared: for an API client "until the
//! browser closes" means nothing.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use url::{Host, Url};

/// More than this and the oldest go: a jar is not a database.
const MAX_COOKIES: usize = 3000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lowercase, without the leading dot.
    pub domain: String,
    /// Only for exactly this host, not its subdomains: the server gave no `Domain`.
    pub host_only: bool,
    pub path: String,
    /// Unix seconds; `None` keeps it until the jar is cleared.
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
    /// Unix milliseconds: of two cookies with the same path, the older goes first.
    pub created: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Jar {
    cookies: Vec<Cookie>,
    /// Something changed since the owner last saved it.
    #[serde(skip)]
    changed: bool,
}

/// One jar for requests going side by side.
pub type SharedJar = Arc<Mutex<Jar>>;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Jar {
    pub fn shared(self) -> SharedJar {
        Arc::new(Mutex::new(self))
    }

    /// Those not expired yet, by domain, path and name.
    pub fn cookies(&self) -> Vec<Cookie> {
        let now = (now_ms() / 1000) as i64;
        let mut list: Vec<Cookie> = self.cookies.iter().filter(|c| c.expires.is_none_or(|e| e > now)).cloned().collect();
        list.sort_by(|a, b| (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name)));
        list
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Whether it changed since the last call: the owner saves it then.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// A `Set-Cookie` header that came from `url`.
    pub fn store(&mut self, url: &Url, set_cookie: &str) {
        self.store_at(url, set_cookie, now_ms());
    }

    /// The `Cookie` header for a request to `url`, if any cookie goes there.
    pub fn header(&mut self, url: &Url) -> Option<String> {
        self.header_at(url, now_ms())
    }

    pub fn remove(&mut self, domain: &str, path: &str, name: &str) -> bool {
        let before = self.cookies.len();
        self.cookies.retain(|c| !(c.domain == domain && c.path == path && c.name == name));
        let removed = self.cookies.len() != before;
        self.changed |= removed;
        removed
    }

    pub fn clear(&mut self) {
        self.changed |= !self.cookies.is_empty();
        self.cookies.clear();
    }

    fn store_at(&mut self, url: &Url, set_cookie: &str, now: u64) {
        let Some(host) = url.host() else { return };
        let host_name = host_name(&host);
        let Some(mut cookie) = parse(set_cookie, url, now) else { return };
        match &cookie.domain {
            // No Domain: this host only.
            d if d.is_empty() => {
                cookie.domain = host_name.clone();
                cookie.host_only = true;
            }
            d if *d == host_name => {
                // A name without dots is a top-level domain, unless it is the host itself
                // (`localhost`): then it is this host only.
                cookie.host_only = !d.contains('.');
            }
            d => {
                // A server sets cookies for itself and its parent domains, never for another
                // one, never for a top-level domain, and an address has no parents.
                if !matches!(host, Host::Domain(_)) || !d.contains('.') || !host_name.ends_with(&format!(".{d}")) {
                    return;
                }
            }
        }
        let old = self.cookies.iter().position(|c| c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path);
        if let Some(i) = old {
            cookie.created = self.cookies[i].created;
            self.cookies.remove(i);
            self.changed = true;
        }
        // An expiry in the past is how a server deletes a cookie.
        if cookie.expires.is_some_and(|e| e <= (now / 1000) as i64) {
            return;
        }
        self.cookies.push(cookie);
        self.changed = true;
        if self.cookies.len() > MAX_COOKIES
            && let Some(oldest) = self.cookies.iter().enumerate().min_by_key(|(_, c)| c.created).map(|(i, _)| i)
        {
            self.cookies.remove(oldest);
        }
    }

    /// The cookies a request to `url` carries, in the order they go: for scripts.
    pub fn for_url(&mut self, url: &Url) -> Vec<Cookie> {
        self.for_url_at(url, now_ms())
    }

    fn header_at(&mut self, url: &Url, now: u64) -> Option<String> {
        let matching = self.for_url_at(url, now);
        (!matching.is_empty()).then(|| matching.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    fn for_url_at(&mut self, url: &Url, now: u64) -> Vec<Cookie> {
        let Some(host) = url.host() else { return Vec::new() };
        let host_name = host_name(&host);
        let now_s = (now / 1000) as i64;
        let expired = self.cookies.len();
        self.cookies.retain(|c| c.expires.is_none_or(|e| e > now_s));
        self.changed |= self.cookies.len() != expired;
        // Browsers treat this machine as secure even over plain HTTP.
        let secure = url.scheme() == "https" || is_loopback(&host);
        let path = url.path();
        let mut matching: Vec<Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                let domain_ok = if c.host_only {
                    host_name == c.domain
                } else {
                    host_name == c.domain || (matches!(host, Host::Domain(_)) && host_name.ends_with(&format!(".{}", c.domain)))
                };
                domain_ok && path_matches(path, &c.path) && (secure || !c.secure)
            })
            .cloned()
            .collect();
        // Longer paths first, then the older: what servers expect when names repeat.
        matching.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.created.cmp(&b.created)));
        matching
    }
}

/// The `Cookie` header with the user's own cookies and those from the jar: the user's win
/// when the names are the same.
pub fn merge(own: Option<&str>, from_jar: Option<&str>) -> Option<String> {
    let own = own.map(str::trim).filter(|o| !o.is_empty());
    let Some(jar) = from_jar else { return own.map(str::to_string) };
    let Some(own) = own else { return Some(jar.to_string()) };
    let names: Vec<&str> = own.split(';').map(|p| p.split('=').next().unwrap_or_default().trim()).collect();
    let rest: Vec<&str> = jar.split("; ").filter(|p| !names.contains(&p.split('=').next().unwrap_or_default())).collect();
    Some(if rest.is_empty() { own.to_string() } else { format!("{own}; {}", rest.join("; ")) })
}

fn host_name(host: &Host<&str>) -> String {
    match host {
        Host::Domain(d) => d.to_ascii_lowercase(),
        Host::Ipv4(ip) => ip.to_string(),
        Host::Ipv6(ip) => format!("[{ip}]"),
    }
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// `/api` covers `/api`, `/api/` and `/api/x`, but not `/apix`.
fn path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie) && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

/// The folder of the request's path: where a cookie without `Path` applies.
fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_string(),
    }
}

/// One `Set-Cookie` header; the domain is empty when the server gave none.
fn parse(header: &str, url: &Url, now: u64) -> Option<Cookie> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut cookie = Cookie {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain: String::new(),
        host_only: false,
        path: default_path(url),
        expires: None,
        secure: false,
        http_only: false,
        same_site: None,
        created: now,
    };
    let mut max_age = None;
    for attr in parts {
        let (key, value) = attr.split_once('=').unwrap_or((attr, ""));
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "expires" => {
                if let Some(at) = parse_date(value) {
                    cookie.expires = Some(at);
                }
            }
            "max-age" => {
                if let Ok(seconds) = value.parse::<i64>() {
                    max_age = Some(seconds);
                }
            }
            "domain" => {
                let domain = value.trim_start_matches('.').to_ascii_lowercase();
                if !domain.is_empty() {
                    cookie.domain = domain;
                }
            }
            "path" => {
                cookie.path = if value.starts_with('/') { value.to_string() } else { default_path(url) };
            }
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "samesite" if !value.is_empty() => cookie.same_site = Some(value.to_string()),
            _ => {}
        }
    }
    // Max-Age wins over Expires.
    if let Some(seconds) = max_age {
        cookie.expires = Some(if seconds <= 0 { 0 } else { (now / 1000) as i64 + seconds });
    }
    Some(cookie)
}

/// A date as servers write it in `Expires`, by the lenient rules of RFC 6265 §5.1.1:
/// `Wed, 21 Oct 2015 07:28:00 GMT`, `Wednesday, 21-Oct-15 07:28:00 GMT` and the like.
fn parse_date(text: &str) -> Option<i64> {
    let delimiter = |c: char| matches!(c, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~');
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for token in text.split(delimiter).filter(|t| !t.is_empty()) {
        let digits = |t: &str, min: usize, max: usize| {
            let n = t.bytes().take_while(u8::is_ascii_digit).count();
            (min..=max).contains(&n).then(|| t[..n].parse::<i64>().ok()).flatten()
        };
        if time.is_none() {
            let fields: Vec<&str> = token.splitn(3, ':').collect();
            if let [h, m, s] = fields[..]
                && let (Some(h), Some(m), Some(s)) = (digits(h, 1, 2), digits(m, 1, 2), digits(s, 1, 2))
            {
                time = Some((h, m, s));
                continue;
            }
        }
        if day.is_none()
            && let Some(d) = digits(token, 1, 2)
        {
            day = Some(d);
            continue;
        }
        if month.is_none()
            && let Some(head) = token.get(..3)
        {
            const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
            if let Some(m) = MONTHS.iter().position(|m| head.eq_ignore_ascii_case(m)) {
                month = Some(m as i64 + 1);
                continue;
            }
        }
        if year.is_none()
            && let Some(y) = digits(token, 2, 4)
        {
            year = Some(y);
        }
    }
    let (h, m, s) = time?;
    let (day, month, mut year) = (day?, month?, year?);
    year += match year {
        70..=99 => 1900,
        0..=69 => 2000,
        _ => 0,
    };
    if !(1..=31).contains(&day) || year < 1601 || h > 23 || m > 59 || s > 59 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn dates_as_servers_write_them() {
        assert_eq!(parse_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1_445_412_480));
        assert_eq!(parse_date("Wednesday, 21-Oct-15 07:28:00 GMT"), Some(1_445_412_480));
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_date("Sun Nov  6 08:49:37 1994"), Some(784_111_777));
        assert_eq!(parse_date("вчера"), None);
    }

    #[test]
    fn sends_back_by_domain_path_and_secure() {
        let mut jar = Jar::default();
        let login = url("https://api.shop.test/auth/login");
        jar.store_at(&login, "session=abc; Path=/; HttpOnly; Secure", NOW);
        jar.store_at(&login, "auth_step=2", NOW + 1);
        jar.store_at(&login, "lang=ru; Domain=.shop.test; Path=/", NOW + 2);
        jar.store_at(&login, "evil=1; Domain=other.test", NOW);
        jar.store_at(&login, "tld=1; Domain=test", NOW);

        assert_eq!(jar.header_at(&url("https://api.shop.test/orders"), NOW).as_deref(), Some("session=abc; lang=ru"));
        assert_eq!(jar.header_at(&url("https://api.shop.test/auth/me"), NOW).as_deref(), Some("auth_step=2; session=abc; lang=ru"));
        // Not the subdomain for a cookie without Domain; not over plain HTTP for a secure one.
        assert_eq!(jar.header_at(&url("https://cdn.shop.test/"), NOW).as_deref(), Some("lang=ru"));
        assert_eq!(jar.header_at(&url("http://api.shop.test/"), NOW).as_deref(), Some("lang=ru"));
        assert_eq!(jar.header_at(&url("https://other.test/"), NOW), None);
    }

    #[test]
    fn replaces_expires_and_deletes() {
        let mut jar = Jar::default();
        let u = url("http://localhost:8080/api/login");
        jar.store_at(&u, "token=1; Max-Age=60; Secure", NOW);
        // Secure still goes to this machine over HTTP; the path defaults to the folder.
        assert_eq!(jar.header_at(&url("http://localhost:8080/api/x"), NOW).as_deref(), Some("token=1"));
        assert_eq!(jar.header_at(&url("http://localhost:8080/apix"), NOW), None);
        jar.store_at(&u, "token=2; Max-Age=60; Secure", NOW + 1000);
        assert_eq!(jar.cookies.len(), 1);
        assert_eq!(jar.header_at(&url("http://localhost:8080/api/"), NOW + 30_000).as_deref(), Some("token=2"));
        assert_eq!(jar.header_at(&url("http://localhost:8080/api/"), NOW + 61_000), None);
        assert!(jar.cookies.is_empty());

        jar.store_at(&u, "a=1", NOW);
        assert!(jar.take_changed());
        jar.store_at(&u, "a=; Expires=Thu, 01 Jan 1970 00:00:00 GMT", NOW);
        assert!(jar.cookies.is_empty() && jar.take_changed());
    }

    #[test]
    fn own_cookies_win() {
        assert_eq!(merge(Some("a=mine"), Some("a=jar; b=2")).as_deref(), Some("a=mine; b=2"));
        assert_eq!(merge(None, Some("b=2")).as_deref(), Some("b=2"));
        assert_eq!(merge(Some("a=1"), None).as_deref(), Some("a=1"));
        assert_eq!(merge(Some(" "), None), None);
    }
}
