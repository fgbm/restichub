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

/// The default number of records in one page.
pub const DEFAULT_PAGE_LIMIT: i64 = 50;

/// The largest number of records in one page.
pub const MAX_PAGE_LIMIT: i64 = 200;

/// The marker of a redacted field.
pub const REDACTED: &str = "<redacted>";

/// The last year of a valid RFC 3339 time.
const MAX_YEAR: i32 = 9999;

/// The key names that are always sensitive.
///
/// A field whose name holds one of these words is sensitive.
const SENSITIVE_KEYS: [&str; 5] = ["password", "secret", "token", "key", "private_key"];

/// The actor of an audit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// A local user.
    User(UserId),
    /// An API token.
    ApiToken(ApiTokenId),
    /// An agent.
    Agent(Uuid),
    /// The control plane.
    System,
    /// An actor type that this version does not know. The list keeps the row.
    Unknown {
        /// The stored actor type.
        actor_type: String,
        /// The stored actor id.
        actor_id: Option<Uuid>,
    },
}

impl Actor {
    /// Returns the stored value of the actor type.
    pub fn actor_type(&self) -> &str {
        match self {
            Self::User(_) => ActorType::User.as_str(),
            Self::ApiToken(_) => ActorType::ApiToken.as_str(),
            Self::Agent(_) => ActorType::Agent.as_str(),
            Self::System => ActorType::System.as_str(),
            Self::Unknown { actor_type, .. } => actor_type,
        }
    }

    /// Returns the stored value of the actor id.
    pub fn actor_id(&self) -> Option<Uuid> {
        match self {
            Self::User(id) => Some(id.as_uuid()),
            Self::ApiToken(id) => Some(id.as_uuid()),
            Self::Agent(id) => Some(*id),
            Self::System => None,
            Self::Unknown { actor_id, .. } => *actor_id,
        }
    }

    /// Builds the actor from the two stored columns.
    ///
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

/// The action of an audit record.
///
/// The string of each action is stable. Do not change a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditAction {
    /// A user signs in.
    SignIn,
    /// A sign-in attempt fails.
    SignInFailed,
    /// A user signs out.
    SignOut,
    /// The control plane creates a configuration.
    ConfigCreate,
    /// The control plane changes a configuration.
    ConfigUpdate,
    /// The control plane removes a configuration.
    ConfigDelete,
    /// A job starts.
    JobRun,
    /// A job is cancelled.
    JobCancel,
    /// A restore starts.
    RestoreStart,
    /// The control plane delivers a secret to an agent.
    SecretDelivered,
    /// An agent enrolls.
    AgentEnrolled,
    /// The control plane revokes an agent.
    AgentRevoked,
    /// The control plane installs an agent remotely.
    RemoteInstall,
    /// The control plane creates a tenant.
    TenantCreate,
    /// The control plane suspends a tenant.
    TenantSuspend,
    /// The control plane resumes a tenant.
    TenantResume,
    /// The control plane removes a tenant.
    TenantDelete,
    /// The control plane creates a user.
    UserCreate,
    /// The control plane changes a user.
    UserUpdate,
    /// The control plane removes a user.
    UserDelete,
    /// The control plane creates an API token.
    ApiTokenCreate,
    /// The control plane revokes an API token.
    ApiTokenRevoke,
    /// The control plane creates a secret.
    SecretCreate,
    /// The control plane replaces a secret.
    SecretReplace,
    /// The control plane removes a secret.
    SecretDelete,
    /// An action that this version does not know. The list keeps the row.
    Unknown(String),
}

impl AuditAction {
    /// Returns the stable stored string of the action.
    pub fn as_str(&self) -> &str {
        match self {
            Self::SignIn => "sign_in",
            Self::SignInFailed => "sign_in_failed",
            Self::SignOut => "sign_out",
            Self::ConfigCreate => "config_create",
            Self::ConfigUpdate => "config_update",
            Self::ConfigDelete => "config_delete",
            Self::JobRun => "job_run",
            Self::JobCancel => "job_cancel",
            Self::RestoreStart => "restore_start",
            Self::SecretDelivered => "secret_delivered",
            Self::AgentEnrolled => "agent_enrolled",
            Self::AgentRevoked => "agent_revoked",
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
            Self::SecretCreate => "secret_create",
            Self::SecretReplace => "secret_replace",
            Self::SecretDelete => "secret_delete",
            Self::Unknown(value) => value,
        }
    }

