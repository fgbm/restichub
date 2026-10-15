//! Audit log operations (ADR-0023, section Audit log; FR-AU-01, AC-AU-01).
//!
//! The table is append-only. This module gives the insert and the list functions only. No function
//! changes or removes an audit record (AC-AU-01).
//!
//! A tenant-scoped transaction writes and reads the records of its own tenant. A platform
//! transaction reads every record. A platform write names the entered tenant (a tenant created in
//! the same transaction), the platform tenant, or an existing tenant through the GUC
//! `restichub.audit_tenant_id`. The policy `audit_log_platform_insert` of migration 0014 accepts
//! these three cases only.
//!
//! The module also holds the diff builder and the [`RedactedDiff`] type. The type keeps its value
//! private. Only [`diff`] and [`RedactedDiff::empty`] build a value. A
//! caller can therefore not write a diff that holds a secret.

use std::collections::BTreeSet;
use std::net::IpAddr;

use serde_json::{Map, Value};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::ActorType;
use crate::ids::{ApiTokenId, AuditLogId, SecretId, TenantId, UserId};
use crate::tx::{PlatformTx, TenantTx};

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

pub const REDACTED: &str = "<redacted>";

/// Last year of a valid RFC 3339 time.
const MAX_YEAR: i32 = 9999;

/// A field whose name holds one of these words is sensitive.
const SENSITIVE_KEYS: [&str; 5] = ["password", "secret", "token", "key", "private_key"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    User(UserId),
    ApiToken(ApiTokenId),
    Agent(Uuid),
    System,
    /// A type from a newer version; the list keeps the row instead of failing.
    Unknown {
        actor_type: String,
        actor_id: Option<Uuid>,
    },
}

impl Actor {
    pub fn actor_type(&self) -> &str {
        match self {
            Self::User(_) => ActorType::User.as_str(),
            Self::ApiToken(_) => ActorType::ApiToken.as_str(),
            Self::Agent(_) => ActorType::Agent.as_str(),
            Self::System => ActorType::System.as_str(),
            Self::Unknown { actor_type, .. } => actor_type,
        }
    }

    pub fn actor_id(&self) -> Option<Uuid> {
        match self {
            Self::User(id) => Some(id.as_uuid()),
            Self::ApiToken(id) => Some(id.as_uuid()),
            Self::Agent(id) => Some(*id),
            Self::System => None,
            Self::Unknown { actor_id, .. } => *actor_id,
        }
    }

    /// The function keeps an unknown type or a missing id as [`Actor::Unknown`]. A list therefore
    /// never fails on a row of a newer schema.
    fn from_parts(actor_type: String, actor_id: Option<Uuid>) -> Self {
        let known = match ActorType::parse(&actor_type) {
            Some(ActorType::System) => Some(Self::System),
            Some(ActorType::User) => actor_id.map(|id| Self::User(UserId::from_uuid(id))),
            Some(ActorType::ApiToken) => {
                actor_id.map(|id| Self::ApiToken(ApiTokenId::from_uuid(id)))
            }
            Some(ActorType::Agent) => actor_id.map(Self::Agent),
            None => None,
        };
        known.unwrap_or(Self::Unknown {
            actor_type,
            actor_id,
        })
    }
}

/// The string of each action is stable. Do not change a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditAction {
    SignIn,
    SignInFailed,
    SignOut,
    SessionSwitch,
    ConfigCreate,
    ConfigUpdate,
    ConfigDelete,
    JobRun,
    JobCancel,
    JobDeadLetter,
    RestoreStart,
    BrowseStart,
    SecretDelivered,
    AgentEnroll,
    AgentRevoke,
    AgentUpdate,
    RemoteInstall,
    TenantCreate,
    TenantSuspend,
    TenantResume,
    TenantDelete,
    UserCreate,
    UserUpdate,
    UserDelete,
    ApiTokenCreate,
    ApiTokenRevoke,
    EnrollmentTokenCreate,
    EnrollmentTokenRevoke,
    EnrollmentTokenUse,
    EnrollmentTokenRefuse,
    SecretCreate,
    SecretReplace,
    SecretDelete,
    RepositoryCreate,
    RepositoryUpdate,
    RepositoryDelete,
    RepositoryUnlockAll,
    ProfileCreate,
    ProfileUpdate,
    ProfileDelete,
    PlanCreate,
    PlanUpdate,
    PlanDelete,
    SigningKeyRotate,
    SigningKeyRetire,
    /// An action from a newer version; the list keeps the row instead of failing.
    Unknown(String),
}

