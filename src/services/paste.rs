use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sqlx::SqlitePool;

use super::{access, compute, expiration::Expiration, language, password};
use crate::{AppState, db, error::AppError, models::Paste};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ID_ATTEMPTS: usize = 5;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePaste {
    pub content: String,
    pub title: Option<String>,
    pub language: Option<String>,
    pub expires_in: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub burn_after_read: bool,
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn random_id() -> Result<String, AppError> {
    let mut result = String::with_capacity(8);
    while result.len() < 8 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| AppError::Internal("OS randomness unavailable"))?;
        for byte in random {
            if byte < 248 {
                result.push(char::from(ALPHABET[usize::from(byte % 62)]));
                if result.len() == 8 {
                    break;
                }
            }
        }
    }
    Ok(result)
}

pub fn unix_now() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Internal("system clock predates Unix epoch"))?
        .as_secs()
        .try_into()
        .map_err(|_| AppError::Internal("timestamp overflow"))
}

pub fn validate(input: &CreatePaste, max_content_bytes: usize) -> Result<Expiration, AppError> {
    if input
        .password
        .as_ref()
        .is_some_and(|value| value.len() > password::MAX_PASSWORD_BYTES)
    {
        return Err(AppError::BadRequest("Password is too long"));
    }
    if input.content.len() > max_content_bytes {
        return Err(AppError::PayloadTooLarge);
    }
    if input.content.is_empty() {
        return Err(AppError::BadRequest("Content is required"));
    }
    if input
        .title
        .as_ref()
        .is_some_and(|title| title.trim().chars().count() > 200)
    {
        return Err(AppError::BadRequest("Title is too long"));
    }
    input
        .expires_in
        .as_deref()
        .map(str::parse)
        .transpose()
        .map(|expiration| expiration.unwrap_or_default())
}

pub async fn create(state: &AppState, input: CreatePaste) -> Result<Paste, AppError> {
    let expiration = validate(&input, state.config.max_content_bytes)?;
    let (input, language) = compute(move || {
        let language = language::resolve(input.language.as_deref(), &input.content)?;
        Ok((input, language))
    })
    .await?;
    let password_hash = match input.password.filter(|value| !value.is_empty()) {
        Some(value) => Some(password::hash(value).await?),
        None => None,
    };
    let created_at = unix_now()?;
    let paste = Paste {
        id: String::new(),
        title: input
            .title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty()),
        content: input.content,
        language,
        password_hash,
        burn_after_read: input.burn_after_read,
        created_at,
        expires_at: expiration.expires_at(created_at)?,
    };
    persist_with_ids(&state.pool, paste, random_id).await
}

async fn persist_with_ids(
    pool: &SqlitePool,
    mut paste: Paste,
    mut generate: impl FnMut() -> Result<String, AppError>,
) -> Result<Paste, AppError> {
    for _ in 0..ID_ATTEMPTS {
        paste.id = generate()?;
        if db::insert_paste(pool, &paste).await? {
            return Ok(paste);
        }
    }
    Err(AppError::Internal("random ID collisions exhausted retries"))
}

pub async fn metadata(
    state: &AppState,
    id: &str,
) -> Result<crate::models::PasteMetadata, AppError> {
    if !valid_id(id) {
        return Err(AppError::NotFound);
    }
    let now = unix_now()?;
    if let Some(metadata) = db::find_live_metadata(&state.pool, id, now).await? {
        return Ok(metadata);
    }
    sqlx::query("DELETE FROM pastes WHERE id = ? AND expires_at IS NOT NULL AND expires_at <= ?")
        .bind(id)
        .bind(now)
        .execute(&state.pool)
        .await?;
    Err(AppError::NotFound)
}

pub async fn authorized_read(
    state: &AppState,
    id: &str,
    headers: &axum::http::HeaderMap,
) -> Result<Paste, AppError> {
    let metadata = metadata(state, id).await?;
    if !access::authorized(state, &metadata, headers).await? {
        return Err(AppError::Unauthorized);
    }
    db::find_live_paste(&state.pool, id, unix_now()?)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn consume(state: &AppState, id: &str, token: &str) -> Result<(), AppError> {
    let hash = access::token_hash(token).ok_or(AppError::BadRequest("Invalid confirmation"))?;
    if !db::consume_confirmed(&state.pool, id, &hash, unix_now()?).await? {
        return Err(AppError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(content: &str) -> CreatePaste {
        CreatePaste {
            content: content.into(),
            title: None,
            language: None,
            expires_in: None,
            password: None,
            burn_after_read: false,
        }
    }

    #[test]
    fn content_limits_are_utf8_bytes_and_title_limits_are_characters() {
        assert!(validate(&input("中文"), 6).is_ok());
        assert!(matches!(
            validate(&input("中文"), 5),
            Err(AppError::PayloadTooLarge)
        ));
        assert!(validate(&input(""), 100).is_err());
        assert!(validate(&input(" \n"), 100).is_ok());
        let mut input = input("ok");
        input.title = Some("中".repeat(200));
        assert!(validate(&input, 100).is_ok());
        input.title = Some("中".repeat(201));
        assert!(validate(&input, 100).is_err());
    }

    #[test]
    fn ids_are_eight_ascii_base62_characters() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = random_id().expect("OS random source");
            assert!(valid_id(&id));
            assert!(seen.insert(id));
        }
        for invalid in [
            "",
            "1234567",
            "123456789",
            "abcd-123",
            "../../ab",
            "中文abcd",
        ] {
            assert!(!valid_id(invalid));
        }
    }

    #[tokio::test]
    async fn collisions_retry_without_overwriting_and_exhaustion_fails() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("database");
        db::MIGRATOR.run(&pool).await.expect("migration");
        let make_paste = |content: &str| Paste {
            id: String::new(),
            title: None,
            content: content.into(),
            language: "text".into(),
            password_hash: None,
            burn_after_read: false,
            created_at: 100,
            expires_at: None,
        };
        persist_with_ids(&pool, make_paste("first"), || Ok("Ab12Cd34".into()))
            .await
            .expect("first paste");
        let mut ids = ["Ab12Cd34", "Ef56Gh78"].into_iter();
        let second = persist_with_ids(&pool, make_paste("second"), || {
            Ok(ids.next().expect("retry id").into())
        })
        .await
        .expect("collision retry");
        assert_eq!(second.id, "Ef56Gh78");
        assert!(
            persist_with_ids(&pool, make_paste("overwrite"), || Ok("Ab12Cd34".into()))
                .await
                .is_err()
        );
        assert_eq!(
            db::find_live_paste(&pool, "Ab12Cd34", 100)
                .await
                .expect("query")
                .expect("paste")
                .content,
            "first"
        );
        pool.close().await;
    }
}