    /// Builds the action from the stored string. The function returns `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        let action = match value {
            "sign_in" => Self::SignIn,
            "sign_in_failed" => Self::SignInFailed,
            "sign_out" => Self::SignOut,
            "config_create" => Self::ConfigCreate,
            "config_update" => Self::ConfigUpdate,
            "config_delete" => Self::ConfigDelete,
            "job_run" => Self::JobRun,
            "job_cancel" => Self::JobCancel,
            "restore_start" => Self::RestoreStart,
            "secret_delivered" => Self::SecretDelivered,
            "agent_enrolled" => Self::AgentEnrolled,
            "agent_revoked" => Self::AgentRevoked,
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
            "secret_create" => Self::SecretCreate,
            "secret_replace" => Self::SecretReplace,
            "secret_delete" => Self::SecretDelete,
            _ => return None,
        };
        Some(action)
    }

    /// Builds the action from a stored string. The function keeps an unknown string.
    fn from_db(value: String) -> Self {
        Self::parse(&value).unwrap_or(Self::Unknown(value))
    }

    /// Returns the known actions, in the stored order.
    ///
    /// The list excludes [`AuditAction::Unknown`]. The OpenAPI schema and the audit query use it.
    pub fn known() -> &'static [Self] {
        &[
            Self::SignIn,
            Self::SignInFailed,
            Self::SignOut,
            Self::ConfigCreate,
            Self::ConfigUpdate,
            Self::ConfigDelete,
            Self::JobRun,
            Self::JobCancel,
            Self::RestoreStart,
            Self::SecretDelivered,
            Self::AgentEnrolled,
            Self::AgentRevoked,
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
            Self::SecretCreate,
            Self::SecretReplace,
            Self::SecretDelete,
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

/// A diff that holds no secret value.
///
/// The type keeps the JSON private. Only [`diff`] and [`RedactedDiff::empty`] build a value. A
/// caller can therefore not pass raw secret data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedDiff(Value);

impl RedactedDiff {
    /// Returns a diff with no changed field. It serialises as `{}`.
    pub fn empty() -> Self {
        Self(Value::Object(Map::new()))
    }

    /// Returns the JSON of the diff.
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

/// Builds the diff of two values.
///
/// The function compares the top-level and the nested fields. A changed field becomes
/// `{"old": .., "new": ..}`. A missing side is `null`.
///
/// The function redacts a field when its name is in `redact` (without the letter case) or when
/// its name holds one of `password`, `secret`, `token`, `key`, `private_key`. It also redacts a
/// field named `value` when the parent object holds a field named `secret`.
///
/// Every emitted value goes through [`redact_value`]. The function therefore redacts a sensitive
/// field at any depth, also when one side is not an object, when a key is on one side only, and
/// inside an array. A redacted field becomes `"<redacted>"`. The function never writes a secret
/// value.
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

/// Returns true if the field is sensitive.
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

/// Returns the redacted marker of a changed field.
fn redacted() -> Value {
    let mut object = Map::new();
    object.insert("old".to_owned(), Value::String(REDACTED.to_owned()));
    object.insert("new".to_owned(), Value::String(REDACTED.to_owned()));
    Value::Object(object)
}

/// Returns a changed field with both sides redacted.
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

/// The input of an insert.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// The actor of the operation.
    pub actor: Actor,
    /// The source IP address of the request. It is optional.
    pub source_ip: Option<IpAddr>,
    /// The action.
    pub action: AuditAction,
    /// The type of the object of the operation.
    pub object_type: String,
    /// The identifier of the object of the operation. It is optional.
    pub object_id: Option<Uuid>,
    /// The diff of the operation. It is optional. It holds no secret.
    pub diff: Option<RedactedDiff>,
}

impl AuditEvent {
    /// Creates an event with a system actor and no diff.
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

/// A stored audit record.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    /// The identifier of the record.
    pub id: AuditLogId,
    /// The tenant of the record.
    pub tenant_id: TenantId,
    /// The time of the record.
    pub occurred_at: OffsetDateTime,
    /// The actor of the operation.
    pub actor: Actor,
    /// The source IP address of the request.
    pub source_ip: Option<IpAddr>,
    /// The action.
    pub action: AuditAction,
    /// The type of the object of the operation.
    pub object_type: String,
    /// The identifier of the object of the operation.
    pub object_id: Option<Uuid>,
    /// The diff of the operation as JSON.
    pub diff: Option<Value>,
}

/// The filter of a list.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// The tenant of the record. The platform list filters by it. A tenant list ignores it, because
    /// Row-Level Security shows its own rows only.
    pub tenant_id: Option<TenantId>,
    /// The start of the time range. It includes the value.
    pub from: Option<OffsetDateTime>,
    /// The end of the time range. It includes the value.
    pub to: Option<OffsetDateTime>,
    /// The action.
    pub action: Option<AuditAction>,
    /// The actor.
    pub actor: Option<Actor>,
    /// The type of the object.
    pub object_type: Option<String>,
    /// The identifier of the object.
    pub object_id: Option<Uuid>,
}

/// The key of the last record of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor {
    /// The time of the record.
    pub occurred_at: OffsetDateTime,
    /// The identifier of the record.
    pub id: AuditLogId,
}

/// The page of a list.
#[derive(Debug, Clone, Copy)]
pub struct AuditPage {
    /// The number of records. The list limits the value to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    /// The last record of the previous page. The list returns the older records.
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

/// One page of audit records.
#[derive(Debug, Clone)]
pub struct AuditPageResult {
    /// The records of the page. The list holds at most `limit` records.
    pub records: Vec<AuditRecord>,
    /// True if the table holds more records after this page.
    pub has_more: bool,
}

/// Inserts a record in the tenant of the transaction.
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

/// Inserts a record in a platform transaction.
///
/// The policy `audit_log_platform_insert` of migration 0014 accepts the entered tenant, the
/// platform tenant and the tenant of the GUC `restichub.audit_tenant_id`. The caller passes the
/// tenant. The caller enters a created tenant first with `PlatformTx::enter_tenant`, and it uses
/// [`insert_platform_for`] for an existing tenant.
pub async fn insert_platform(
    tx: &mut PlatformTx<'_>,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<AuditLogId, DbError> {
    insert_row(tx.as_conn(), tenant, event).await
}

/// Inserts a record for an existing tenant in a platform transaction (issue #20).
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

/// Sets or clears the GUC `restichub.audit_tenant_id` of the transaction.
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

/// Inserts a record with a connection.
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

/// Lists the records of the tenant of the transaction.
pub async fn list(
    tx: &mut TenantTx<'_>,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    list_rows(tx.as_conn(), filter, page).await
}

/// Lists the records of every tenant in a platform transaction.
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

/// The column tuple of one audit row.
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

/// Returns true if the year of the time is in the range 1970 to 9999.
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
        // Delete: an object with a secret to None.
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
        // Object to scalar.
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
