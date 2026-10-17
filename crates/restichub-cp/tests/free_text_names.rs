//! Free-text names of profiles, plans and alert channels (FR-PF-01, FR-SC-01, FR-MN-04).
//!
//! A name is any printable text of 1 to 100 characters. The control and format characters are
//! refused. The name is unique for the tenant, ignoring the letter case.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_profile, seed_repository,
    seed_secret, seed_session, send,
};

const CYRILLIC: &str = "Ночной бэкап";
const WITH_EMOJI: &str = "Nightly 📦 backup";

struct Fixture {
    app: Router,
    cookie: String,
    repository_id: String,
    profile_id: String,
    agent_id: String,
    _db: restichub_db::test_util::TestDatabase,
}

async fn fixture() -> Option<Fixture> {
    let db = common::test_db().await?;
    let tenant = create_tenant_with_key(db.database(), "names").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let secret = seed_secret(db.database(), tenant, "names.password").await;
    let repository = seed_repository(db.database(), tenant, "names-repo", secret).await;
    let profile = seed_profile(db.database(), tenant, "names-profile", repository).await;
    let agent = seed_agent(db.database(), tenant, "names-host").await;
    Some(Fixture {
        app,
        cookie,
        repository_id: repository.to_string(),
        profile_id: profile.to_string(),
        agent_id: agent.to_string(),
        _db: db,
    })
}

impl Fixture {
    fn profile(&self, name: &str) -> Value {
        json!({
            "name": name,
            "repository_id": self.repository_id,
            "paths": {"all": ["/srv"]},
        })
    }

    fn plan(&self, name: &str) -> Value {
        json!({
            "name": name,
            "profile_id": self.profile_id,
            "agent_id": self.agent_id,
            "cron": "0 2 * * *",
            "time_zone": "UTC",
            "missed_run_policy": "run_all",
        })
    }

    fn channel(name: &str) -> Value {
        json!({"name": name, "kind": "email", "recipients": ["ops@example.com"]})
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let (status, _, response) = send(
            &self.app,
            request("POST", path, Some(&self.cookie), Some(body)),
        )
        .await;
        (status, response)
    }
}

type Body = fn(&Fixture, &str) -> Value;

/// The three create endpoints with a body builder for each.
fn kinds() -> [(&'static str, Body); 3] {
    [
        ("/api/v1/profiles", Fixture::profile),
        ("/api/v1/plans", Fixture::plan),
        ("/api/v1/notification-channels", |_, name| {
            Fixture::channel(name)
        }),
    ]
}

/// FR-PF-01: a name with Cyrillic letters, spaces and an emoji is accepted, trimmed, and returned
/// unchanged in the view and in the audit record.
#[tokio::test]
async fn fr_pf_01_a_name_is_free_text() {
    let Some(f) = fixture().await else {
        return;
    };
    for (index, (path, body)) in kinds().into_iter().enumerate() {
        for (offset, name) in [CYRILLIC, WITH_EMOJI, "  padded  "].into_iter().enumerate() {
            let unique = format!("{name} {index}{offset}");
            let (status, response) = f.post(path, body(&f, &unique)).await;
            assert_eq!(status, StatusCode::CREATED, "{path}: {response}");
            let stored = response
                .as_object()
                .and_then(|map| map.values().find(|value| value["name"].is_string()))
                .unwrap_or(&response);
            assert_eq!(stored["name"], unique.trim(), "{path}: {response}");
        }
    }
    let (status, _, audit) = send(
        &f.app,
        request("GET", "/api/v1/audit", Some(&f.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{audit}");
    assert!(audit.to_string().contains(CYRILLIC), "{audit}");
}

/// FR-PF-01: the length is counted in characters, from 1 to 100.
#[tokio::test]
async fn fr_pf_01_a_name_holds_one_to_one_hundred_characters() {
    let Some(f) = fixture().await else {
        return;
    };
    for (path, body) in kinds() {
        let (status, response) = f.post(path, body(&f, &"я".repeat(100))).await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {response}");

        let (status, response) = f.post(path, body(&f, &"ю".repeat(101))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {response}");
        assert_eq!(response["field"], "name");
        assert_eq!(response["reason"], "too_long");

        for empty in ["", "   "] {
            let (status, response) = f.post(path, body(&f, empty)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {response}");
            assert_eq!(response["reason"], "empty");
        }
    }
}

/// FR-PR-10: a control character and a bidirectional override are refused.
#[tokio::test]
async fn fr_pr_10_a_name_with_a_control_or_format_character_is_refused() {
    let Some(f) = fixture().await else {
        return;
    };
    for (path, body) in kinds() {
        for bad in [
            "line\nbreak",
            "tab\there",
            "bidi \u{202E}override",
            "isolate \u{2066}x",
            "zero\u{200B}width",
        ] {
            let (status, response) = f.post(path, body(&f, bad)).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{path} {bad:?}: {response}"
            );
            assert_eq!(response["field"], "name");
            assert_eq!(response["reason"], "invalid_character");
        }
    }
}

/// FR-PF-01: the name is unique for the tenant and the comparison ignores the letter case, also
/// for Cyrillic letters.
#[tokio::test]
async fn fr_pf_01_a_name_is_unique_ignoring_the_letter_case() {
    let Some(f) = fixture().await else {
        return;
    };
    for (path, body) in kinds() {
        let (status, response) = f.post(path, body(&f, "Ночной Бэкап")).await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {response}");
        for same in ["ночной бэкап", "НОЧНОЙ БЭКАП", " Ночной Бэкап "]
        {
            let (status, response) = f.post(path, body(&f, same)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{path} {same}: {response}");
        }
        let (status, response) = f.post(path, body(&f, "Ночной Бэкап 2")).await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {response}");
    }
}

/// FR-PF-01: a restic argument, a tag and a path hold the identifier of the profile, never its
/// name.
#[tokio::test]
async fn fr_pf_01_the_name_of_a_profile_is_not_in_the_preview() {
    let Some(f) = fixture().await else {
        return;
    };
    let name = "Секретное имя \"$(whoami)\" `id` ; rm";
    let (status, response) = f.post("/api/v1/profiles", f.profile(name)).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    let id = response["profile"]["id"].as_str().expect("the id");
    let (status, _, preview) = send(
        &f.app,
        request(
            "POST",
            &format!("/api/v1/profiles/{id}/preview"),
            Some(&f.cookie),
            Some(json!({"agent_id": f.agent_id})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let steps = preview["steps"].to_string();
    assert!(!steps.contains("Секретное"), "{steps}");
    assert!(!steps.contains("whoami"), "{steps}");
}
