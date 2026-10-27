//! Admin write authentication: verified OIDC subject + optional allowlist.

use axum::http::{HeaderMap, StatusCode, header};

use crate::oidc::OidcProvider;

/// Fixed CSRF marker for loopback-only admin writes when `adminAuth` is unset.
pub const ADMIN_WRITE_HEADER: &str = "x-agentdesktop-admin";

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
    /// True when the operator configured `adminAuth`, including an explicitly
    /// empty allowlist that must keep failing closed.
    write_enforcement: bool,
}

impl AdminAuth {
    pub fn new(oidc: Option<OidcProvider>, allowed_subjects: Vec<String>) -> Self {
        Self {
            write_enforcement: oidc.is_some() && !allowed_subjects.is_empty(),
            oidc,
            allowed_subjects,
        }
    }

    /// Marks writes as configured even when the allowlist is empty.
    pub fn with_write_enforcement(mut self, enforced: bool) -> Self {
        self.write_enforcement = enforced;
        self
    }

    /// Enforce authentication when an operator has configured it. Without an
    /// OIDC provider or allowlist the admin listener remains loopback-only.
    ///
    /// Reads keep the historical behavior: an empty allowlist leaves loopback
    /// reads open. Writes use `write_enforcement`, so an explicitly configured
    /// empty allowlist still fails closed.
    pub async fn require_if_configured(
        &self,
        headers: &HeaderMap,
    ) -> Result<(), (StatusCode, &'static str)> {
        if self.oidc.is_some() && !self.allowed_subjects.is_empty() {
            self.authenticate(headers).await.map(|_| ())?;
        }
        Ok(())
    }

    /// Authorize a mutating admin request.
    ///
    /// Configured `adminAuth` keeps the fail-closed bearer path. Without it,
    /// the loopback-only listener accepts the fixed CSRF marker, which an HTML
    /// form cannot set and cross-origin JavaScript cannot send without a
    /// preflight.
    pub async fn authorize_write(
        &self,
        headers: &HeaderMap,
    ) -> Result<AdminActor, (StatusCode, &'static str)> {
        if self.write_enforcement {
            return self.authenticate(headers).await;
        }
        match headers
            .get(ADMIN_WRITE_HEADER)
            .and_then(|value| value.to_str().ok())
        {
            Some("1") => Ok(AdminActor {
                issuer: "local".to_owned(),
                subject: "loopback-admin".to_owned(),
            }),
            _ => Err((
                StatusCode::FORBIDDEN,
                "admin writes require the x-agentdesktop-admin header",
            )),
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

    use super::{ADMIN_WRITE_HEADER, AdminAuth};

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

    #[tokio::test]
    async fn loopback_write_requires_the_csrf_header() {
        let auth = AdminAuth::new(None, Vec::new());
        let error = auth.authorize_write(&HeaderMap::new()).await.unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn loopback_write_records_a_local_actor() {
        let auth = AdminAuth::new(None, Vec::new());
        let mut headers = HeaderMap::new();
        headers.insert(ADMIN_WRITE_HEADER, "1".parse().unwrap());
        let actor = auth.authorize_write(&headers).await.unwrap();
        assert_eq!(actor.requested_by(), "local#loopback-admin");
    }

    #[tokio::test]
    async fn configured_write_enforcement_still_requires_a_token() {
        let auth = AdminAuth::new(None, Vec::new()).with_write_enforcement(true);
        let mut headers = HeaderMap::new();
        headers.insert(ADMIN_WRITE_HEADER, "1".parse().unwrap());
        let error = auth.authorize_write(&headers).await.unwrap_err();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }
}
