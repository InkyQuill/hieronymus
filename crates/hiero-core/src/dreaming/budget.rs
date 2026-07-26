use std::collections::BTreeSet;

use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone)]
pub(crate) struct AffectedCrystalIds {
    limit: usize,
    ids: BTreeSet<i64>,
}

impl AffectedCrystalIds {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            ids: BTreeSet::new(),
        }
    }

    pub(crate) fn with_ids(limit: usize, ids: impl IntoIterator<Item = i64>) -> Self {
        Self {
            limit,
            ids: ids.into_iter().collect(),
        }
    }

    pub(crate) fn can_reserve(&self, ids: &[i64]) -> bool {
        self.ids.len().saturating_add(self.additional(ids)) <= self.limit
    }

    pub(crate) fn can_reserve_with_new(&self, ids: &[i64], new_count: usize) -> bool {
        self.ids
            .len()
            .saturating_add(self.additional(ids))
            .saturating_add(new_count)
            <= self.limit
    }

    pub(crate) fn reserve(&mut self, ids: &[i64]) -> bool {
        if !self.can_reserve(ids) {
            return false;
        }
        self.ids.extend(ids.iter().copied());
        true
    }

    pub(crate) fn include_existing(&mut self, ids: impl IntoIterator<Item = i64>) {
        self.ids.extend(ids);
    }

    pub(crate) const fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.ids.iter().copied()
    }

    fn additional(&self, ids: &[i64]) -> usize {
        ids.iter()
            .copied()
            .filter(|id| !self.ids.contains(id))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

pub(crate) async fn load_affected(
    pool: &SqlitePool,
    maintenance_cycle_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT crystal_id FROM dream_affected_crystals
         WHERE maintenance_cycle_id=?
         ORDER BY crystal_id",
    )
    .bind(maintenance_cycle_id)
    .fetch_all(pool)
    .await
}

pub(crate) async fn merge_affected(
    connection: &mut SqliteConnection,
    maintenance_cycle_id: i64,
    budgets: &mut [AffectedCrystalIds],
) -> Result<(), sqlx::Error> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT crystal_id FROM dream_affected_crystals
         WHERE maintenance_cycle_id=?
         ORDER BY crystal_id",
    )
    .bind(maintenance_cycle_id)
    .fetch_all(&mut *connection)
    .await?;
    for budget in budgets {
        budget.include_existing(ids.iter().copied());
    }
    Ok(())
}

pub(crate) async fn record_affected(
    connection: &mut SqliteConnection,
    maintenance_cycle_id: i64,
    ids: &[i64],
) -> Result<(), sqlx::Error> {
    for id in ids.iter().copied().collect::<BTreeSet<_>>() {
        sqlx::query(
            "INSERT OR IGNORE INTO dream_affected_crystals(
                maintenance_cycle_id,crystal_id,created_at
             ) VALUES (?,?,CURRENT_TIMESTAMP)",
        )
        .bind(maintenance_cycle_id)
        .bind(id)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}
