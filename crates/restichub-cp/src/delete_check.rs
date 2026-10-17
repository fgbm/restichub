//! The records that block the delete of a repository, a profile or a secret.
//!
//! The delete handlers and the usage routes share these checks, so the answer before the
//! confirmation and the refusal of the delete never differ.

use restichub_db::TenantTx;
use restichub_db::ids::{ProfileId, RepositoryId, SecretId};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::{ApiError, BlockerKind, Blockers, ErrorCode};

/// The answer of a usage route: what blocks the delete now.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsageView {
    /// Null when the delete is allowed. Else the code that the delete returns.
    pub blocked_by: Option<ErrorCode>,
    pub blockers: Blockers,
}

pub struct Block {
    pub code: ErrorCode,
    pub blockers: Blockers,
}

impl Block {
    pub fn into_error(self) -> ApiError {
        ApiError::blocked(self.code, self.blockers)
    }
}

impl From<Option<Block>> for UsageView {
    fn from(block: Option<Block>) -> Self {
        match block {
            Some(block) => Self {
                blocked_by: Some(block.code),
                blockers: block.blockers,
            },
            None => Self {
                blocked_by: None,
                blockers: Blockers {
                    items: Vec::new(),
                    total: 0,
                },
            },
        }
    }
}

/// A profile binds the repository, or an active job holds it. The profile check comes first.
pub async fn repository_block(
    tx: &mut TenantTx<'_>,
    id: RepositoryId,
) -> Result<Option<Block>, ApiError> {
    let profiles = restichub_db::profiles::referrers_of_repository(tx, id).await?;
    if !profiles.is_empty() {
        return Ok(Some(Block {
            code: ErrorCode::RepositoryInUse,
            blockers: Blockers::new(BlockerKind::Profile, &profiles),
        }));
    }
    let jobs = restichub_db::jobs::active_for_repository(tx, id).await?;
    if !jobs.is_empty() {
        return Ok(Some(Block {
            code: ErrorCode::RepositoryHasActiveJob,
            blockers: Blockers::new(BlockerKind::Job, &jobs),
        }));
    }
    Ok(None)
}

/// Plans hold a foreign key to it (FR-SC-11).
pub async fn profile_block(
    tx: &mut TenantTx<'_>,
    id: ProfileId,
) -> Result<Option<Block>, ApiError> {
    let plans = restichub_db::plans::referrers_of_profile(tx, id).await?;
    if plans.is_empty() {
        return Ok(None);
    }
    Ok(Some(Block {
        code: ErrorCode::ProfileInUse,
        blockers: Blockers::new(BlockerKind::Plan, &plans),
    }))
}

/// A repository, a channel, a profile or a job template names the secret.
pub async fn secret_block(tx: &mut TenantTx<'_>, id: SecretId) -> Result<Option<Block>, ApiError> {
    let usage = restichub_db::secrets::list_usage(tx, Some(id)).await?;
    if usage.is_empty() {
        return Ok(None);
    }
    let items = usage
        .iter()
        .take(restichub_db::referrers::REFERRER_LIMIT as usize)
        .map(|item| crate::error::Blocker {
            kind: BlockerKind::from(item.kind),
            id: item.object_id.to_string(),
            name: item.object_name.clone(),
        })
        .collect();
    Ok(Some(Block {
        code: ErrorCode::SecretInUse,
        blockers: Blockers {
            items,
            total: usage.len() as i64,
        },
    }))
}
