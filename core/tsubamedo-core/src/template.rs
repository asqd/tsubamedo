//! `{{name}}` substitution.
//!
//! A variable's value may refer to other variables. Names starting with `$` are built in:
//! `$uuid`, `$timestamp`, `$timestampMs`, `$isoTimestamp`, `$randomInt` and `$env.NAME` (the process
//! environment, so CI can pass secrets without files).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

pub type Vars = BTreeMap<String, String>;

/// Renders many strings and reports every undefined variable at once.
pub struct Renderer<'a> {
    vars: &'a Vars,
    missing: BTreeSet<String>,
    /// Every variable met, defined or not, with those inside other variables' values.
    used: BTreeSet<String>,
    cycle: Option<String>,
}

impl<'a> Renderer<'a> {
    pub fn new(vars: &'a Vars) -> Self {
        Renderer { vars, missing: BTreeSet::new(), used: BTreeSet::new(), cycle: None }
    }

    /// Undefined variables are left as written and remembered for [`Renderer::finish`].
    pub fn render(&mut self, input: &str) -> String {
        self.expand(input, &mut Vec::new())
    }

    pub fn used(&self) -> &BTreeSet<String> {
        &self.used
    }

    pub fn finish(self) -> Result<()> {
        if let Some(name) = self.cycle {
            return Err(Error::VarCycle(name));
        }
        if !self.missing.is_empty() {
            return Err(Error::UndefinedVars(self.missing.into_iter().collect()));
        }
        Ok(())
    }

    fn expand(&mut self, input: &str, stack: &mut Vec<String>) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after = &rest[open + 2..];
            let Some(close) = after.find("}}") else {
                out.push_str(&rest[open..]);
                return out;
            };
            let raw = &after[..close];
            match self.resolve(raw.trim(), stack) {
                Some(value) => out.push_str(&value),
                None => {
                    out.push_str("{{");
                    out.push_str(raw);
                    out.push_str("}}");
                }
            }
            rest = &after[close + 2..];
        }
        out.push_str(rest);
        out
    }

    fn resolve(&mut self, name: &str, stack: &mut Vec<String>) -> Option<String> {
        if let Some(builtin) = name.strip_prefix('$') {
            let value = dynamic(builtin);
            if value.is_none() {
                self.missing.insert(name.to_string());
            }
            return value;
        }
        self.used.insert(name.to_string());
        if stack.iter().any(|n| n == name) {
            self.cycle.get_or_insert_with(|| name.to_string());
            return Some(String::new());
        }
        let Some(value) = self.vars.get(name) else {
            self.missing.insert(name.to_string());
            return None;
        };
        stack.push(name.to_string());
        let value = self.expand(value, stack);
        stack.pop();
        Some(value)
    }
}

/// Renders one string; any undefined variable is an error.
pub fn render(input: &str, vars: &Vars) -> Result<String> {
    let mut renderer = Renderer::new(vars);
    let out = renderer.render(input);
    renderer.finish().map(|()| out)
}

fn dynamic(name: &str) -> Option<String> {
    let now = || SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    match name {
        "uuid" => Some(uuid::Uuid::new_v4().to_string()),
        "timestamp" => Some(now().as_secs().to_string()),
        "timestampMs" => Some(now().as_millis().to_string()),
        "isoTimestamp" => Some(iso(now())),
        "randomInt" => Some(fastrand::u32(0..1000).to_string()),
        _ => std::env::var(name.strip_prefix("env.")?).ok(),
    }
}

/// `2025-01-31T12:00:00.000Z`, in UTC.
fn iso(t: std::time::Duration) -> String {
    let secs = t.as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days to a civil date, from Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3600, rest % 3600 / 60, rest % 60, t.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vars {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn substitutes_nested_variables() {
        let v = vars(&[("host", "api.example.com"), ("base", "https://{{host}}/v1")]);
        assert_eq!(render("{{ base }}/users/{{id}}", &v).unwrap_err().to_string(), "не заданы переменные: id");
        assert_eq!(render("{{ base }}/users", &v).unwrap(), "https://api.example.com/v1/users");
    }

    #[test]
    fn reports_all_missing_and_cycles() {
        let v = vars(&[("a", "{{b}}"), ("b", "{{a}}")]);
        assert!(matches!(render("{{a}}", &v), Err(Error::VarCycle(_))));
        let mut r = Renderer::new(&v);
        assert_eq!(r.render("{{x}} and {{y}}"), "{{x}} and {{y}}");
        assert!(matches!(r.finish(), Err(Error::UndefinedVars(names)) if names == ["x", "y"]));
    }

    #[test]
    fn leaves_unclosed_braces_and_builtins() {
        let v = Vars::new();
        assert_eq!(render("a {{ b", &v).unwrap(), "a {{ b");
        assert_eq!(render("{{$uuid}}", &v).unwrap().len(), 36);
        assert_eq!(iso(std::time::Duration::from_millis(1_709_210_096_789)), "2024-02-29T12:34:56.789Z");
        assert!(render("{{$nope}}", &v).is_err());
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("TSUBAMEDO_TEST_TOKEN", "t0k") };
        assert_eq!(render("{{$env.TSUBAMEDO_TEST_TOKEN}}", &v).unwrap(), "t0k");
    }
}
