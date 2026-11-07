use sqlx::SqlitePool;
use std::time::Duration;
use tokio::sync::watch;

use super::paste;
use crate::{db, error::AppError};

pub async fn sweep(pool: &SqlitePool) -> Result<u64, AppError> {
    Ok(db::cleanup_expired(pool, paste::unix_now()?).await?)
}

pub async fn run(pool: SqlitePool, interval: Duration, mut stop: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            biased;
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() { break; }
            }
            _ = ticker.tick() => match sweep(&pool).await {
                Ok(deleted) => tracing::debug!(deleted, "expired pastes cleaned"),
                Err(error) => tracing::error!(%error, "expiration cleanup failed"),
            }
        }
    }
}
