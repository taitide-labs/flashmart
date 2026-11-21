use sqlx::{Postgres, Transaction};

/// 显式回滚事务并在回滚失败时记录结构化告警，避免以 `let _ = tx.rollback().await` 静默吞错
pub async fn rollback_tx(tx: Transaction<'_, Postgres>, slice: &'static str) {
    if let Err(err) = tx.rollback().await {
        tracing::warn!(slice, error = %err, "database transaction rollback failed");
    }
}
