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

/// True if the text names a requirement, an acceptance criterion or a decision of the repository.
fn holds_internal_id(text: &str) -> bool {
    const PREFIXES: [&str; 4] = ["FR-", "AC-", "ADR-", "NFR-"];
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter_map(|token| {
            let prefix = PREFIXES.iter().find(|prefix| token.starts_with(**prefix))?;
            Some(&token[prefix.len()..])
        })
        .any(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
                && rest.chars().any(|c| c.is_ascii_digit())
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
fn no_description_holds_an_internal_id() {
    let document = document();
    for text in descriptions(&document) {
        assert!(
            !holds_internal_id(&text),
            "a description names an internal identifier: {text}"
        );
    }
}

#[test]
fn the_time_fields_are_date_time_strings() {
    let document = document();
    let mut found = 0;
    fn walk(node: &Value, found: &mut usize) {
        match node {
            Value::Object(object) => {
                if let Some(Value::Object(properties)) = object.get("properties") {
                    for (name, property) in properties {
                        let Some(kind) = property.get("type") else {
                            continue;
                        };
                        let is_string = kind == "string"
                            || kind
                                == &Value::Array(vec![
                                    Value::String("string".to_owned()),
                                    Value::String("null".to_owned()),
                                ]);
                        if is_string && name.ends_with("_at") {
                            assert_eq!(
                                property.get("format").and_then(Value::as_str),
                                Some("date-time"),
                                "the field `{name}` is a time without the `date-time` format"
                            );
                            *found += 1;
                        }
                    }
                }
                for value in object.values() {
                    walk(value, found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    walk(&document["components"]["schemas"], &mut found);
    assert!(found >= 50, "only {found} time fields are covered");
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
