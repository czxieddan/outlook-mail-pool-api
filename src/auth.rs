#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    Missing,
    Invalid,
}

pub fn authorize_header(header: Option<&str>, api_key: &str) -> Result<(), AuthError> {
    let configured = api_key.trim();
    let Some(raw) = header.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(AuthError::Missing);
    };
    let provided = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw)
        .trim();

    if !configured.is_empty() && provided == configured {
        Ok(())
    } else {
        Err(AuthError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthError, authorize_header};

    #[test]
    fn accepts_plain_bearer_and_x_api_key_style_values() {
        assert_eq!(
            authorize_header(Some("Bearer secret-key"), "secret-key"),
            Ok(())
        );
        assert_eq!(authorize_header(Some("secret-key"), "secret-key"), Ok(()));
    }

    #[test]
    fn rejects_missing_or_wrong_keys() {
        assert_eq!(
            authorize_header(None, "secret-key"),
            Err(AuthError::Missing)
        );
        assert_eq!(
            authorize_header(Some("Bearer nope"), "secret-key"),
            Err(AuthError::Invalid)
        );
    }
}
