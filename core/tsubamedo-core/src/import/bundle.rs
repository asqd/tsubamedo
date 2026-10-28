//! An OpenAPI description split into files (`$ref: schemas/pet.yaml`, `common.yaml#/Error`)
//! pulled into one before the import reads it. A schema from another file becomes a component
//! with its name (so its check file is `schemas/Pet.json`, as for one written in place); a path
//! item, parameter or response from another file takes the place of its `$ref`. References
//! inside such files are read from their own folder, and files may refer to each other in
//! circles. Addresses on the network are not fetched.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::SIZE_LIMIT;

/// Where a value stands, which decides what an outside reference in it becomes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Any,
    Schema,
    /// A map of name → schema: `properties`, `components.schemas`.
    Schemas,
    /// A list of schemas: `allOf`, `prefixItems`.
    SchemaList,
}

struct Bundler {
    main: PathBuf,
    v2: bool,
    files: HashMap<PathBuf, Result<Value, String>>,
    /// `file#pointer` of a schema → its component name.
    names: HashMap<String, String>,
    taken: BTreeSet<String>,
    hoisted: Vec<(String, Value)>,
    warnings: Vec<String>,
}

/// Resolves the references to other files of the description read from `file`; returns what
/// could not be resolved, in words.
pub fn bundle(doc: &mut Value, file: &Path) -> Vec<String> {
    let v2 = doc.get("swagger").is_some();
    let components = if v2 { doc.get("definitions") } else { doc.pointer("/components/schemas") };
    let taken = components.and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let main = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let mut b = Bundler { main: main.clone(), v2, files: HashMap::new(), names: HashMap::new(), taken, hoisted: Vec::new(), warnings: Vec::new() };
    b.walk(doc, &main, Kind::Any, 0);
    if !b.hoisted.is_empty() {
        let schemas = if v2 { entry(doc, &["definitions"]) } else { entry(doc, &["components", "schemas"]) };
        for (name, schema) in b.hoisted {
            schemas.insert(name, schema);
        }
    }
    b.warnings
}

/// The object at `path` in `doc`, made when missing.
fn entry<'a>(doc: &'a mut Value, path: &[&str]) -> &'a mut Map<String, Value> {
    let mut at = doc;
    for key in path {
        let map = at.as_object_mut().expect("an OpenAPI description is an object");
        at = map.entry(key.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !at.is_object() {
            *at = Value::Object(Map::new());
        }
    }
    at.as_object_mut().expect("made an object above")
}

