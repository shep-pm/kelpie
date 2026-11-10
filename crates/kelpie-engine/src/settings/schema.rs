//! The settings schema, and the check every settings value passes through
//!
//! [`document`] is the schema [`schemars`](schemars::JsonSchema) writes from
//! [`Settings`], which is also what `docs/settings.schema.json` holds. Reading a
//! layer file and writing a value both go through [`check`], so a person's
//! editor and kelpie refuse the same things.
//!
//! The check covers the keywords kelpie's own types can produce. A keyword it
//! does not know is a refusal rather than a pass, and
//! `the_check_covers_every_keyword_the_schema_uses` keeps the two in step.

use std::fs;
use std::io;
use std::sync::OnceLock;

use schemars::schema_for;
use serde_json::{Map, Value};

use super::layers::{Layer, LayerDoc, LayerFile};
use super::{ConfigDir, Field, SCHEMA_KEY, Settings, SettingsError};

/// The keywords [`check`] knows, as constraints or as plain description
pub(crate) const KNOWN: &[&str] = &[
    "$defs",
    "$id",
    "$ref",
    "$schema",
    "additionalProperties",
    "anyOf",
    "const",
    "default",
    "deprecated",
    "description",
    "enum",
    "items",
    "maxLength",
    "minLength",
    "maximum",
    "minimum",
    "oneOf",
    "properties",
    "required",
    "title",
    "type",
    "uniqueItems",
];

/// The keywords that say nothing about what a value may be
const DESCRIPTION: &[&str] = &[
    "$defs",
    "$id",
    "$ref",
    "$schema",
    "default",
    "deprecated",
    "description",
    "title",
];

/// The schema of one settings file, built once per process
pub(crate) fn document() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| schema_for!(Settings).to_value())
}

/// Read one layer's file and check it against the schema
///
/// A file that is not there reads as nothing set, and nothing is created.
///
/// # Errors
///
/// [`SettingsError::Unparsed`] names the file and the line its JSON broke at,
/// and [`SettingsError::FileRefused`] names the part of it the schema refuses.
pub(crate) fn read(dir: &ConfigDir, at: LayerFile<'_>) -> Result<LayerDoc, SettingsError> {
    let file = dir.file(at)?;
    let Some(text) = read_if_exists(&file)? else {
        return Ok(LayerDoc {
            file,
            fields: Map::new(),
        });
    };
    let fields = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(fields)) => fields,
        Ok(_) => return Err(unparsed(&file, 1, 1, "expected an object of settings")),
        Err(error) => {
            return Err(unparsed(
                &file,
                error.line(),
                error.column(),
                &error.to_string(),
            ));
        }
    };
    let mut doc = LayerDoc { file, fields };
    // A field written as null is a field nobody set: it is dropped before any
    // check asks which layer set it
    doc.fields.retain(|_, value| !value.is_null());
    holds_only_its_layer(&doc, at.layer())?;
    if let Some((path, reason)) = check(document(), &Value::Object(doc.fields.clone()), "") {
        return Err(SettingsError::FileRefused {
            file: doc.file,
            path,
            reason,
        });
    }
    Ok(doc)
}

