//! Every burrow tool against a real LeafWiki: the release pinned in
//! `leafwiki.rs`, started on a free local port with throwaway data. A plain
//! `cargo test` skips these tests because the first run downloads LeafWiki;
//! `cargo test -- --include-ignored` runs them too.

mod client;
mod leafwiki;
mod session;
mod tools;

use regex::Regex;
use serde_json::Value;

/// `expected` is part of `actual`: objects may have more keys, arrays match
/// element by element with the same length, everything else matches whole.
fn has(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            e.iter().all(|(k, v)| a.get(k).is_some_and(|x| has(x, v)))
        }
        (Value::Array(a), Value::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(x, y)| has(x, y))
        }
        _ => actual == expected,
    }
}

#[track_caller]
pub fn assert_has(actual: &Value, expected: Value) {
    assert!(
        has(actual, &expected),
        "expected {actual}\nto contain {expected}"
    );
}

#[track_caller]
pub fn assert_re(text: &str, pattern: &str) {
    assert!(
        Regex::new(pattern).unwrap().is_match(text),
        "{text:?}\ndoes not match /{pattern}/"
    );
}

#[track_caller]
pub fn str_of(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not a string: {value}"))
}

/// The only element of an array.
#[track_caller]
pub fn one(value: &Value) -> &Value {
    match value.as_array().map(Vec::as_slice) {
        Some([only]) => only,
        _ => panic!("expected exactly one element: {value}"),
    }
}
