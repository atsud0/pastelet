#[derive(Clone, sqlx::FromRow)]
pub struct Paste {
    pub id: String,
    pub title: Option<String>,
    pub content: String,
    pub language: String,
    pub password_hash: Option<String>,
    pub burn_after_read: bool,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
pub struct PasteMetadata {
    pub id: String,
    pub password_hash: Option<String>,
    pub burn_after_read: bool,
    pub expires_at: Option<i64>,
}

impl Paste {
    pub fn is_expired_at(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiration_boundary_and_never() {
        let mut paste = Paste {
            id: "Ab12Cd34".into(),
            title: None,
            content: "hello".into(),
            language: "text".into(),
            password_hash: None,
            burn_after_read: false,
            created_at: 100,
            expires_at: Some(200),
        };
        assert!(!paste.is_expired_at(199));
        assert!(paste.is_expired_at(200));
        assert!(paste.is_expired_at(201));
        paste.expires_at = None;
        assert!(!paste.is_expired_at(i64::MAX));
    }
}
