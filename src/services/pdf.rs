use crate::{
    AppState,
    error::AppError,
    models::Paste,
    services::{
        password,
        paste::{self, CreatePaste},
    },
};

pub const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;
pub const PDF_LANGUAGE: &str = "pdf";

pub fn validate(data: &[u8]) -> Result<(), AppError> {
    if data.len() > MAX_PDF_BYTES {
        return Err(AppError::PayloadTooLarge);
    }
    if !data.starts_with(b"%PDF-") {
        return Err(AppError::BadRequest("Invalid PDF header"));
    }
    Ok(())
}

pub async fn create(
    state: &AppState,
    mut input: CreatePaste,
    data: Vec<u8>,
) -> Result<Paste, AppError> {
    validate(&data)?;
    input.content = "PDF".into();
    let expiration = paste::validate(&input, usize::MAX)?;
    let password_hash = match input.password.filter(|value| !value.is_empty()) {
        Some(value) => Some(password::hash(value).await?),
        None => None,
    };
    let now = paste::unix_now()?;
    let mut record = Paste {
        id: String::new(),
        title: input
            .title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty()),
        content: "PDF".into(),
        language: PDF_LANGUAGE.into(),
        password_hash,
        burn_after_read: input.burn_after_read,
        created_at: now,
        expires_at: expiration.expires_at(now)?,
    };
    for _ in 0..5 {
        record.id = paste::random_id()?;
        let mut transaction = state.pool.begin().await?;
        let inserted = sqlx::query("INSERT INTO pastes (id,title,content,language,password_hash,burn_after_read,created_at,expires_at) VALUES (?,?,'PDF','pdf',?,?,?,?) ON CONFLICT(id) DO NOTHING")
            .bind(&record.id).bind(&record.title).bind(&record.password_hash).bind(record.burn_after_read).bind(now).bind(record.expires_at)
            .execute(&mut *transaction).await?.rows_affected();
        if inserted == 0 {
            transaction.rollback().await?;
            continue;
        }
        sqlx::query("INSERT INTO pdf_files (paste_id,storage_name,data) VALUES (?,?,?)")
            .bind(&record.id)
            .bind(format!("{}.pdf", record.id))
            .bind(&data)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        return Ok(record);
    }
    Err(AppError::Internal("random ID collisions exhausted retries"))
}

pub async fn data(state: &AppState, id: &str) -> Result<Vec<u8>, AppError> {
    sqlx::query_scalar("SELECT data FROM pdf_files JOIN pastes ON pastes.id = pdf_files.paste_id WHERE pastes.id = ? AND (expires_at IS NULL OR expires_at > ?)")
        .bind(id).bind(paste::unix_now()?).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)
}

pub fn download_name(title: Option<&str>, id: &str) -> String {
    let title = title.unwrap_or(id);
    let cleaned: String = title
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '/' | '\\' | ':' | '"' | '<' | '>' | '|' | '?' | '*'
                )
        })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    let name = if cleaned.is_empty() { id } else { cleaned };
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.to_owned()
    } else {
        format!("{name}.pdf")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_pdf_limit_and_signature() {
        let mut bytes = b"%PDF-".to_vec();
        bytes.resize(MAX_PDF_BYTES, b' ');
        assert!(validate(&bytes).is_ok());
        bytes.push(b' ');
        assert!(matches!(validate(&bytes), Err(AppError::PayloadTooLarge)));
        for invalid in [b"".as_slice(), b"fake.pdf", b" %PDF-", b"<html>"] {
            assert!(validate(invalid).is_err());
        }
    }
    #[test]
    fn download_filename_has_no_paths_or_header_controls() {
        assert_eq!(
            download_name(Some("../../报告\r\n.pdf"), "Ab12Cd34"),
            "报告.pdf"
        );
        assert_eq!(download_name(Some("Title"), "Ab12Cd34"), "Title.pdf");
        assert_eq!(download_name(Some("/..\\"), "Ab12Cd34"), "Ab12Cd34.pdf");
    }
}