impl AuditAction {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SignIn => "sign_in",
            Self::SignInFailed => "sign_in_failed",
            Self::SignOut => "sign_out",
            Self::SessionSwitch => "session_switch",
            Self::ConfigCreate => "config_create",
            Self::ConfigUpdate => "config_update",
            Self::ConfigDelete => "config_delete",
            Self::JobRun => "job_run",
            Self::JobCancel => "job_cancel",
            Self::JobDeadLetter => "job_dead_letter",
            Self::RestoreStart => "restore_start",
            Self::BrowseStart => "browse_start",
            Self::SecretDelivered => "secret_delivered",
            Self::AgentEnroll => "agent_enroll",
            Self::AgentRevoke => "agent_revoke",
            Self::AgentUpdate => "agent_update",
            Self::RemoteInstall => "remote_install",
            Self::TenantCreate => "tenant_create",
            Self::TenantSuspend => "tenant_suspend",
            Self::TenantResume => "tenant_resume",
            Self::TenantDelete => "tenant_delete",
            Self::UserCreate => "user_create",
            Self::UserUpdate => "user_update",
            Self::UserDelete => "user_delete",
            Self::ApiTokenCreate => "api_token_create",
            Self::ApiTokenRevoke => "api_token_revoke",
            Self::EnrollmentTokenCreate => "enrollment_token_create",
            Self::EnrollmentTokenRevoke => "enrollment_token_revoke",
            Self::EnrollmentTokenUse => "enrollment_token_use",
            Self::EnrollmentTokenRefuse => "enrollment_token_refuse",
            Self::SecretCreate => "secret_create",
            Self::SecretReplace => "secret_replace",
            Self::SecretDelete => "secret_delete",
            Self::RepositoryCreate => "repository_create",
            Self::RepositoryUpdate => "repository_update",
            Self::RepositoryDelete => "repository_delete",
            Self::RepositoryUnlockAll => "repository_unlock_all",
            Self::ProfileCreate => "profile_create",
            Self::ProfileUpdate => "profile_update",
            Self::ProfileDelete => "profile_delete",
            Self::PlanCreate => "plan_create",
            Self::PlanUpdate => "plan_update",
            Self::PlanDelete => "plan_delete",
            Self::SigningKeyRotate => "signing_key_rotate",
            Self::SigningKeyRetire => "signing_key_retire",
            Self::Unknown(value) => value,
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        let action = match value {
            "sign_in" => Self::SignIn,
            "sign_in_failed" => Self::SignInFailed,
            "sign_out" => Self::SignOut,
            "session_switch" => Self::SessionSwitch,
            "config_create" => Self::ConfigCreate,
            "config_update" => Self::ConfigUpdate,
            "config_delete" => Self::ConfigDelete,
            "job_run" => Self::JobRun,
            "job_cancel" => Self::JobCancel,
            "job_dead_letter" => Self::JobDeadLetter,
            "restore_start" => Self::RestoreStart,
            "browse_start" => Self::BrowseStart,
            "secret_delivered" => Self::SecretDelivered,
            "agent_enroll" => Self::AgentEnroll,
            "agent_revoke" => Self::AgentRevoke,
            "agent_update" => Self::AgentUpdate,
            "remote_install" => Self::RemoteInstall,
            "tenant_create" => Self::TenantCreate,
            "tenant_suspend" => Self::TenantSuspend,
            "tenant_resume" => Self::TenantResume,
            "tenant_delete" => Self::TenantDelete,
            "user_create" => Self::UserCreate,
            "user_update" => Self::UserUpdate,
            "user_delete" => Self::UserDelete,
            "api_token_create" => Self::ApiTokenCreate,
            "api_token_revoke" => Self::ApiTokenRevoke,
            "enrollment_token_create" => Self::EnrollmentTokenCreate,
            "enrollment_token_revoke" => Self::EnrollmentTokenRevoke,
            "enrollment_token_use" => Self::EnrollmentTokenUse,
            "enrollment_token_refuse" => Self::EnrollmentTokenRefuse,
            "secret_create" => Self::SecretCreate,
            "secret_replace" => Self::SecretReplace,
            "secret_delete" => Self::SecretDelete,
            "repository_create" => Self::RepositoryCreate,
            "repository_update" => Self::RepositoryUpdate,
            "repository_delete" => Self::RepositoryDelete,
            "repository_unlock_all" => Self::RepositoryUnlockAll,
            "profile_create" => Self::ProfileCreate,
            "profile_update" => Self::ProfileUpdate,
            "profile_delete" => Self::ProfileDelete,
            "plan_create" => Self::PlanCreate,
            "plan_update" => Self::PlanUpdate,
            "plan_delete" => Self::PlanDelete,
            "signing_key_rotate" => Self::SigningKeyRotate,
            "signing_key_retire" => Self::SigningKeyRetire,
            _ => return None,
        };
        Some(action)
    }

    /// Keeps an unknown string.
    fn from_db(value: String) -> Self {
        Self::parse(&value).unwrap_or(Self::Unknown(value))
    }

    /// The list is in the stored order and excludes [`AuditAction::Unknown`]. The OpenAPI schema and
    /// the audit query use it.
    pub fn known() -> &'static [Self] {
        &[
            Self::SignIn,
            Self::SignInFailed,
            Self::SignOut,
            Self::SessionSwitch,
            Self::ConfigCreate,
            Self::ConfigUpdate,
            Self::ConfigDelete,
            Self::JobRun,
            Self::JobCancel,
            Self::JobDeadLetter,
            Self::RestoreStart,
            Self::BrowseStart,
            Self::SecretDelivered,
            Self::AgentEnroll,
            Self::AgentRevoke,
            Self::AgentUpdate,
            Self::RemoteInstall,
            Self::TenantCreate,
            Self::TenantSuspend,
            Self::TenantResume,
            Self::TenantDelete,
            Self::UserCreate,
            Self::UserUpdate,
            Self::UserDelete,
            Self::ApiTokenCreate,
            Self::ApiTokenRevoke,
            Self::EnrollmentTokenCreate,
            Self::EnrollmentTokenRevoke,
            Self::EnrollmentTokenUse,
            Self::EnrollmentTokenRefuse,
            Self::SecretCreate,
            Self::SecretReplace,
            Self::SecretDelete,
            Self::RepositoryCreate,
            Self::RepositoryUpdate,
            Self::RepositoryDelete,
            Self::RepositoryUnlockAll,
            Self::ProfileCreate,
            Self::ProfileUpdate,
            Self::ProfileDelete,
            Self::SigningKeyRotate,
            Self::SigningKeyRetire,
        ]
    }
}