impl Bundler {
    fn walk(&mut self, v: &mut Value, base: &Path, kind: Kind, depth: usize) {
        if depth > 64 {
            return;
        }
        match v {
            Value::Object(map) => {
                if let Some(Value::String(r)) = map.get("$ref") {
                    let r = r.clone();
                    self.reference(v, &r, base, kind, depth);
                    return;
                }
                for (key, child) in map.iter_mut() {
                    let child_kind = match (kind, key.as_str()) {
                        (Kind::Schemas, _) => Kind::Schema,
                        // Values, not descriptions: a `$ref` in them is data.
                        (_, "example" | "enum" | "default" | "const") => continue,
                        (Kind::Schema, "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas") => Kind::Schemas,
                        (Kind::Schema, "allOf" | "anyOf" | "oneOf" | "prefixItems") => Kind::SchemaList,
                        (Kind::Schema, "items") if child.is_array() => Kind::SchemaList,
                        (
                            Kind::Schema,
                            "items" | "additionalProperties" | "additionalItems" | "not" | "contains" | "if" | "then" | "else" | "propertyNames"
                            | "unevaluatedProperties" | "unevaluatedItems",
                        ) => Kind::Schema,
                        (Kind::Schema, _) => continue,
                        (_, "schema") => Kind::Schema,
                        (_, "schemas") => Kind::Schemas,
                        (Kind::Any, "definitions") if depth == 0 && self.v2 => Kind::Schemas,
                        _ => Kind::Any,
                    };
                    self.walk(child, base, child_kind, depth + 1);
                }
            }
            Value::Array(list) => {
                let item_kind = if kind == Kind::SchemaList { Kind::Schema } else { kind };
                for item in list {
                    self.walk(item, base, item_kind, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// `{"$ref": r}` read in `base`: a reference to the main file stays (from its own place), a
    /// schema elsewhere becomes a component, anything else elsewhere is put in its place.
    fn reference(&mut self, v: &mut Value, r: &str, base: &Path, kind: Kind, depth: usize) {
        if r.starts_with("http://") || r.starts_with("https://") {
            self.warnings.push(format!("ссылки по сети не загружаются: {r}"));
            return;
        }
        let (file, pointer) = r.split_once('#').unwrap_or((r, ""));
        let target = if file.is_empty() {
            base.to_path_buf()
        } else {
            let path = base.parent().unwrap_or(Path::new(".")).join(percent_decode(file));
            match fs::canonicalize(&path) {
                Ok(path) => path,
                Err(e) => {
                    self.warnings.push(format!("ссылка {r}: не открыть {}: {e}", path.display()));
                    return;
                }
            }
        };
        let pointer = percent_decode(pointer);
        if target == self.main {
            v["$ref"] = Value::String(format!("#{pointer}"));
            return;
        }
        if kind == Kind::Schema || kind == Kind::Schemas {
            let name = self.hoist(&target, &pointer, r, depth);
            let prefix = if self.v2 { "#/definitions/" } else { "#/components/schemas/" };
            v["$ref"] = Value::String(format!("{prefix}{}", name.replace('~', "~0").replace('/', "~1")));
            return;
        }
        if let Some(mut found) = self.load(&target, &pointer, r) {
            self.walk(&mut found, &target, kind, depth + 1);
            *v = found;
        }
    }

    /// The component name of the schema at `pointer` in `file`, pulled in the first time.
    fn hoist(&mut self, file: &Path, pointer: &str, r: &str, depth: usize) -> String {
        let key = format!("{}#{pointer}", file.display());
        if let Some(name) = self.names.get(&key) {
            return name.clone();
        }
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Schema".into());
        let wanted = pointer.rsplit('/').next().filter(|s| !s.is_empty()).map(|s| s.replace("~1", "/").replace("~0", "~")).unwrap_or(stem);
        let mut name = wanted.clone();
        let mut n = 2;
        while self.taken.contains(&name) {
            name = format!("{wanted}{n}");
            n += 1;
        }
        self.taken.insert(name.clone());
        // Named before it is read: a schema that refers back to it gets the name.
        self.names.insert(key, name.clone());
        let mut schema = self.load(file, pointer, r).unwrap_or_else(|| Value::Object(Map::new()));
        self.walk(&mut schema, file, Kind::Schema, depth + 1);
        self.hoisted.push((name.clone(), schema));
        name
    }

    fn load(&mut self, file: &Path, pointer: &str, r: &str) -> Option<Value> {
        let doc = self.files.entry(file.to_path_buf()).or_insert_with(|| read(file));
        let doc = match &*doc {
            Ok(doc) => doc,
            Err(e) => {
                let e = e.clone();
                self.warnings.push(format!("ссылка {r}: {e}"));
                return None;
            }
        };
        let found = if pointer.is_empty() { Some(doc) } else { doc.pointer(pointer) };
        if found.is_none() {
            self.warnings.push(format!("ссылка {r} никуда не ведёт"));
        }
        found.cloned()
    }
}

fn read(file: &Path) -> Result<Value, String> {
    let size = fs::metadata(file).map_err(|e| format!("не прочитать {}: {e}", file.display()))?.len();
    if size > SIZE_LIMIT {
        return Err(format!("{} больше {} МБ", file.display(), SIZE_LIMIT >> 20));
    }
    let text = fs::read_to_string(file).map_err(|e| format!("не прочитать {}: {e}", file.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with(['{', '[']) {
        serde_json::from_str(text).map_err(|e| format!("{}: не JSON: {e}", file.display()))
    } else {
        serde_yaml_ng::from_str(text).map_err(|e| format!("{}: ни JSON, ни YAML: {e}", file.display()))
    }
}

/// `%20` and the like of a reference, which is a URI.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
