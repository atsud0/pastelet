use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};

use super::compute;
use crate::error::AppError;

pub const MAX_PASSWORD_BYTES: usize = 1024;

pub async fn hash(password: String) -> Result<String, AppError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AppError::BadRequest("Password is too long"));
    }
    compute(move || {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| AppError::Internal("OS randomness unavailable"))?;
        let salt = SaltString::encode_b64(&bytes)
            .map_err(|_| AppError::Internal("salt encoding failed"))?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| AppError::Internal("password hashing failed"))
    })
    .await
}

pub async fn verify(password: String, encoded: String) -> Result<bool, AppError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Ok(false);
    }
    compute(move || {
        let hash = PasswordHash::new(&encoded)
            .map_err(|_| AppError::Internal("invalid stored password hash"))?;
        match Argon2::default().verify_password(password.as_bytes(), &hash) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(_) => Err(AppError::Internal("password verification failed")),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn salted_hashes_verify_exact_passwords() {
        let first = hash(" 密码 ".into()).await.expect("hash");
        let second = hash(" 密码 ".into()).await.expect("hash");
        assert!(first.starts_with("$argon2id$"));
        assert_ne!(first, second);
        assert!(
            verify(" 密码 ".into(), first.clone())
                .await
                .expect("verify")
        );
        assert!(!verify("密码".into(), first.clone()).await.expect("verify"));
        assert!(!verify("".into(), first.clone()).await.expect("verify"));
        assert!(
            !verify("x".repeat(MAX_PASSWORD_BYTES + 1), first)
                .await
                .expect("verify")
        );
        assert!(hash("x".repeat(MAX_PASSWORD_BYTES + 1)).await.is_err());
        assert!(verify("password".into(), "invalid".into()).await.is_err());
    }
}
