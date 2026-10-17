//! The records that block the delete of another record.
//!
//! A delete check returns a bounded list, so a tenant with many references never builds a large
//! response.

use uuid::Uuid;

use crate::DbError;
use crate::tx::TenantTx;

pub const REFERRER_LIMIT: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    pub id: Uuid,
    pub name: String,
}

/// The first `REFERRER_LIMIT` records, and the count of all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Referrers {
    pub items: Vec<Referrer>,
    pub total: i64,
}

impl Referrers {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// Runs a query that selects `(id, name, total)` with one `$1` parameter and `LIMIT $2`.
pub(crate) async fn fetch(
    tx: &mut TenantTx<'_>,
    sql: &'static str,
    subject: Uuid,
) -> Result<Referrers, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(sql)
        .bind(subject)
        .bind(REFERRER_LIMIT)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    let total = rows.first().map_or(0, |row| row.2);
    Ok(Referrers {
        items: rows
            .into_iter()
            .map(|(id, name, _)| Referrer { id, name })
            .collect(),
        total,
    })
}
