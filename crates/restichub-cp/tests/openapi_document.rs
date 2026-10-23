//! Tests of the shape of the OpenAPI document: the tags, the descriptions, the formats and the
//! error responses. The API reference renders the document, so a missing tag or an internal
//! identifier is a visible defect.

#![allow(clippy::expect_used)]

use serde_json::Value;

/// Builds the document through `openapi_document()` and serializes it to a JSON value.
fn document() -> Value {
    serde_json::to_value(restichub_cp::server::openapi_document()).expect("the document is JSON")
}

/// Every operation with its method, its path and the operation object.
fn operations(document: &Value) -> Vec<(String, String, &Value)> {
    let mut found = Vec::new();
    for (path, item) in document["paths"].as_object().expect("paths").iter() {
        for (method, operation) in item.as_object().expect("a path item").iter() {
            found.push((method.clone(), path.clone(), operation));
        }
    }
    found
}

/// True if the text holds a reference that only the repository uses: a requirement, an acceptance
/// criterion, a decision, a requirements section, a threat model entry, a rustdoc link or the name
/// of an unfinished stage.
fn holds_internal_reference(text: &str) -> bool {
    const PREFIXES: [&str; 4] = ["FR-", "AC-", "ADR-", "NFR-"];
    if text.contains('[') || text.contains("MVP") {
        return true;
    }
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .collect();
    tokens.iter().enumerate().any(|(index, token)| {
        // A threat model entry is `T` and a number, for example `T17`.
        let threat_model_entry = token.len() > 1
            && token.starts_with('T')
            && token[1..].chars().all(|c| c.is_ascii_digit());
        // A requirements section is the word and a number, for example `section 2`.
        let requirements_section = *token == "section"
            && tokens
                .get(index + 1)
                .is_some_and(|next| !next.is_empty() && next.chars().all(|c| c.is_ascii_digit()));
        let named_id = PREFIXES
            .iter()
            .find(|prefix| token.starts_with(**prefix))
            .is_some_and(|prefix| {
                let rest = &token[prefix.len()..];
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
                    && rest.chars().any(|c| c.is_ascii_digit())
            });
        named_id || threat_model_entry || requirements_section
    })
}

