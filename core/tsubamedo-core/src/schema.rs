//! JSON Schema for the `schema` check: `$ schema schemas/order.json`, or the schema right in
//! the line: `$.id schema {"type": "integer"}`. `file.json#/$defs/Order` takes a part of a file.
//!
//! A file is looked for from the request's folder up to the collection's root, so a shared
//! `schemas/` folder at the top serves every request; `$ref` to other files works relative to
//! the schema file. Schemas without `$schema` are draft 2020-12; `format` is a note, not a
//! rule, as the standard says. Nothing is fetched from the network.

use std::path::{Path, PathBuf};

use boon::{CompileError, Compiler, Draft, ErrorKind, Schemas, Type, ValidationError};
use serde_json::Value;

use crate::collection::MANIFEST;

/// How many violations a failed check lists.
const SHOWN: usize = 3;

/// Whether a schema written in a check line compiles, for the editor.
pub fn check_inline(schema: &Value) -> Result<(), String> {
    compile(schema, None).map(|_| ())
}

/// Validates each value (with where it is in the response) against the schema: an inline one
/// or a file path. `Err` when the schema itself could not be had; `Ok(Err(why))` when values
/// break it.
pub fn validate(schema: &Value, dir: Option<&Path>, values: &[(Vec<String>, &Value)]) -> Result<Result<(), String>, String> {
    let (schemas, index) = compile(schema, dir)?;
    let mut problems = Vec::new();
    for (at, value) in values {
        if let Err(e) = schemas.validate(value, index) {
            leaves(&e, at, false, &mut problems);
        }
    }
    if problems.is_empty() {
        return Ok(Ok(()));
    }
    let total = problems.len();
    let mut text = problems.into_iter().take(SHOWN).collect::<Vec<_>>().join("; ");
    if total > SHOWN {
        text.push_str(&format!("; и ещё {}", total - SHOWN));
    }
    Ok(Err(text))
}

fn compile(schema: &Value, dir: Option<&Path>) -> Result<(Schemas, boon::SchemaIndex), String> {
    let mut compiler = Compiler::new();
    compiler.set_default_draft(Draft::V2020_12);
    let mut schemas = Schemas::new();
    let (url, label) = match schema {
        Value::String(path) => {
            // `file.json#/$defs/Order` is a part of the file.
            let (path, fragment) = path.split_once('#').unwrap_or((path, ""));
            let file = find(path, dir)?;
            let mut url = url::Url::from_file_path(&file).map_err(|_| format!("схема {}: путь не годится", file.display()))?;
            url.set_fragment(Some(fragment).filter(|f| !f.is_empty()));
            (url.to_string(), format!("схема {path}"))
        }
        inline => {
            // Relative `$ref` in a schema written in the line resolve from the request's folder.
            let base = dir.and_then(|d| std::path::absolute(d).ok()).unwrap_or_else(std::env::temp_dir);
            let url = url::Url::from_file_path(base.join("inline-schema.json")).map_err(|_| "схема: путь не годится".to_string())?.to_string();
            compiler.add_resource(&url, inline.clone()).map_err(|e| broken(&e, "схема"))?;
            (url, "схема".to_string())
        }
    };
    let index = compiler.compile(&url, &mut schemas).map_err(|e| broken(&e, &label))?;
    Ok((schemas, index))
}

/// The file from the request's folder up to the collection's root.
fn find(path: &str, dir: Option<&Path>) -> Result<PathBuf, String> {
    let rel = Path::new(path);
    if rel.is_absolute() {
        return if rel.is_file() { Ok(rel.to_path_buf()) } else { Err(format!("нет файла схемы {path}")) };
    }
    let mut at = dir.and_then(|d| std::path::absolute(d).ok());
    while let Some(d) = at {
        let file = d.join(rel);
        if file.is_file() {
            return Ok(file);
        }
        if d.join(MANIFEST).is_file() {
            break;
        }
        at = d.parent().map(Path::to_path_buf);
    }
    Err(format!("нет файла схемы {path} — искал от папки запроса до корня коллекции"))
}

/// Why a schema could not be used, without the long file URLs the library speaks in.
fn broken(e: &CompileError, label: &str) -> String {
    match e {
        // The schema itself breaks the rules of JSON Schema: said like a failed check.
        CompileError::ValidationError { src, .. } => {
            let mut problems = Vec::new();
            leaves(src, &[], true, &mut problems);
            format!("{label} неверна: {}", problems.join("; "))
        }
        CompileError::LoadUrlError { url, src } => {
            let file = url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok()).map(|p| p.display().to_string()).unwrap_or_else(|| url.clone());
            format!("{label}: не прочитать {file}: {src}")
        }
        CompileError::UnsupportedUrlScheme { url } => format!("{label}: {url} — схемы берутся только из файлов, не из сети"),
        other => format!("{label}: {other}"),
    }
}

