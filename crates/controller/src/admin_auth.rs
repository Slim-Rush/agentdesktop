//! Admin write authentication: verified OIDC subject + optional allowlist.

use axum::http::{HeaderMap, StatusCode, header};

use crate::oidc::OidcProvider;

/// A verified admin actor. `requested_by` is derived from this, never from a
/// caller-supplied header.
#[derive(Clone, Debug)]
pub struct AdminActor {
    pub issuer: String,
    pub subject: String,
}

impl AdminActor {
    pub fn requested_by(&self) -> String {
        format!("{}#{}", self.issuer, self.subject)
    }
}

#[derive(Clone)]
pub struct AdminAuth {
    oidc: Option<OidcProvider>,
    allowed_subjects: Vec<String>,
}

impl AdminAuth {
    pub fn new(oidc: Option<OidcProvider>, allowed_subjects: Vec<String>) -> Self {
        Self {
            oidc,
            allowed_subjects,
        }
    }

    /// Authenticate an `Authorization: Bearer <OIDC access token>` request.
    /// Missing OIDC configuration or an empty allowlist fails closed.
    pub async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<AdminActor, (StatusCode, &'static str)> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or((
                StatusCode::UNAUTHORIZED,
                "admin requests require an OIDC bearer token",
            ))?;
        let oidc = self.oidc.as_ref().ok_or((
            StatusCode::FORBIDDEN,
            "admin authentication is not configured",
        ))?;
        let principal = oidc
            .authenticate_access_token(token)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "admin access token is not valid"))?;
        if self.allowed_subjects.is_empty() || !self.allowed_subjects.contains(&principal.subject) {
            return Err((StatusCode::FORBIDDEN, "admin subject is not allowed"));
        }
        Ok(AdminActor {
            issuer: principal.issuer,
            subject: principal.subject,
        })
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, StatusCode, header};

    use super::AdminAuth;

    #[tokio::test]
    async fn a_missing_token_is_unauthorized() {
        let auth = AdminAuth::new(None, Vec::new());
        let error = auth.authenticate(&HeaderMap::new()).await.unwrap_err();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_token_without_a_configured_provider_is_forbidden() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer test".parse().unwrap());
        let auth = AdminAuth::new(None, Vec::new());
        let error = auth.authenticate(&headers).await.unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }
}
