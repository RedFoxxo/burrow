//! Tool parameters: one definition gives both the JSON schema the model reads and the
//! validation of the arguments it sends. The schema is as lean as the TypeScript build's
//! (no `$schema`, no safe-integer bounds, no `propertyNames`): every byte is model tokens.

use regex::Regex;
use serde_json::{Map, Value, json};

#[derive(Clone)]
pub enum Kind {
    Str {
        trim: bool,
        min: Option<usize>,
        pattern: Option<(Regex, &'static str)>,
    },
    OneOf(&'static [&'static str]),
    Int {
        min: Option<i64>,
        max: Option<i64>,
    },
    Bool,
    List {
        item: Box<Kind>,
        min: Option<usize>,
    },
    /// An object of strings.
    Record,
}

#[derive(Clone)]
pub struct Field {
    pub name: &'static str,
    pub kind: Kind,
    pub optional: bool,
    pub description: Option<&'static str>,
    /// The description comes before the type keys when it was given after `optional()`, as
    /// zod writes a description on the optional wrapper; `tests/golden/` pins this order.
    pub description_first: bool,
}

fn field(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        optional: false,
        description: None,
        description_first: false,
    }
}

/// Free text, kept as sent.
pub fn text(name: &'static str) -> Field {
    field(
        name,
        Kind::Str {
            trim: false,
            min: None,
            pattern: None,
        },
    )
}

/// Text trimmed before it is checked and used.
pub fn trimmed(name: &'static str) -> Field {
    field(
        name,
        Kind::Str {
            trim: true,
            min: None,
            pattern: None,
        },
    )
}