/// The violations at the bottom of the error tree, each with its place. `first_choice` follows
/// the first option of `anyOf`/`oneOf`: for a broken schema that is the useful one.
fn leaves(e: &ValidationError, at: &[String], first_choice: bool, out: &mut Vec<String>) {
    let descend = matches!(e.kind, ErrorKind::Group | ErrorKind::Schema { .. } | ErrorKind::Reference { .. } | ErrorKind::AllOf | ErrorKind::ContentSchema);
    if descend && !e.causes.is_empty() {
        for c in &e.causes {
            leaves(c, at, first_choice, out);
        }
        return;
    }
    if first_choice && matches!(e.kind, ErrorKind::AnyOf | ErrorKind::OneOf(None)) && let Some(first) = e.causes.first() {
        return leaves(first, at, first_choice, out);
    }
    let mut place: Vec<String> = at.to_vec();
    place.extend(e.instance_location.tokens.iter().map(|t| match t {
        boon::InstanceToken::Prop(p) => p.to_string(),
        boon::InstanceToken::Item(i) => i.to_string(),
    }));
    out.push(format!("{}: {}", path(&place), say(&e.kind)));
}

/// `$.items[2].price` from its parts.
pub(crate) fn path(parts: &[String]) -> String {
    let mut out = String::from("$");
    for p in parts {
        if p.parse::<usize>().is_ok() {
            out.push_str(&format!("[{p}]"));
        } else if !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_') {
            out.push('.');
            out.push_str(p);
        } else {
            out.push_str(&format!("[{}]", Value::String(p.clone())));
        }
    }
    out
}

fn kind(t: Type) -> &'static str {
    match t {
        Type::Null => "null",
        Type::Boolean => "true/false",
        Type::Number => "число",
        Type::Integer => "целое",
        Type::String => "строка",
        Type::Array => "массив",
        Type::Object => "объект",
    }
}

fn list<T: std::fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    items.into_iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
}

/// A violation in words.
fn say(k: &ErrorKind) -> String {
    match k {
        ErrorKind::Type { got, want } => format!("ожидалось {}, а здесь {}", want.iter().map(kind).collect::<Vec<_>>().join(" или "), kind(*got)),
        ErrorKind::Required { want } if want.len() == 1 => format!("нет поля {}", want[0]),
        ErrorKind::Required { want } => format!("нет полей {}", list(want)),
        ErrorKind::AdditionalProperties { got } => format!("лишние поля: {}", list(got)),
        ErrorKind::Enum { want } => format!("ожидалось одно из {}", list(want.iter())),
        ErrorKind::Const { want } => format!("ожидалось {want}"),
        ErrorKind::Minimum { got, want } => format!("{got} меньше {want}"),
        ErrorKind::Maximum { got, want } => format!("{got} больше {want}"),
        ErrorKind::ExclusiveMinimum { got, want } => format!("{got} не больше {want}"),
        ErrorKind::ExclusiveMaximum { got, want } => format!("{got} не меньше {want}"),
        ErrorKind::MultipleOf { got, want } => format!("{got} не делится на {want}"),
        ErrorKind::MinLength { got, want } => format!("длина {got}, а нужно не меньше {want}"),
        ErrorKind::MaxLength { got, want } => format!("длина {got}, а нужно не больше {want}"),
        ErrorKind::MinItems { got, want } => format!("элементов {got}, а нужно не меньше {want}"),
        ErrorKind::MaxItems { got, want } => format!("элементов {got}, а нужно не больше {want}"),
        ErrorKind::MinProperties { got, want } => format!("полей {got}, а нужно не меньше {want}"),
        ErrorKind::MaxProperties { got, want } => format!("полей {got}, а нужно не больше {want}"),
        ErrorKind::UniqueItems { got: [a, b] } => format!("элементы {a} и {b} одинаковые"),
        ErrorKind::Pattern { got, want } => format!("«{got}» не подходит под шаблон {want}"),
        ErrorKind::Format { got, want, .. } => format!("{got} не в формате {want}"),
        ErrorKind::FalseSchema => "этого здесь быть не должно".into(),
        ErrorKind::Not => "подходит под то, что запрещено (not)".into(),
        ErrorKind::AnyOf | ErrorKind::OneOf(None) => "не подходит ни под один вариант".into(),
        ErrorKind::OneOf(Some((a, b))) => format!("подходит сразу под варианты {a} и {b}, а нужен один"),
        ErrorKind::Contains => "нет ни одного подходящего элемента".into(),
        ErrorKind::DependentRequired { prop, missing } | ErrorKind::Dependency { prop, missing } => {
            format!("раз есть {prop}, нужны и {}", list(missing))
        }
        other => other.to_string(),
    }
}
