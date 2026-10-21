//! A strict reader of the Prometheus text exposition format and the scrape helpers of the
//! metrics tests.
//!
//! `promtool` is not a build dependency, so the reader checks the family declarations, the sample
//! syntax, the label set and the histogram invariants that `promtool check metrics` checks.

#![allow(clippy::expect_used, dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use axum::Router;
use axum::http::{StatusCode, header};
use http_body_util::BodyExt as _;

use crate::queue_support::Env;

#[derive(Debug)]
pub struct Sample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

pub struct Exposition {
    pub types: BTreeMap<String, String>,
    pub samples: Vec<Sample>,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Reads the labels of `{a="b",c="d"}`. A quote or a backslash in a value is escaped.
fn parse_labels(text: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (name, after) = rest.split_once("=\"").expect("a label has a quoted value");
        assert!(valid_name(name) && !name.contains(':'), "label name {name}");
        let mut value = String::new();
        let mut chars = after.char_indices();
        let end = loop {
            let (index, character) = chars.next().expect("the label value ends");
            match character {
                '\\' => {
                    let (_, escaped) = chars.next().expect("an escape follows");
                    assert!(matches!(escaped, '\\' | '"' | 'n'), "escape {escaped}");
                    value.push(escaped);
                }
                '"' => break index,
                other => value.push(other),
            }
        };
        assert!(
            labels.insert(name.to_owned(), value).is_none(),
            "duplicate label {name}"
        );
        rest = &after[end + 1..];
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
    labels
}

pub fn parse(text: &str) -> Exposition {
    assert!(text.ends_with('\n'), "the text ends with a line break");
    let mut types = BTreeMap::new();
    let mut helped = BTreeSet::new();
    let mut samples = Vec::new();
    for line in text.lines() {
        assert!(!line.is_empty(), "no empty line");
        if let Some(rest) = line.strip_prefix("# HELP ") {
            let (name, help) = rest.split_once(' ').expect("a help text");
            assert!(valid_name(name) && !help.is_empty());
            assert!(helped.insert(name.to_owned()), "one HELP for {name}");
        } else if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').expect("a type");
            assert!(valid_name(name));
            assert!(
                ["counter", "gauge", "histogram", "summary", "untyped"].contains(&kind),
                "type {kind}"
            );
            assert!(
                types.insert(name.to_owned(), kind.to_owned()).is_none(),
                "one TYPE for {name}"
            );
        } else {
            assert!(!line.starts_with('#'), "unknown comment {line}");
            let (series, value) = line.rsplit_once(' ').expect("a value");
            let value: f64 = value.parse().expect("a numeric value");
            let (name, labels) = match series.split_once('{') {
                Some((name, rest)) => (
                    name,
                    parse_labels(rest.strip_suffix('}').expect("a closing brace")),
                ),
                None => (series, BTreeMap::new()),
            };
            assert!(valid_name(name), "metric name {name}");
            samples.push(Sample {
                name: name.to_owned(),
                labels,
                value,
            });
        }
    }
    // Every sample belongs to a declared family.
    for sample in &samples {
        let family = ["_bucket", "_sum", "_count"]
            .iter()
            .find_map(|suffix| {
                sample
                    .name
                    .strip_suffix(suffix)
                    .filter(|base| types.get(*base).is_some_and(|kind| kind == "histogram"))
            })
            .unwrap_or(&sample.name);
        assert!(types.contains_key(family), "no TYPE for {}", sample.name);
        assert!(helped.contains(family), "no HELP for {}", sample.name);
    }
    // Counters end with _total.
    for (name, kind) in &types {
        if kind == "counter" {
            assert!(name.ends_with("_total"), "counter {name}");
        }
    }
    Exposition { types, samples }
}

type Series = Vec<(String, String)>;

pub fn check_histograms(exposition: &Exposition, family: &str) {
    let mut groups: BTreeMap<Series, Vec<(f64, f64)>> = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for sample in &exposition.samples {
        let mut labels: Vec<_> = sample
            .labels
            .iter()
            .filter(|(name, _)| *name != "le")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        labels.sort();
        if sample.name == format!("{family}_bucket") {
            let bound = match sample.labels["le"].as_str() {
                "+Inf" => f64::INFINITY,
                other => other.parse().expect("a numeric bound"),
            };
            groups
                .entry(labels)
                .or_default()
                .push((bound, sample.value));
        } else if sample.name == format!("{family}_count") {
            counts.insert(labels, sample.value);
        }
    }
    for (labels, buckets) in groups {
        assert!(
            buckets
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0 && pair[0].1 <= pair[1].1),
            "the buckets are sorted and cumulative: {buckets:?}"
        );
        let last = buckets.last().expect("a bucket");
        assert!(last.0.is_infinite(), "the last bucket is +Inf");
        assert_eq!(Some(&last.1), counts.get(&labels), "+Inf equals the count");
    }
}

pub async fn scrape(env: &Env, token: Option<&str>) -> (StatusCode, Option<String>, String) {
    scrape_app(&env.app, token).await
}

pub async fn scrape_app(app: &Router, token: Option<&str>) -> (StatusCode, Option<String>, String) {
    let request = match token {
        Some(token) => crate::common::bearer_request("GET", "/metrics", token, None),
        None => {
            let mut request = axum::http::Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(axum::body::Body::empty())
                .expect("the request");
            request.extensions_mut().insert(crate::common::peer());
            request
        }
    };
    let response = crate::common::send_response(app, request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).expect("UTF-8 text"),
    )
}

/// Pipes the exposition through the real `promtool check metrics` when the CI job provides it.
pub fn check_with_promtool(text: &str) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let Some(promtool) = std::env::var_os("RESTICHUB_TEST_PROMTOOL") else {
        assert!(
            std::env::var_os("RESTICHUB_REQUIRE_PROMTOOL").is_none(),
            "RESTICHUB_REQUIRE_PROMTOOL is set but RESTICHUB_TEST_PROMTOOL is not"
        );
        return;
    };
    let mut child = Command::new(promtool)
        .args(["check", "metrics"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start promtool");
    child
        .stdin
        .take()
        .expect("the stdin")
        .write_all(text.as_bytes())
        .expect("cannot write to promtool");
    let output = child.wait_with_output().expect("promtool must finish");
    assert!(
        output.status.success(),
        "promtool rejected the exposition: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn labelled<'a>(exposition: &'a Exposition, name: &'a str) -> impl Iterator<Item = &'a Sample> {
    exposition
        .samples
        .iter()
        .filter(move |sample| sample.name == name)
}