pub fn one_of(name: &'static str, values: &'static [&'static str]) -> Field {
    field(name, Kind::OneOf(values))
}

pub fn integer(name: &'static str, min: Option<i64>, max: Option<i64>) -> Field {
    field(name, Kind::Int { min, max })
}

pub fn boolean(name: &'static str) -> Field {
    field(name, Kind::Bool)
}

/// A list of trimmed, non-empty strings.
pub fn names(name: &'static str) -> Field {
    field(
        name,
        Kind::List {
            item: Box::new(Kind::Str {
                trim: true,
                min: Some(1),
                pattern: None,
            }),
            min: None,
        },
    )
}

/// A list of strings, kept as sent.
pub fn texts(name: &'static str) -> Field {
    field(
        name,
        Kind::List {
            item: Box::new(Kind::Str {
                trim: false,
                min: None,
                pattern: None,
            }),
            min: None,
        },
    )
}

pub fn record(name: &'static str) -> Field {
    field(name, Kind::Record)
}

impl Field {
    /// At least one character (string) or item (list).
    pub fn nonempty(mut self) -> Self {
        match &mut self.kind {
            Kind::Str { min, .. } | Kind::List { min, .. } => *min = Some(1),
            _ => unreachable!("nonempty applies to strings and lists"),
        }
        self
    }

    pub fn pattern(mut self, re: &str, message: &'static str) -> Self {
        let Kind::Str { pattern, .. } = &mut self.kind else {
            unreachable!("pattern applies to strings")
        };
        *pattern = Some((Regex::new(re).expect("valid pattern"), message));
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn describe(mut self, description: &'static str) -> Self {
        self.description = Some(description);
        self.description_first = self.optional;
        self
    }
}

fn kind_schema(kind: &Kind) -> Map<String, Value> {
    let mut s = Map::new();
    match kind {
        Kind::Str { min, pattern, .. } => {
            s.insert("type".into(), json!("string"));
            if let Some(n) = min {
                s.insert("minLength".into(), json!(n));
            }
            if let Some((re, _)) = pattern {
                s.insert("pattern".into(), json!(re.as_str()));
            }
        }
        Kind::OneOf(values) => {
            s.insert("type".into(), json!("string"));
            s.insert("enum".into(), json!(values));
        }
        Kind::Int { min, max } => {
            s.insert("type".into(), json!("integer"));
            if let Some(n) = min {
                s.insert("minimum".into(), json!(n));
            }
            if let Some(n) = max {
                s.insert("maximum".into(), json!(n));
            }
        }
        Kind::Bool => {
            s.insert("type".into(), json!("boolean"));
        }
        Kind::List { item, min } => {
            if let Some(n) = min {
                s.insert("minItems".into(), json!(n));
            }
            s.insert("type".into(), json!("array"));
            s.insert("items".into(), Value::Object(kind_schema(item)));
        }
        Kind::Record => {
            s.insert("type".into(), json!("object"));
            s.insert("additionalProperties".into(), json!({ "type": "string" }));
        }
    }
    s
}

/// The `inputSchema` of a tool.
pub fn input_schema(fields: &[Field]) -> Value {
    let mut properties = Map::new();
    for f in fields {
        let mut s = Map::new();
        if let Some(d) = f.description.filter(|_| f.description_first) {
            s.insert("description".into(), json!(d));
        }
        s.extend(kind_schema(&f.kind));
        if let Some(d) = f.description.filter(|_| !f.description_first) {
            s.insert("description".into(), json!(d));
        }
        properties.insert(f.name.into(), Value::Object(s));
    }
    let required: Vec<&str> = fields
        .iter()
        .filter(|f| !f.optional)
        .map(|f| f.name)
        .collect();
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    Value::Object(schema)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The value as the handler gets it (strings trimmed where asked), or the problems with it.
fn check(kind: &Kind, value: &Value, path: &str, issues: &mut Vec<String>) -> Value {
    let at = |message: String| format!("{message} at {path}");
    match kind {
        Kind::Str { trim, min, pattern } => {
            let Value::String(s) = value else {
                issues.push(at(format!(
                    "Invalid input: expected string, received {}",
                    type_name(value)
                )));
                return Value::Null;
            };
            let s = if *trim { s.trim() } else { s.as_str() };
            if let Some(n) = min.filter(|n| s.chars().count() < *n) {
                issues.push(at(format!(
                    "Too small: expected string to have >={n} characters"
                )));
            }
            if let Some((re, message)) = pattern {
                if !re.is_match(s) {
                    issues.push(at((*message).to_string()));
                }
            }
            json!(s)
        }
        Kind::OneOf(values) => {
            if !value.as_str().is_some_and(|s| values.contains(&s)) {
                let options: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
                issues.push(at(format!(
                    "Invalid option: expected one of {}",
                    options.join("|")
                )));
            }
            value.clone()
        }
        Kind::Int { min, max } => {
            let Some(n) = value.as_f64() else {
                issues.push(at(format!(
                    "Invalid input: expected number, received {}",
                    type_name(value)
                )));
                return Value::Null;
            };
            if n.fract() != 0.0 || n.abs() > 9_007_199_254_740_991.0 {
                issues.push(at("Invalid input: expected int, received number".into()));
                return Value::Null;
            }
            let n = n as i64;
            if let Some(m) = min.filter(|m| n < *m) {
                issues.push(at(format!("Too small: expected number to be >={m}")));
            }
            if let Some(m) = max.filter(|m| n > *m) {
                issues.push(at(format!("Too big: expected number to be <={m}")));
            }
            json!(n)
        }
        Kind::Bool => {
            if !value.is_boolean() {
                issues.push(at(format!(
                    "Invalid input: expected boolean, received {}",
                    type_name(value)
                )));
            }
            value.clone()
        }
        Kind::List { item, min } => {
            let Value::Array(items) = value else {
                issues.push(at(format!(
                    "Invalid input: expected array, received {}",
                    type_name(value)
                )));
                return Value::Null;
            };
            if let Some(n) = min.filter(|n| items.len() < *n) {
                issues.push(at(format!("Too small: expected array to have >={n} items")));
            }
            Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| check(item, v, &format!("{path}.{i}"), issues))
                    .collect(),
            )
        }
        Kind::Record => {
            let Value::Object(map) = value else {
                issues.push(at(format!(
                    "Invalid input: expected record, received {}",
                    type_name(value)
                )));
                return Value::Null;
            };
            for (k, v) in map.iter().filter(|(_, v)| !v.is_string()) {
                issues.push(format!(
                    "Invalid input: expected string, received {} at {path}.{k}",
                    type_name(v)
                ));
            }
            value.clone()
        }
    }
}

/// Checks `args` against `fields`; unknown keys are dropped. `Err` lists every problem.
pub fn validate(fields: &[Field], args: &Value) -> Result<Args, String> {
    let empty = Map::new();
    let given = match args {
        Value::Object(map) => map,
        Value::Null => &empty,
        other => {
            return Err(format!(
                "Invalid input: expected object, received {}",
                type_name(other)
            ));
        }
    };
    let mut issues = Vec::new();
    let mut out = Map::new();
    for f in fields {
        match given.get(f.name) {
            None if f.optional => {}
            None => {
                let expected = match f.kind {
                    Kind::Str { .. } | Kind::OneOf(_) => "string",
                    Kind::Int { .. } => "number",
                    Kind::Bool => "boolean",
                    Kind::List { .. } => "array",
                    Kind::Record => "record",
                };
                issues.push(format!(
                    "Invalid input: expected {expected}, received undefined at {}",
                    f.name
                ));
            }
            Some(v) => {
                let checked = check(&f.kind, v, f.name, &mut issues);
                out.insert(f.name.into(), checked);
            }
        }
    }
    if issues.is_empty() {
        Ok(Args(out))
    } else {
        Err(issues.join("; "))
    }
}

/// Validated arguments. Getters return `None` for a parameter that was left out.
#[derive(Debug)]
pub struct Args(Map<String, Value>);

impl Args {
    pub fn opt_str(&self, name: &str) -> Option<String> {
        self.0.get(name).and_then(Value::as_str).map(str::to_string)
    }

    /// A required string; validation guarantees it is there.
    pub fn str(&self, name: &str) -> String {
        self.opt_str(name).unwrap_or_default()
    }

    pub fn opt_int(&self, name: &str) -> Option<i64> {
        self.0.get(name).and_then(Value::as_i64)
    }

    pub fn opt_bool(&self, name: &str) -> Option<bool> {
        self.0.get(name).and_then(Value::as_bool)
    }

    pub fn opt_strs(&self, name: &str) -> Option<Vec<String>> {
        self.0.get(name).and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    pub fn opt_map(&self, name: &str) -> Option<Map<String, Value>> {
        self.0.get(name).and_then(Value::as_object).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_before_checking_and_hands_on_the_trimmed_value() {
        let fields = [
            trimmed("page").nonempty(),
            integer("limit", Some(1), Some(100)).optional(),
        ];
        let args = validate(&fields, &json!({ "page": "  docs ", "extra": 1 })).unwrap();
        assert_eq!(args.str("page"), "docs");
        assert_eq!(args.opt_int("limit"), None);
        let problems = validate(&fields, &json!({ "page": "   ", "limit": 0 })).unwrap_err();
        assert!(
            problems.contains("Too small: expected string to have >=1 characters at page"),
            "{problems}"
        );
        assert!(
            problems.contains("Too small: expected number to be >=1 at limit"),
            "{problems}"
        );
    }

    #[test]
    fn refuses_missing_wrong_typed_and_fractional_values() {
        let fields = [
            trimmed("page").nonempty(),
            integer("position", Some(0), None).optional(),
            names("tags").optional(),
        ];
        assert!(
            validate(&fields, &json!({}))
                .unwrap_err()
                .contains("received undefined at page")
        );
        assert!(
            validate(&fields, &json!({ "page": 1 }))
                .unwrap_err()
                .contains("expected string, received number")
        );
        assert!(
            validate(&fields, &json!({ "page": "p", "position": 1.5 }))
                .unwrap_err()
                .contains("expected int")
        );
        assert!(
            validate(&fields, &json!({ "page": "p", "tags": ["a", " "] }))
                .unwrap_err()
                .contains("at tags.1")
        );
        assert_eq!(
            validate(&fields, &json!({ "page": "p", "position": 2.0 }))
                .unwrap()
                .opt_int("position"),
            Some(2)
        );
    }
}