fn descriptions(document: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    fn walk(node: &Value, texts: &mut Vec<String>) {
        match node {
            Value::Object(object) => {
                for (key, value) in object {
                    if let Value::String(text) = value
                        && (key == "description" || key == "summary")
                    {
                        texts.push(text.clone());
                    }
                    walk(value, texts);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, texts);
                }
            }
            Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    walk(document, &mut texts);
    texts
}

#[test]
fn every_operation_carries_one_declared_tag() {
    let document = document();
    let declared: Vec<&str> = document["tags"]
        .as_array()
        .expect("the document declares tags")
        .iter()
        .map(|tag| tag["name"].as_str().expect("a tag name"))
        .collect();
    for name in &declared {
        let tag = document["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .find(|tag| tag["name"].as_str() == Some(name))
            .expect("the tag");
        assert!(
            tag["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "the tag `{name}` has no description"
        );
    }
    let mut used = Vec::new();
    for (method, path, operation) in operations(&document) {
        let tags = operation["tags"].as_array().unwrap_or_else(|| {
            panic!("{method} {path} has no tag");
        });
        assert_eq!(tags.len(), 1, "{method} {path} has more than one tag");
        let tag = tags[0].as_str().expect("a tag");
        assert!(
            declared.contains(&tag),
            "{method} {path} carries the undeclared tag `{tag}`"
        );
        used.push(tag.to_owned());
    }
    for name in &declared {
        assert!(
            used.iter().any(|tag| tag == name),
            "the tag `{name}` groups no operation"
        );
    }
}

#[test]
fn no_description_holds_an_internal_reference() {
    let document = document();
    for text in descriptions(&document) {
        assert!(
            !holds_internal_reference(&text),
            "a description holds an internal reference: {text}"
        );
    }
}

/// The names that the document gives to a time without the `_at` suffix: a log line, a snapshot and
/// a tree entry, the fire times of a plan and the ends of a time range.
const TIME_NAMES: [&str; 6] = ["ts", "time", "mtime", "runs", "from", "to"];

fn is_time_name(name: &str) -> bool {
    TIME_NAMES.contains(&name) || name.ends_with("_at") || name.ends_with("_time")
}

/// True for a schema of `string`, of `string` or null, and of an array of one of these.
fn is_string_schema(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => kind == "string",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "string"),
        _ => false,
    }
}

/// The schema that holds the text of a time: the string itself or the items of the array.
fn time_schema(schema: &Value) -> Option<&Value> {
    if is_string_schema(schema) {
        return Some(schema);
    }
    let items = schema.get("items")?;
    is_string_schema(items).then_some(items)
}

/// Checks that the documented time holds a string, and that the string is a `date-time`.
fn assert_date_time(location: &str, name: &str, schema: &Value, found: &mut usize) {
    let text = time_schema(schema)
        .unwrap_or_else(|| panic!("{location}: the time `{name}` is not a string: {schema}"));
    assert_eq!(
        text.get("format").and_then(Value::as_str),
        Some("date-time"),
        "{location}: the time `{name}` has no `date-time` format"
    );
    *found += 1;
}

#[test]
fn the_time_fields_and_parameters_are_date_time_strings() {
    let document = document();
    let mut found = 0;
    fn walk(node: &Value, schema: &str, found: &mut usize) {
        match node {
            Value::Object(object) => {
                if let Some(Value::Object(properties)) = object.get("properties") {
                    for (name, property) in properties {
                        if is_time_name(name) {
                            assert_date_time(schema, name, property, found);
                        }
                    }
                }
                for value in object.values() {
                    walk(value, schema, found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, schema, found);
                }
            }
            Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("the schemas");
    for (name, schema) in schemas {
        walk(schema, name, &mut found);
    }
    for (method, path, operation) in operations(&document) {
        let Some(parameters) = operation["parameters"].as_array() else {
            continue;
        };
        for parameter in parameters {
            let name = parameter["name"].as_str().unwrap_or_default();
            if is_time_name(name) {
                let location = format!("{method} {path}");
                assert_date_time(&location, name, &parameter["schema"], &mut found);
            }
        }
    }
    assert!(found >= 70, "only {found} times are covered");
}

#[test]
fn an_error_response_is_a_problem() {
    let document = document();
    let mut found = 0;
    for (method, path, operation) in operations(&document) {
        let responses = operation["responses"]
            .as_object()
            .unwrap_or_else(|| panic!("{method} {path} has no responses"));
        for (status, response) in responses {
            let client_or_server_error = status.starts_with('4') || status.starts_with('5');
            if !client_or_server_error {
                continue;
            }
            let content = &response["content"];
            assert!(
                content["application/json"].is_null(),
                "{method} {path} answers {status} with `application/json`"
            );
            assert!(
                !content["application/problem+json"].is_null(),
                "{method} {path} answers {status} without a problem body"
            );
            found += 1;
        }
    }
    assert!(found >= 150, "only {found} error responses are covered");
}

#[test]
fn every_one_of_variant_has_a_title() {
    let document = document();
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("the schemas");
    let mut variants = 0;
    for (name, schema) in schemas {
        let Some(one_of) = schema["oneOf"].as_array() else {
            continue;
        };
        for variant in one_of {
            assert!(
                variant["title"]
                    .as_str()
                    .is_some_and(|title| !title.is_empty()),
                "a variant of `{name}` has no title"
            );
            variants += 1;
        }
    }
    assert!(variants >= 10, "only {variants} variants are covered");
}

#[test]
fn the_server_is_relative() {
    let document = document();
    let servers = document["servers"].as_array().expect("the servers");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["url"], "/");
}
