pub mod access;
pub mod cleanup;
pub mod expiration;
pub mod language;
pub mod password;
pub mod paste;
pub mod pdf;
pub mod render;

use crate::error::AppError;

static CPU_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub async fn compute<T, F>(operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    let permit = CPU_WORKERS
        .acquire()
        .await
        .map_err(|_| AppError::Internal("render workers unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await?
}