impl serde::Serialize for AuditAction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for AuditAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| serde::de::Error::custom("the audit action is not known"))
    }
}

impl utoipa::PartialSchema for AuditAction {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::ObjectBuilder;
        use utoipa::openapi::schema::Type;
        // The action is an open string: the database keeps an action that a newer version writes.
        // The known values are examples, not a closed enum, so a strict client does not fail.
        ObjectBuilder::new()
            .schema_type(Type::String)
            .examples(
                Self::known()
                    .iter()
                    .map(|action| serde_json::Value::String(action.as_str().to_owned())),
            )
            .build()
            .into()
    }
}

impl utoipa::ToSchema for AuditAction {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("AuditAction")
    }
}

/// Holds no secret value.
///
/// The type keeps the JSON private. Only [`diff`] and [`RedactedDiff::empty`] build a value. A
/// caller can therefore not pass raw secret data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedDiff(Value);

impl RedactedDiff {
    /// Serialises as `{}`.
    pub fn empty() -> Self {
        Self(Value::Object(Map::new()))
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

/// The top-level and the nested fields are compared. A changed field becomes `{"old": .., "new": ..}`;
/// a missing side is `null`.
///
/// A field is redacted when its name is in `redact` (without the letter case) or holds one of
/// `password`, `secret`, `token`, `key`, `private_key`. A field named `value` is also redacted when
/// the parent object holds a field named `secret`.
///
/// Every emitted value goes through [`redact_value`], so a sensitive field is redacted at any depth:
/// also when one side is not an object, when a key is on one side only, and inside an array. A
/// redacted field becomes `"<redacted>"`. A secret value is never written.
pub fn diff(before: Option<&Value>, after: Option<&Value>, redact: &[&str]) -> RedactedDiff {
    match diff_value(before, after, redact, false) {
        Some(value) => RedactedDiff(value),
        None => RedactedDiff::empty(),
    }
}

/// Returns the diff of two values, or `None` if the values are equal.
///
/// `parent_has_secret` says if the parent object of the two values holds a field named `secret`.
fn diff_value(
    before: Option<&Value>,
    after: Option<&Value>,
    redact: &[&str],
    parent_has_secret: bool,
) -> Option<Value> {
    match (before, after) {
        (Some(Value::Object(before)), Some(Value::Object(after))) => {
            let has_secret = parent_has_secret
                || before
                    .keys()
                    .chain(after.keys())
                    .any(|key| key.eq_ignore_ascii_case("secret"));
            let mut result = Map::new();
            let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
            for key in keys {
                let old = before.get(key);
                let new = after.get(key);
                if is_sensitive(key, redact, has_secret) {
                    if old != new {
                        result.insert(key.clone(), redacted());
                    }
                    continue;
                }
                if let Some(child) = diff_value(old, new, redact, has_secret) {
                    result.insert(key.clone(), child);
                }
            }
            if result.is_empty() {
                None
            } else {
                Some(Value::Object(result))
            }
        }
        _ => {
            if before == after {
                None
            } else {
                Some(changed_redacted(before, after, redact, parent_has_secret))
            }
        }
    }
}

/// Returns a value with every sensitive field at any depth redacted.
///
/// `parent_has_secret` says if the parent object holds a field named `secret`.
pub fn redact_value(value: &Value, redact: &[&str], parent_has_secret: bool) -> Value {
    match value {
        Value::Object(map) => {
            let has_secret =
                parent_has_secret || map.keys().any(|key| key.eq_ignore_ascii_case("secret"));
            let mut result = Map::new();
            for (key, child) in map {
                if is_sensitive(key, redact, has_secret) {
                    result.insert(key.clone(), Value::String(REDACTED.to_owned()));
                } else {
                    result.insert(key.clone(), redact_value(child, redact, has_secret));
                }
            }
            Value::Object(result)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value(item, redact, parent_has_secret))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn is_sensitive(key: &str, redact: &[&str], parent_has_secret: bool) -> bool {
    if redact.iter().any(|name| name.eq_ignore_ascii_case(key)) {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    if SENSITIVE_KEYS.iter().any(|name| lower.contains(name)) {
        return true;
    }
    lower == "value" && parent_has_secret
}

fn redacted() -> Value {
    let mut object = Map::new();
    object.insert("old".to_owned(), Value::String(REDACTED.to_owned()));
    object.insert("new".to_owned(), Value::String(REDACTED.to_owned()));
    Value::Object(object)
}

fn changed_redacted(
    before: Option<&Value>,
    after: Option<&Value>,
    redact: &[&str],
    parent_has_secret: bool,
) -> Value {
    let mut object = Map::new();
    object.insert(
        "old".to_owned(),
        before
            .map(|value| redact_value(value, redact, parent_has_secret))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "new".to_owned(),
        after
            .map(|value| redact_value(value, redact, parent_has_secret))
            .unwrap_or(Value::Null),
    );
    Value::Object(object)
}

#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub actor: Actor,
    pub source_ip: Option<IpAddr>,
    pub action: AuditAction,
    pub object_type: String,
    pub object_id: Option<Uuid>,
    /// Holds no secret.
    pub diff: Option<RedactedDiff>,
}

impl AuditEvent {
    pub fn new(action: AuditAction, object_type: impl Into<String>) -> Self {
        Self {
            actor: Actor::System,
            source_ip: None,
            action,
            object_type: object_type.into(),
            object_id: None,
            diff: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub id: AuditLogId,
    pub tenant_id: TenantId,
    pub occurred_at: OffsetDateTime,
    pub actor: Actor,
    pub source_ip: Option<IpAddr>,
    pub action: AuditAction,
    pub object_type: String,
    pub object_id: Option<Uuid>,
    pub diff: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// The platform list filters by it; a tenant list ignores it, because Row-Level Security shows
    /// its own rows only.
    pub tenant_id: Option<TenantId>,
    /// Start of the time range, inclusive.
    pub from: Option<OffsetDateTime>,
    /// End of the time range, inclusive.
    pub to: Option<OffsetDateTime>,
    pub action: Option<AuditAction>,
    pub actor: Option<Actor>,
    pub object_type: Option<String>,
    pub object_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor {
    pub occurred_at: OffsetDateTime,
    pub id: AuditLogId,
}

#[derive(Debug, Clone, Copy)]
pub struct AuditPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<AuditCursor>,
}

impl Default for AuditPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditPageResult {
    pub records: Vec<AuditRecord>,
    pub has_more: bool,
}

pub async fn insert(tx: &mut TenantTx<'_>, event: &AuditEvent) -> Result<AuditLogId, DbError> {
    let tenant = tx.tenant();
    insert_row(tx.as_conn(), tenant, event).await
}

/// Inserts the record of a secret delivery to an agent (AC-RP-03).
///
/// The record holds the tenant, the agent, the job, the repository and the time. The function
/// takes no secret value, so no secret value can appear in the record. The delivery path
/// [`crate::delivery::load_ciphertext_for_delivery`] and the audit service use this function, so
/// the shape of the record lives in one place.
pub async fn insert_secret_delivered(
    tx: &mut TenantTx<'_>,
    secret_id: SecretId,
    agent_id: Uuid,
    job_id: Uuid,
    repository_id: Uuid,
) -> Result<AuditLogId, DbError> {
    let tenant = tx.tenant();
    let after = serde_json::json!({
        "tenant_id": tenant.to_string(),
        "agent_id": agent_id.to_string(),
        "job_id": job_id.to_string(),
        "repository_id": repository_id.to_string(),
    });
    let mut event = AuditEvent::new(AuditAction::SecretDelivered, "secret");
    event.object_id = Some(secret_id.as_uuid());
    event.diff = Some(diff(None, Some(&after), &[]));
    insert(tx, &event).await
}

/// The policy `audit_log_platform_insert` of migration 0014 accepts the entered tenant, the
/// platform tenant and the tenant of the GUC `restichub.audit_tenant_id`. A created tenant is
/// entered first with `PlatformTx::enter_tenant`; [`insert_platform_for`] is used for an existing
/// tenant.
pub async fn insert_platform(
    tx: &mut PlatformTx<'_>,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<AuditLogId, DbError> {
    insert_row(tx.as_conn(), tenant, event).await
}

/// Inserts a record for an existing tenant in a platform transaction.
///
/// A platform operation on an existing tenant (suspend, resume or delete) uses this function. The
/// function sets the GUC `restichub.audit_tenant_id` for one insert and clears it again, so the
/// transaction keeps no tenant scope after the write. Only the policy `audit_log_platform_insert`
/// of migration 0014 reads the GUC.
pub async fn insert_platform_for(
    tx: &mut PlatformTx<'_>,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<AuditLogId, DbError> {
    set_audit_tenant(tx.as_conn(), Some(tenant)).await?;
    let result = insert_row(tx.as_conn(), tenant, event).await;
    let cleared = set_audit_tenant(tx.as_conn(), None).await;
    match (result, cleared) {
        (Ok(id), Ok(())) => Ok(id),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

async fn set_audit_tenant(
    conn: &mut PgConnection,
    tenant: Option<TenantId>,
) -> Result<(), DbError> {
    let value = tenant.map(|tenant| tenant.to_string()).unwrap_or_default();
    sqlx::query("SELECT set_config('restichub.audit_tenant_id', $1, true)")
        .bind(value)
        .execute(conn)
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

async fn insert_row(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<AuditLogId, DbError> {
    let id = AuditLogId::new();
    sqlx::query(
        "INSERT INTO audit_log \
         (id, tenant_id, actor_type, actor_id, source_ip, action, object_type, object_id, diff) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(tenant)
    .bind(event.actor.actor_type())
    .bind(event.actor.actor_id())
    .bind(event.source_ip)
    .bind(event.action.as_str())
    .bind(&event.object_type)
    .bind(event.object_id)
    .bind(event.diff.as_ref().map(RedactedDiff::as_value))
    .execute(conn)
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

pub async fn list(
    tx: &mut TenantTx<'_>,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    list_rows(tx.as_conn(), filter, page).await
}

pub async fn list_platform(
    tx: &mut PlatformTx<'_>,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    list_rows(tx.as_conn(), filter, page).await
}

/// Lists records with a connection.
///
/// The list orders the records by time and identifier, from the newest to the oldest. It reads one
/// record more than the page holds, to report `has_more` correctly. The keyset cursor of
/// `page.after` continues the list.
async fn list_rows(
    conn: &mut PgConnection,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let action = filter.action.as_ref().map(AuditAction::as_str);
    let actor_type = filter.actor.as_ref().map(Actor::actor_type);
    let actor_id = filter.actor.as_ref().and_then(Actor::actor_id);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.occurred_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let mut records: Vec<AuditRecord> = sqlx::query_as::<_, Row>(
        "SELECT id, tenant_id, occurred_at, actor_type, actor_id, source_ip, \
                action, object_type, object_id, diff \
         FROM audit_log \
         WHERE ($1::timestamptz IS NULL OR occurred_at >= $1) \
           AND ($2::timestamptz IS NULL OR occurred_at <= $2) \
           AND ($3::text IS NULL OR action = $3) \
           AND ($4::text IS NULL OR actor_type = $4) \
           AND ($5::uuid IS NULL OR actor_id = $5) \
           AND ($6::text IS NULL OR object_type = $6) \
           AND ($7::uuid IS NULL OR object_id = $7) \
           AND ($8::timestamptz IS NULL OR (occurred_at, id) < ($8::timestamptz, $9::uuid)) \
           AND ($10::uuid IS NULL OR tenant_id = $10) \
         ORDER BY occurred_at DESC, id DESC \
         LIMIT $11",
    )
    .bind(filter.from)
    .bind(filter.to)
    .bind(action)
    .bind(actor_type)
    .bind(actor_id)
    .bind(filter.object_type.as_deref())
    .bind(filter.object_id)
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(filter.tenant_id)
    .bind(limit + 1)
    .fetch_all(conn)
    .await
    .map_err(DbError::Query)?
    .into_iter()
    .map(AuditRecord::try_from)
    .collect::<Result<_, _>>()?;
    let has_more = records.len() as i64 > limit;
    records.truncate(limit as usize);
    Ok(AuditPageResult { records, has_more })
}

type Row = (
    Uuid,
    Uuid,
    OffsetDateTime,
    String,
    Option<Uuid>,
    Option<IpAddr>,
    String,
    String,
    Option<Uuid>,
    Option<Value>,
);

impl TryFrom<Row> for AuditRecord {
    type Error = DbError;

    fn try_from(row: Row) -> Result<Self, DbError> {
        let (
            id,
            tenant_id,
            occurred_at,
            actor_type,
            actor_id,
            source_ip,
            action,
            object_type,
            object_id,
            diff,
        ) = row;
        Ok(Self {
            id: AuditLogId::from_uuid(id),
            tenant_id: TenantId::from_uuid(tenant_id),
            occurred_at,
            actor: Actor::from_parts(actor_type, actor_id),
            source_ip,
            action: AuditAction::from_db(action),
            object_type,
            object_id,
            diff,
        })
    }
}

/// True for a year in the range 1970 to 9999.
pub fn is_supported_time(value: OffsetDateTime) -> bool {
    (1970..=MAX_YEAR).contains(&value.year())
}

#[cfg(test)]
mod diff_tests {
    use serde_json::json;

    use super::{REDACTED, diff};

    #[test]
    fn ac_au_01_diff_redacts_secret_fields() {
        let before = json!({
            "name": "old",
            "password": "canary-old",
            "auth": {"secret": "kind", "value": "canary-inner"},
            "plain": {"value": "plain-old"},
            "note": "same"
        });
        let after = json!({
            "name": "new",
            "password": "canary-new",
            "auth": {"secret": "kind2", "value": "canary-inner-2"},
            "plain": {"value": "plain-new"},
            "note": "same"
        });
        let result = diff(Some(&before), Some(&after), &[]);
        let value = result.as_value();
        let text = value.to_string();
        assert!(!text.contains("canary"), "{text}");
        assert_eq!(value["name"]["old"], json!("old"));
        assert_eq!(value["name"]["new"], json!("new"));
        assert_eq!(value["password"]["old"], json!(REDACTED));
        assert_eq!(value["password"]["new"], json!(REDACTED));
        assert_eq!(value["auth"]["value"]["old"], json!(REDACTED));
        assert_eq!(value["auth"]["value"]["new"], json!(REDACTED));
        assert_eq!(value["plain"]["value"]["old"], json!("plain-old"));
        assert_eq!(value["plain"]["value"]["new"], json!("plain-new"));
        assert!(value.get("note").is_none());
    }

    #[test]
    fn ac_au_01_diff_redacts_a_created_object() {
        // Create: None to an object with a new key that holds a secret. The whole object is the
        // `new` value, so the redaction applies inside it.
        let after = json!({"credentials": {"password": "canary", "kind": "x"}});
        let value = diff(None, Some(&after), &[]);
        let text = value.as_value().to_string();
        assert!(!text.contains("canary"), "{text}");
        assert_eq!(
            value.as_value()["new"]["credentials"]["password"],
            json!(REDACTED)
        );
    }

    #[test]
    fn ac_au_01_diff_redacts_a_deleted_object() {
        let before = json!({"token": "canary"});
        let value = diff(Some(&before), None, &[]);
        let text = value.as_value().to_string();
        assert!(!text.contains("canary"), "{text}");
        assert_eq!(value.as_value()["old"]["token"], json!(REDACTED));
    }

    #[test]
    fn ac_au_01_diff_redacts_an_array_of_objects() {
        let before = json!({"items": [{"name": "a", "password": "canary-old"}]});
        let after = json!({"items": [{"name": "b", "password": "canary-new"}]});
        let value = diff(Some(&before), Some(&after), &[]);
        let text = value.as_value().to_string();
        assert!(!text.contains("canary"), "{text}");
        assert_eq!(
            value.as_value()["items"]["new"][0]["password"],
            json!(REDACTED)
        );
    }

    #[test]
    fn ac_au_01_diff_redacts_on_a_type_change() {
        let before = json!({"data": {"secret": "canary"}});
        let after = json!({"data": "plain"});
        let value = diff(Some(&before), Some(&after), &[]);
        let text = value.as_value().to_string();
        assert!(!text.contains("canary"), "{text}");
        assert_eq!(value.as_value()["data"]["old"]["secret"], json!(REDACTED));
    }

    #[test]
    fn ac_au_01_diff_redacts_deep_nesting() {
        let before = json!({"a": {"b": {"c": {"private_key": "canary-old"}}}});
        let after = json!({"a": {"b": {"c": {"private_key": "canary-new"}}}});
        let value = diff(Some(&before), Some(&after), &[]);
        let text = value.as_value().to_string();
        assert!(!text.contains("canary"), "{text}");
    }

    #[test]
    fn the_redact_list_marks_a_field() {
        let before = json!({"token_name": "one"});
        let after = json!({"token_name": "two"});
        let value = diff(Some(&before), Some(&after), &["token_name"]);
        assert_eq!(value.as_value()["token_name"]["new"], json!(REDACTED));
    }

    #[test]
    fn a_redacted_field_never_writes_the_value() {
        let before = json!({});
        let after = json!({"api_key": "canary"});
        let value = diff(Some(&before), Some(&after), &[]);
        assert!(!value.as_value().to_string().contains("canary"));
        assert_eq!(value.as_value()["api_key"]["new"], json!(REDACTED));
    }
}