/// Refuse a file that sets a field its layer cannot hold, or names a key kelpie has no setting for
fn holds_only_its_layer(doc: &LayerDoc, layer: Layer) -> Result<(), SettingsError> {
    let refused = |path: String, reason: String| SettingsError::FileRefused {
        file: doc.file.clone(),
        path,
        reason,
    };
    for key in doc.fields.keys() {
        if key == SCHEMA_KEY {
            continue;
        }
        match Field::ALL.iter().find(|field| field.key() == key) {
            Some(field) if field.layers().contains(&layer) => {}
            Some(field) => {
                let layers = field
                    .layers()
                    .iter()
                    .map(|layer| layer.name())
                    .collect::<Vec<_>>()
                    .join(" or ");
                return Err(refused(
                    format!("/{key}"),
                    format!("only the {layers} layer can hold it"),
                ));
            }
            None => {
                return Err(refused(
                    format!("/{key}"),
                    "is not a setting kelpie has".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// The schema one field must satisfy
///
/// # Errors
///
/// [`SettingsError::FieldNotInLayer`] when the layer cannot hold the field, and
/// [`SettingsError::FieldNotInSchema`] when the schema has no place for it.
pub(crate) fn field_schema(field: Field, layer: Layer) -> Result<Value, SettingsError> {
    if !field.layers().contains(&layer) {
        return Err(SettingsError::FieldNotInLayer { field, layer });
    }
    document()
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get(field.key()))
        .cloned()
        .ok_or(SettingsError::FieldNotInSchema { field })
}

/// The first rule in `rule` that `value` breaks, as where it is and what it must be
pub(crate) fn check(rule: &Value, value: &Value, path: &str) -> Option<(String, String)> {
    let broke = |reason: String| Some((path.to_string(), reason));
    let Value::Object(rule) = rule else {
        return if rule.is_boolean() {
            broke("kelpie's types never write a schema this bare".to_string())
        } else {
            None
        };
    };
    for key in rule.keys() {
        if !KNOWN.contains(&key.as_str()) {
            return broke(format!(
                "the schema uses `{key}`, which kelpie's check does not know"
            ));
        }
    }
    if let Some(reference) = rule.get("$ref").and_then(Value::as_str) {
        let other = rule
            .keys()
            .any(|key| key != "$ref" && !DESCRIPTION.contains(&key.as_str()));
        if other {
            return broke(format!(
                "the schema points at {reference} with a rule beside it"
            ));
        }
        return document()
            .pointer(reference.trim_start_matches('#'))
            .and_then(|target| check(target, value, path));
    }
    if let Some(kinds) = rule.get("type") {
        let names = match kinds {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => return broke("the schema's `type` is not a name".to_string()),
        };
        if !names.iter().any(|kind| is_type(kind, value)) {
            return broke(format!("must be {}", names.join(" or ")));
        }
    }
    if let Some(allowed) = rule.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        let names = allowed
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return broke(format!("must be one of: {names}"));
    }
    if let Some(only) = rule.get("const")
        && value != only
    {
        return broke(format!("must be {only}"));
    }
    for (keyword, exactly_one) in [("anyOf", false), ("oneOf", true)] {
        let Some(Value::Array(branches)) = rule.get(keyword) else {
            continue;
        };
        let reasons: Vec<String> = branches
            .iter()
            .filter_map(|branch| check(branch, value, path))
            .map(|(_, reason)| reason)
            .collect();
        let matched = branches.len() - reasons.len();
        let enough = if exactly_one {
            matched == 1
        } else {
            matched > 0
        };
        if !enough {
            let why = if reasons.is_empty() {
                "matches more than one form".to_string()
            } else {
                reasons.join(", or ")
            };
            return broke(why);
        }
    }
    if let Value::String(text) = value {
        let length = text.chars().count();
        if let Some(max) = rule.get("maxLength").and_then(Value::as_u64)
            && u64::try_from(length).is_ok_and(|length| length > max)
        {
            return broke(format!("is longer than {max}"));
        }
        if let Some(min) = rule.get("minLength").and_then(Value::as_u64)
            && u64::try_from(length).is_ok_and(|length| length < min)
        {
            return broke(format!("is shorter than {min}"));
        }
    }
    if let Value::Array(items) = value {
        if let Some(item_rule) = rule.get("items") {
            for (index, item) in items.iter().enumerate() {
                if let Some(bad) = check(item_rule, item, &format!("{path}/{index}")) {
                    return Some(bad);
                }
            }
        }
        if rule.get("uniqueItems") == Some(&Value::Bool(true)) {
            for (index, item) in items.iter().enumerate() {
                if items[..index].contains(item) {
                    return broke(format!("repeats {item} at /{index}"));
                }
            }
        }
    }
    if let Value::Number(number) = value {
        let real = number.as_f64();
        if let Some(max) = rule.get("maximum").and_then(Value::as_f64)
            && real.is_some_and(|number| number > max)
        {
            return broke(format!("is more than {max}"));
        }
        if let Some(min) = rule.get("minimum").and_then(Value::as_f64)
            && real.is_some_and(|number| number < min)
        {
            return broke(format!("is less than {min}"));
        }
    }
    if let Value::Object(fields) = value {
        let properties = rule.get("properties").and_then(Value::as_object);
        for (key, nested) in fields {
            let path = format!("{path}/{key}");
            match properties.and_then(|properties| properties.get(key)) {
                Some(nested_rule) => {
                    if let Some(bad) = check(nested_rule, nested, &path) {
                        return Some(bad);
                    }
                }
                None => match rule.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        return broke(format!("{path} is not a setting kelpie has"));
                    }
                    Some(any_other @ Value::Object(_)) => {
                        if let Some(bad) = check(any_other, nested, &path) {
                            return Some(bad);
                        }
                    }
                    _ => {}
                },
            }
        }
        if let Some(Value::Array(required)) = rule.get("required") {
            for key in required.iter().filter_map(Value::as_str) {
                if !fields.contains_key(key) {
                    return broke(format!("{path}/{key} is needed"));
                }
            }
        }
    }
    None
}

/// Whether `value` is one of the types JSON Schema names
fn is_type(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => false,
    }
}

/// The file's text, or `None` when the file is not there
fn read_if_exists(file: &std::path::Path) -> Result<Option<String>, SettingsError> {
    match fs::read_to_string(file) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SettingsError::Read {
            file: file.to_path_buf(),
            source,
        }),
    }
}

fn unparsed(file: &std::path::Path, line: usize, column: usize, message: &str) -> SettingsError {
    SettingsError::Unparsed {
        file: file.to_path_buf(),
        line,
        column,
        message: message.to_string(),
    }
}

/// Every keyword the schema uses on a schema node, for the test that keeps
/// [`KNOWN`] and the schema in step
#[cfg(test)]
pub(crate) fn keywords(node: &Value, into: &mut Vec<String>) {
    let Value::Object(node) = node else { return };
    for (key, value) in node {
        into.push(key.clone());
        match key.as_str() {
            "$defs" | "properties" => {
                if let Value::Object(map) = value {
                    map.values().for_each(|nested| keywords(nested, into));
                }
            }
            "allOf" | "anyOf" | "oneOf" => {
                if let Value::Array(list) = value {
                    list.iter().for_each(|nested| keywords(nested, into));
                }
            }
            "additionalProperties" | "items" | "not" => keywords(value, into),
            _ => {}
        }
    }
}
