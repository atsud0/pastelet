use std::str::FromStr;

use crate::error::AppError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expiration {
    TenMinutes,
    OneHour,
    #[default]
    OneDay,
    OneWeek,
    OneMonth,
    Never,
}

impl FromStr for Expiration {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "10m" => Ok(Self::TenMinutes),
            "1h" => Ok(Self::OneHour),
            "1d" => Ok(Self::OneDay),
            "1w" => Ok(Self::OneWeek),
            "1m" => Ok(Self::OneMonth),
            "never" => Ok(Self::Never),
            _ => Err(AppError::BadRequest("Invalid expiration")),
        }
    }
}

impl Expiration {
    pub fn expires_at(self, created_at: i64) -> Result<Option<i64>, AppError> {
        let seconds = match self {
            Self::TenMinutes => 600,
            Self::OneHour => 3600,
            Self::OneDay => 86400,
            Self::OneWeek => 7 * 86400,
            Self::OneMonth => 30 * 86400,
            Self::Never => return Ok(None),
        };
        created_at
            .checked_add(seconds)
            .map(Some)
            .ok_or(AppError::Internal("timestamp overflow"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_durations_and_default() {
        for (input, seconds) in [
            ("10m", 600),
            ("1h", 3600),
            ("1d", 86400),
            ("1w", 604800),
            ("1m", 2592000),
        ] {
            assert_eq!(
                input
                    .parse::<Expiration>()
                    .expect("expiration")
                    .expires_at(100)
                    .expect("timestamp"),
                Some(100 + seconds)
            );
        }
        assert_eq!(Expiration::default(), Expiration::OneDay);
        assert_eq!(Expiration::Never.expires_at(100).expect("never"), None);
    }

    #[test]
    fn invalid_parameters_and_overflow() {
        for input in ["", "1y", "0", "-1", "1D", " 1d", "null"] {
            assert!(matches!(
                input.parse::<Expiration>(),
                Err(AppError::BadRequest(_))
            ));
        }
        assert!(Expiration::OneDay.expires_at(i64::MAX).is_err());
    }
}
