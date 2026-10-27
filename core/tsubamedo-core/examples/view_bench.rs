//! How long the viewer takes on a big body and how much the index costs:
//!
//!   cargo run --release -p tsubamedo-core --example view_bench -- big.json [query]

use std::time::Instant;

use tsubamedo_core::http::ResponseBody;
use tsubamedo_core::view::{Matcher, TextView};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("path to a file");
    let query = args.next().unwrap_or_else(|| "Сочи".into());
    let data = std::fs::read(&path).unwrap();
    let body = ResponseBody::Memory(data.into());

    let t = Instant::now();
    let raw = TextView::raw(&body).unwrap();
    println!("raw: {} lines in {:?}", raw.line_count(), t.elapsed());

    let t = Instant::now();
    let pretty = TextView::pretty_json(&body, 8 << 20, std::env::temp_dir().join("tsubamedo")).unwrap().unwrap();
    println!("pretty: {} lines, {} MB in {:?}", pretty.line_count(), pretty.len() >> 20, t.elapsed());

    for (name, case) in [("exact", true), ("any case", false)] {
        let t = Instant::now();
        let found = pretty.search(&Matcher::new(&query, case).unwrap());
        println!("search {name} «{query}»: {} matches on {} lines in {:?}", found.total, found.lines.len(), t.elapsed());
    }
}
