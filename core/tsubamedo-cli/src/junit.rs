//! JUnit XML, the report every CI reads: one test case per request sent. A request that did
//! not get a response is an error; failed checks are a failure.

use std::fmt::Write;
use std::time::Duration;

use tsubamedo_core::run::Exchange;

struct Case {
    id: String,
    seconds: f64,
    error: Option<String>,
    failures: Vec<String>,
}

pub struct Report {
    name: String,
    cases: Vec<Case>,
}

impl Report {
    pub fn new(name: &str) -> Self {
        Report { name: name.to_string(), cases: Vec::new() }
    }

    /// A request file that could not be read.
    pub fn broken(&mut self, error: &str) {
        self.cases.push(Case { id: tsubamedo_core::tr!("файл", "file"), seconds: 0.0, error: Some(error.to_string()), failures: Vec::new() });
    }

    /// `failures` are those of the exchange plus whatever the run itself adds (`--fail`).
    pub fn case(&mut self, ex: &Exchange, failures: &[String]) {
        let (seconds, error, failures) = match &ex.result {
            Ok(r) => (r.timings.total.as_secs_f64(), None, failures.to_vec()),
            Err(e) => (0.0, Some(e.to_string()), Vec::new()),
        };
        self.cases.push(Case { id: ex.id.clone().unwrap_or_default(), seconds, error, failures });
    }

    /// Passed and failed cases.
    pub fn counts(&self) -> (usize, usize) {
        let failed = self.cases.iter().filter(|c| c.error.is_some() || !c.failures.is_empty()).count();
        (self.cases.len() - failed, failed)
    }

    pub fn xml(&self, total: Duration) -> String {
        let errors = self.cases.iter().filter(|c| c.error.is_some()).count();
        let failures = self.cases.iter().filter(|c| c.error.is_none() && !c.failures.is_empty()).count();
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let head = format!(
            "name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"{errors}\" time=\"{:.3}\"",
            escape(&self.name),
            self.cases.len(),
            total.as_secs_f64()
        );
        let _ = writeln!(out, "<testsuites {head}>");
        let _ = writeln!(out, "  <testsuite {head}>");
        for case in &self.cases {
            // CI tools group by class: the request's folder, or the collection for the root.
            let class = match case.id.rsplit_once('/') {
                Some((folder, _)) => format!("{}.{}", self.name, folder.replace('/', ".")),
                None => self.name.clone(),
            };
            let _ = write!(out, "    <testcase classname=\"{}\" name=\"{}\" time=\"{:.3}\"", escape(&class), escape(&case.id), case.seconds);
            if let Some(error) = &case.error {
                let _ = writeln!(out, ">\n      <error message=\"{}\" type=\"error\"/>\n    </testcase>", escape(error));
            } else if let Some(first) = case.failures.first() {
                let _ = writeln!(
                    out,
                    ">\n      <failure message=\"{}\" type=\"check\">{}</failure>\n    </testcase>",
                    escape(first),
                    escape(&case.failures.join("\n"))
                );
            } else {
                let _ = writeln!(out, "/>");
            }
        }
        out.push_str("  </testsuite>\n</testsuites>\n");
        out
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            // XML 1.0 has no other control characters.
            c if c.is_control() && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_cases_failures_and_errors() {
        let mut r = Report::new("Магазин & Ко");
        r.cases.push(Case { id: "auth/login".into(), seconds: 0.12, error: None, failures: Vec::new() });
        r.cases.push(Case { id: "orders/list".into(), seconds: 0.05, error: None, failures: vec!["status == 200 — пришло 404".into(), "$.a exists — в ответе нет $.a".into()] });
        r.cases.push(Case { id: "ping".into(), seconds: 0.0, error: Some("не удалось подключиться к x:1: <отказ>".into()), failures: Vec::new() });
        assert_eq!(r.counts(), (1, 2));
        tsubamedo_core::lang::in_language(true, || r.broken("x.toml: oops"));
        assert_eq!(r.cases.pop().map(|c| c.id).as_deref(), Some("file"));
        let xml = r.xml(Duration::from_millis(1500));
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <testsuites name=\"Магазин &amp; Ко\" tests=\"3\" failures=\"1\" errors=\"1\" time=\"1.500\">\n\
             \x20 <testsuite name=\"Магазин &amp; Ко\" tests=\"3\" failures=\"1\" errors=\"1\" time=\"1.500\">\n\
             \x20   <testcase classname=\"Магазин &amp; Ко.auth\" name=\"auth/login\" time=\"0.120\"/>\n\
             \x20   <testcase classname=\"Магазин &amp; Ко.orders\" name=\"orders/list\" time=\"0.050\">\n\
             \x20     <failure message=\"status == 200 — пришло 404\" type=\"check\">status == 200 — пришло 404&#10;$.a exists — в ответе нет $.a</failure>\n\
             \x20   </testcase>\n\
             \x20   <testcase classname=\"Магазин &amp; Ко\" name=\"ping\" time=\"0.000\">\n\
             \x20     <error message=\"не удалось подключиться к x:1: &lt;отказ&gt;\" type=\"error\"/>\n\
             \x20   </testcase>\n\
             \x20 </testsuite>\n\
             </testsuites>\n"
        );
    }
}
