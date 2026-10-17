//! The control plane start checks (ADR-0023, section Secrets; AC-RP-02).
//!
//! At start the control plane loads the key encryption key (KEK) and verifies the `kek_check` row
//! of the platform tenant. On the first start it creates the row. On every later start it verifies
//! the row, so the control plane refuses to start with an incorrect KEK and shows the cause
//! (AC-RP-02).

use restichub_db::{Database, PLATFORM_TENANT_ID, TenantStatus, kek, tenants};
use restichub_secrets::{
    CryptoError, KekCheck, KekError, KekId, KekProvider, make_kek_check, verify_kek_check,
};

use crate::config::{TenancyConfig, TenancyMode};

const KEK_CHECK_LOCK: &str = "restichub.kek_check:";

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    Database(#[from] restichub_db::DbError),
    #[error(transparent)]
    Kek(#[from] KekError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the stored kek_check row is not valid")]
    BadCheck,
    #[error("the single-tenant mode cannot start: {reason}")]
    Tenancy { reason: String },
}

/// Checks that the database matches the tenancy mode (ADR-0024).
///
/// Single mode needs exactly one active tenant, named `tenancy.default_tenant`. The function
/// refuses the start otherwise and names the problem. An installation with no non-platform tenant
/// is a fresh install, so the setup creates the tenant `default` later. A suspended or a deleted
/// tenant does not block the switch, because only an active tenant serves requests.
pub async fn ensure_tenancy_mode(
    db: &Database,
    tenancy: &TenancyConfig,
) -> Result<(), StartupError> {
    if tenancy.mode == TenancyMode::Multi {
        return Ok(());
    }
    let mut tx = db.begin_platform().await?;
    let all = tenants::list(&mut tx).await?;
    tx.commit().await?;
    let non_platform: Vec<&tenants::Tenant> = all
        .iter()
        .filter(|tenant| tenant.id != PLATFORM_TENANT_ID)
        .collect();
    if non_platform.is_empty() {
        return Ok(());
    }
    let active: Vec<&&tenants::Tenant> = non_platform
        .iter()
        .filter(|tenant| tenant.status == TenantStatus::Active)
        .collect();
    if active.is_empty() {
        let named = non_platform
            .iter()
            .find(|tenant| tenant.name.eq_ignore_ascii_case(&tenancy.default_tenant));
        let reason = match named {
            Some(tenant) => format!(
                "the tenant `{}` is `{}`, not active; single mode needs it active",
                tenant.name,
                tenant.status.as_str()
            ),
            None => format!(
                "the database holds no active tenant named `{}`; create it or set the setting `tenancy.default_tenant`",
                tenancy.default_tenant
            ),
        };
        return Err(StartupError::Tenancy { reason });
    }
    if active.len() > 1 {
        return Err(StartupError::Tenancy {
            reason: format!(
                "it needs exactly one active tenant, but the database holds {} active tenants of {} non-platform tenants; delete or suspend the other tenants first",
                active.len(),
                non_platform.len()
            ),
        });
    }
    let only = active[0];
    if !only.name.eq_ignore_ascii_case(&tenancy.default_tenant) {
        return Err(StartupError::Tenancy {
            reason: format!(
                "the only active tenant is named `{}`, but the setting `tenancy.default_tenant` is `{}`; rename the tenant or set the setting",
                only.name, tenancy.default_tenant
            ),
        });
    }
    Ok(())
}

/// Verifies the `kek_check` row of the platform tenant, or creates it on the first start.
///
/// An advisory lock is taken before the read, so two starts cannot create two rows. The start is
/// refused when the loaded KEK does not match the stored check; the error is [`KekError::Mismatch`]
/// with the two key IDs (AC-RP-02).
pub async fn ensure_kek_check(
    db: &Database,
    provider: &dyn KekProvider,
) -> Result<(), StartupError> {
    let key = provider.current();
    let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
    // The lock serializes two starts. The read below happens after the lock, so the second start
    // sees the row that the first start created.
    tx.lock(KEK_CHECK_LOCK).await?;
    match kek::read_check(&mut tx).await? {
        Some(row) => {
            let check = KekCheck {
                kek_id: KekId::parse(&row.kek_id)?,
                nonce: row
                    .nonce
                    .as_slice()
                    .try_into()
                    .map_err(|_| StartupError::BadCheck)?,
                ciphertext: row.ciphertext,
            };
            verify_kek_check(key, &check)?;
        }
        None => {
            let check = make_kek_check(key)?;
            kek::insert_check(
                &mut tx,
                check.kek_id.as_str(),
                &check.nonce,
                &check.ciphertext,
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Bounds one warning line; the job log names every plan through its failed jobs.
const REPORTED_PLANS: usize = 20;

/// Logs each local repository without a host and each plan of one that runs on another host.
///
/// The check changes no row: the data stays, the plan save refuses the old binding, and the job
/// builder refuses a job on another host with `not_repository_host`.
pub async fn report_local_repository_hosts(db: &Database) -> Result<(), restichub_db::DbError> {
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        let mut tx = db.begin_tenant(tenant.id).await?;
        let problems = restichub_db::plans::local_host_problems(&mut tx).await?;
        tx.commit().await?;
        for problem in problems {
            let plan_count = problem.plans_off_host.len();
            let plans: Vec<String> = problem
                .plans_off_host
                .iter()
                .take(REPORTED_PLANS)
                .map(ToString::to_string)
                .collect();
            match problem.host {
                None => tracing::warn!(
                    tenant = %tenant.id,
                    repository = %problem.repository_id,
                    plan_count,
                    plans = ?plans,
                    "a local repository has no host; set its host, then move its plans to that host"
                ),
                Some(host) => tracing::warn!(
                    tenant = %tenant.id,
                    repository = %problem.repository_id,
                    host = %host,
                    plan_count,
                    plans = ?plans,
                    "plans of a local repository run on another host; their jobs fail with not_repository_host"
                ),
            }
        }
    }
    Ok(())
}

/// Logs each repository that shares its location with another repository of its tenant (FR-RP-11).
///
/// The check changes no row. A duplicate from before the rule stays until the owner deletes one.
pub async fn report_shared_locations(db: &Database) -> Result<(), restichub_db::DbError> {
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        let mut tx = db.begin_tenant(tenant.id).await?;
        let repositories = restichub_db::repositories::list(&mut tx).await?;
        tx.commit().await?;
        let sharing = crate::repositories::location::sharing_names(&repositories);
        for (repository, others) in repositories.iter().zip(sharing) {
            if !others.is_empty() {
                tracing::warn!(
                    tenant = %tenant.id,
                    repository = %repository.id,
                    shared_with = ?others,
                    "a repository shares its location with another repository; delete one of them"
                );
            }
        }
    }
    Ok(())
}
