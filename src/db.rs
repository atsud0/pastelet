use std::{str::FromStr, time::Duration};

use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use thiserror::Error;

use crate::{
    config::Config,
    models::{Paste, PasteMetadata},
};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("cannot prepare database directory")]
    Directory(#[from] std::io::Error),
    #[error("cannot connect to SQLite")]
    Connection(#[from] sqlx::Error),
    #[error("cannot migrate SQLite")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

pub async fn connect(config: &Config) -> Result<SqlitePool, DatabaseError> {
    let options = SqliteConnectOptions::from_str(&config.database_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .pragma("secure_delete", "ON")
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    if let Some(parent) = options
        .get_filename()
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(config.database_max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;
    if let Err(error) = MIGRATOR.run(&pool).await {
        pool.close().await;
        return Err(error.into());
    }
    Ok(pool)
}

pub async fn insert_paste(pool: &SqlitePool, paste: &Paste) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO pastes (id, title, content, language, password_hash, burn_after_read, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(&paste.id)
    .bind(&paste.title)
    .bind(&paste.content)
    .bind(&paste.language)
    .bind(&paste.password_hash)
    .bind(paste.burn_after_read)
    .bind(paste.created_at)
    .bind(paste.expires_at)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn find_live_paste(
    pool: &SqlitePool,
    id: &str,
    now: i64,
) -> Result<Option<Paste>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, title, content, language, password_hash, burn_after_read, created_at, expires_at
         FROM pastes WHERE id = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(id)
    .bind(now)
    .fetch_optional(pool)
    .await
}

pub async fn find_live_metadata(
    pool: &SqlitePool,
    id: &str,
    now: i64,
) -> Result<Option<PasteMetadata>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, password_hash, burn_after_read, expires_at
         FROM pastes WHERE id = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(id)
    .bind(now)
    .fetch_optional(pool)
    .await
}

pub async fn insert_token(
    pool: &SqlitePool,
    id: &str,
    hash: &[u8],
    kind: &str,
    expires_at: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "INSERT INTO paste_tokens (token_hash, paste_id, kind, expires_at)
        SELECT ?, id, ?, ? FROM pastes WHERE id = ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(hash)
    .bind(kind)
    .bind(expires_at)
    .bind(id)
    .bind(now)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

pub async fn valid_token(
    pool: &SqlitePool,
    id: &str,
    hash: &[u8],
    kind: &str,
    now: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM paste_tokens WHERE paste_id = ? AND token_hash = ? AND kind = ? AND expires_at > ?)")
        .bind(id).bind(hash).bind(kind).bind(now).fetch_one(pool).await
}

pub async fn consume_confirmed(
    pool: &SqlitePool,
    id: &str,
    token_hash: &[u8],
    now: i64,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM pastes WHERE id = ? AND burn_after_read = 1
        AND (expires_at IS NULL OR expires_at > ?)
        AND EXISTS(SELECT 1 FROM paste_tokens WHERE paste_id = pastes.id AND token_hash = ? AND kind = 'confirm' AND expires_at > ?)")
        .bind(id).bind(now).bind(token_hash).bind(now).execute(pool).await?.rows_affected() == 1)
}

pub async fn cleanup_expired(pool: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let deleted =
        sqlx::query("DELETE FROM pastes WHERE expires_at IS NOT NULL AND expires_at <= ?")
            .bind(now)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
    sqlx::query("DELETE FROM paste_tokens WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(deleted)
}

pub async fn consume_api(pool: &SqlitePool, paste: &Paste, now: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM pastes WHERE id = ? AND burn_after_read = 1
        AND created_at = ? AND password_hash IS ? AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(&paste.id)
    .bind(paste.created_at)
    .bind(&paste.password_hash)
    .bind(now)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}
