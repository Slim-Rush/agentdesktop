use std::{net::SocketAddr, sync::Arc, time::SystemTime};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use tracing::info;

use agentdesktop_core::config::DaemonConfig;
use agentdesktop_proto::fleet::JobAction;
use agentdesktop_usage::{UsageError, UsageQuery, UsageReport};

use crate::{
    admin_auth::AdminAuth,
    catalog::CatalogStore,
    daemon_config::DaemonConfigStore,
    database::{AuditEventPage, AuditFilter, Database, DeviceDetail, DeviceState, DeviceSummary},
    gateway_jwt::{self, GatewayJwks},
    jobs::{JobRecord, NewJob},
    usage,
};

const DEFAULT_AUDIT_LIMIT: u32 = 50;

#[derive(Clone)]
pub struct AdminState {
    database: Database,
    daemon_config: DaemonConfigStore,
    catalog: CatalogStore,
    auth: AdminAuth,
    settings: ControllerSettings,
}

#[derive(Clone, Serialize)]
pub struct ControllerSettings {
    pub fleet_listen: String,
    pub admin_listen: String,
    pub oidc_enabled: bool,
    pub tls_enabled: bool,
    pub gateway_jwt_enabled: bool,
    pub usage_enabled: bool,
    pub max_devices_per_user: u32,
}

impl AdminState {
    pub fn new(
        database: Database,
        daemon_config: DaemonConfigStore,
        catalog: CatalogStore,
        auth: AdminAuth,
        settings: ControllerSettings,
    ) -> Self {
        Self {
            database,
            daemon_config,
            catalog,
            auth,
            settings,
        }
    }
}

#[derive(RustEmbed)]
#[folder = "../../frontend/controller/dist/"]
struct AdminAssets;

#[derive(Serialize)]
struct Overview {
    total_devices: usize,
    online_devices: usize,
    offline_devices: usize,
    config_failures: usize,
    active_revision: Option<u64>,
    recent_devices: Vec<DeviceSummary>,
}

#[derive(Clone)]
struct AllowedHosts(Arc<Vec<String>>);

/// The admin listener is loopback-only, but a browser can be pointed at
/// 127.0.0.1 through DNS rebinding while keeping its own `Host` header. Only
/// loopback host names are accepted so the loopback boundary is real at the
/// HTTP layer as well.
fn allowed_admin_hosts(address: SocketAddr) -> Vec<String> {
    let port = address.port();
    vec![
        "127.0.0.1".to_owned(),
        "localhost".to_owned(),
        "[::1]".to_owned(),
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
}

fn host_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    host.is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        allowed.iter().any(|candidate| candidate == &host)
    })
}

async fn require_allowed_host(
    State(AllowedHosts(allowed)): State<AllowedHosts>,
    request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host_allowed(host, &allowed) {
        return (StatusCode::FORBIDDEN, "admin host is not allowed").into_response();
    }
    next.run(request).await
}

pub async fn serve(
    address: SocketAddr,
    state: AdminState,
    gateway_jwks: Option<GatewayJwks>,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/devices", get(devices))
        .route("/api/v1/daemon-config", get(daemon_config))
        .route("/api/v1/devices/{device_id}/jobs", post(create_job))
        .route("/api/v1/devices/{device_id}/retire", post(retire_device))
        .route("/api/v1/devices/{device_id}/revoke", post(revoke_device))
        .route("/api/v1/devices/{device_id}/audit", get(device_audit))
        .route("/api/v1/audit", get(audit_events))
        .route("/api/v1/jobs/{job_id}", get(job_status))
        .route("/api/v1/devices/{device_id}", get(device))
        .route("/api/v1/settings", get(settings))
        .route("/api/v1/usage", get(usage_report))
        .fallback(get(asset))
        .with_state(state)
        .merge(gateway_jwt::routes(gateway_jwks));
    let listener = tokio::net::TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;
    let allowed_hosts = AllowedHosts(Arc::new(allowed_admin_hosts(bound)));
    let app = app.layer(middleware::from_fn_with_state(
        allowed_hosts,
        require_allowed_host,
    ));
    info!(listen = %bound, "controller admin UI listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn overview(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<Overview>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    let devices = state.database.list_devices().await?;
    let now = unix_time_seconds();
    let online_devices = devices
        .iter()
        .filter(|device| {
            device.state == "active"
                && device
                    .last_seen_at
                    .is_some_and(|last_seen| now - last_seen <= 90)
        })
        .count();
    let config_failures = devices
        .iter()
        .filter(|device| device.config_state == Some(2))
        .count();
    Ok(Json(Overview {
        total_devices: devices.len(),
        online_devices,
        offline_devices: devices.len() - online_devices,
        config_failures,
        active_revision: state.daemon_config.current().map(|config| config.revision),
        recent_devices: devices.into_iter().take(5).collect(),
    }))
}

#[derive(Deserialize)]
struct DeviceListQuery {
    state: Option<String>,
}

fn parse_device_state(value: &str) -> Option<DeviceState> {
    match value {
        "active" => Some(DeviceState::Active),
        "retired" => Some(DeviceState::Retired),
        "revoked" => Some(DeviceState::Revoked),
        _ => None,
    }
}

async fn devices(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<DeviceListQuery>,
) -> Result<Json<Vec<DeviceSummary>>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    let filter = query
        .state
        .as_deref()
        .map(|value| {
            parse_device_state(value).ok_or(AdminError::BadRequest("unknown device state"))
        })
        .transpose()?
        .map(DeviceState::as_str);
    Ok(Json(state.database.list_devices_by_state(filter).await?))
}

async fn device(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceDetail>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    state
        .database
        .get_device(&device_id)
        .await?
        .map(Json)
        .ok_or(AdminError::NotFound)
}

#[derive(Debug, Serialize)]
struct DeviceLifecycleResponse {
    id: String,
    state: String,
    changed: bool,
}

async fn retire_device(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceLifecycleResponse>, AdminError> {
    transition_device(state, headers, device_id, DeviceState::Retired).await
}

async fn revoke_device(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceLifecycleResponse>, AdminError> {
    transition_device(state, headers, device_id, DeviceState::Revoked).await
}

async fn transition_device(
    state: AdminState,
    headers: HeaderMap,
    device_id: String,
    target: DeviceState,
) -> Result<Json<DeviceLifecycleResponse>, AdminError> {
    let actor = state.auth.authorize_write(&headers).await?;
    let result = state
        .database
        .set_device_state(&device_id, target, &actor.requested_by())
        .await;
    let transition = match result {
        Ok(Some(transition)) => transition,
        Ok(None) => return Err(AdminError::NotFound),
        Err(error)
            if error
                .downcast_ref::<crate::database::InvalidDeviceTransition>()
                .is_some() =>
        {
            return Err(AdminError::Conflict(
                "a revoked device cannot be reinstated",
            ));
        }
        Err(error)
            if error
                .downcast_ref::<crate::database::DeviceTransitionConflict>()
                .is_some() =>
        {
            return Err(AdminError::Conflict(
                "device state changed concurrently; retry",
            ));
        }
        Err(error) => return Err(AdminError::Internal(error)),
    };
    info!(%device_id, state = %transition.state, "changed device lifecycle state");
    Ok(Json(DeviceLifecycleResponse {
        id: device_id,
        state: transition.state,
        changed: transition.changed,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditQuery {
    subject: Option<String>,
    action: Option<String>,
    device_id: Option<String>,
    after: Option<i64>,
    before: Option<String>,
    limit: Option<u32>,
}

fn audit_filter(query: AuditQuery) -> Result<AuditFilter, AdminError> {
    for (value, max) in [
        (query.subject.as_deref(), 512),
        (query.action.as_deref(), 64),
        (query.device_id.as_deref(), 128),
        (query.before.as_deref(), 128),
    ] {
        if value.is_some_and(|value| value.len() > max) {
            return Err(AdminError::BadRequest("audit filter is too long"));
        }
    }
    if let Some(before) = query.before.as_deref()
        && before
            .split_once(':')
            .and_then(|(created_at, _)| created_at.parse::<i64>().ok())
            .is_none()
    {
        return Err(AdminError::BadRequest("invalid audit cursor"));
    }
    Ok(AuditFilter {
        subject: query.subject,
        action: query.action,
        device_id: query.device_id,
        after: query.after,
        before: query.before,
        limit: query.limit.unwrap_or(DEFAULT_AUDIT_LIMIT),
        ..AuditFilter::default()
    })
}

async fn audit_events(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditEventPage>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    Ok(Json(
        state.database.audit_events(&audit_filter(query)?).await?,
    ))
}

async fn device_audit(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditEventPage>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    if !state.database.device_exists(&device_id).await? {
        return Err(AdminError::NotFound);
    }
    let mut filter = audit_filter(query)?;
    filter.device_id = None;
    filter.device_ids = Some(state.database.device_history(&device_id).await?);
    Ok(Json(state.database.audit_events(&filter).await?))
}

async fn settings(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<ControllerSettings>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    Ok(Json(state.settings))
}

async fn usage_report(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageReport>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    let labels = usage::labels(&state.database).await?;
    Ok(Json(
        agentdesktop_usage::report(
            &state.database.usage(),
            &query,
            state.settings.usage_enabled,
            &labels,
        )
        .await?,
    ))
}

#[derive(Serialize)]
struct ActiveDaemonConfig {
    config: Option<DaemonConfig>,
}

async fn daemon_config(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<ActiveDaemonConfig>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    let config = state
        .daemon_config
        .current()
        .map(|active| {
            let yaml = std::str::from_utf8(&active.yaml).context("daemon config is not UTF-8")?;
            agentdesktop_core::config::parse_daemon(yaml)
        })
        .transpose()?;
    Ok(Json(ActiveDaemonConfig { config }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateJobRequest {
    action: String,
    agent_id: String,
    version: String,
    idempotency_key: String,
}

async fn create_job(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<JobRecord>, AdminError> {
    let actor = state.auth.authorize_write(&headers).await?;
    if !state.database.device_exists(&device_id).await? {
        return Err(AdminError::NotFound);
    }
    let snapshot = state
        .catalog
        .current()
        .ok_or(AdminError::BadRequest("agent catalog is not configured"))?;
    let action = match request.action.as_str() {
        "install" => JobAction::Install,
        "uninstall" => JobAction::Uninstall,
        "update" => JobAction::Update,
        _ => return Err(AdminError::BadRequest("unknown job action")),
    };
    let entry = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.id == request.agent_id)
        .ok_or(AdminError::BadRequest("unknown agent"))?;
    if !request.version.is_empty()
        && !entry
            .versions
            .iter()
            .any(|version| version.version == request.version)
    {
        return Err(AdminError::BadRequest("unknown agent version"));
    }
    if request.idempotency_key.is_empty() {
        return Err(AdminError::BadRequest("idempotencyKey is required"));
    }
    let job = state
        .database
        .create_job(&NewJob {
            device_id,
            idempotency_key: request.idempotency_key,
            origin: "admin".to_owned(),
            requested_by: actor.requested_by(),
            catalog_id: snapshot.catalog.catalog_id.clone(),
            catalog_revision: snapshot.catalog.revision as i64,
            catalog_sha256: snapshot.sha256.clone(),
            agent_id: request.agent_id,
            version: request.version,
            action,
            deadline_unix_seconds: unix_time_seconds() + 900,
        })
        .await?;
    state
        .database
        .insert_audit(
            &actor.requested_by(),
            "job.create",
            &job.device_id,
            &serde_json::json!({
                "jobId": job.id,
                "agentId": job.agent_id,
                "action": job.action,
            }),
        )
        .await?;
    Ok(Json(job))
}

async fn job_status(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobRecord>, AdminError> {
    state.auth.require_if_configured(&headers).await?;
    state
        .database
        .job(&job_id)
        .await?
        .map(Json)
        .ok_or(AdminError::NotFound)
}

async fn asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (path, file) = AdminAssets::get(path)
        .map(|file| (path, file))
        .or_else(|| AdminAssets::get("index.html").map(|file| ("index.html", file)))
        .expect("embedded controller UI must contain index.html");
    let content_type = mime_guess::from_path(path).first_or_octet_stream();
    (
        [
            (header::CONTENT_TYPE, content_type.as_ref()),
            (
                header::CACHE_CONTROL,
                if path == "index.html" {
                    "no-cache"
                } else {
                    "public, max-age=31536000, immutable"
                },
            ),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::CONTENT_SECURITY_POLICY, "frame-ancestors 'none'"),
        ],
        file.data,
    )
        .into_response()
}

fn unix_time_seconds() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[derive(Debug)]
enum AdminError {
    Internal(anyhow::Error),
    NotFound,
    BadRequest(&'static str),
    Conflict(&'static str),
    Unauthorized(&'static str),
    Forbidden(&'static str),
}

impl From<(StatusCode, &'static str)> for AdminError {
    fn from((status, message): (StatusCode, &'static str)) -> Self {
        if status == StatusCode::UNAUTHORIZED {
            Self::Unauthorized(message)
        } else {
            Self::Forbidden(message)
        }
    }
}

impl From<anyhow::Error> for AdminError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl From<UsageError> for AdminError {
    fn from(error: UsageError) -> Self {
        match error {
            UsageError::InvalidQuery(message) => Self::BadRequest(message),
            UsageError::Storage(error) => Self::Internal(error),
        }
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "device not found").into_response(),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::Conflict(message) => (StatusCode::CONFLICT, message).into_response(),
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message).into_response(),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message).into_response(),
            Self::Internal(error) => {
                tracing::error!(%error, "admin API operation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "controller state error").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, response::IntoResponse};

    use super::{AuditQuery, DEFAULT_AUDIT_LIMIT, audit_filter, parse_device_state};

    #[test]
    fn device_state_filter_rejects_unknown_values() {
        for (value, expected) in [
            ("active", Some(crate::database::DeviceState::Active)),
            ("retired", Some(crate::database::DeviceState::Retired)),
            ("revoked", Some(crate::database::DeviceState::Revoked)),
            ("deleted", None),
            ("", None),
        ] {
            assert_eq!(parse_device_state(value), expected);
        }
    }

    #[test]
    fn audit_filter_defaults_and_bounds_user_input() {
        let filter = match audit_filter(AuditQuery {
            subject: None,
            action: None,
            device_id: None,
            after: None,
            before: None,
            limit: None,
        }) {
            Ok(filter) => filter,
            Err(_) => panic!("default filter must be valid"),
        };
        assert_eq!(filter.limit, DEFAULT_AUDIT_LIMIT);

        let error = match audit_filter(AuditQuery {
            subject: Some("s".repeat(513)),
            action: None,
            device_id: None,
            after: None,
            before: None,
            limit: None,
        }) {
            Ok(_) => panic!("oversized subject must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn device_list_and_audit_reads_expose_lifecycle_history() {
        use axum::{
            Json,
            extract::{Path, Query, State},
            http::HeaderMap,
        };

        use crate::{
            admin_auth::AdminAuth,
            catalog::CatalogStore,
            daemon_config::DaemonConfigStore,
            database::{Database, EnrollmentOutcome, EnrollmentRequest},
        };

        use super::{AdminState, ControllerSettings, device_audit, devices};

        let path = std::env::temp_dir().join(format!(
            "agentdesktop-admin-audit-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let database = Database::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect database");
        let old = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "old",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: None,
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("enroll old device");
        assert_eq!(old, EnrollmentOutcome::Enrolled { linked: false });
        let outcome = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "new",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: Some("old"),
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("link new device");
        assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: true });

        let state = AdminState::new(
            database.clone(),
            DaemonConfigStore::new(None),
            CatalogStore::empty(),
            AdminAuth::new(None, Vec::new()),
            ControllerSettings {
                fleet_listen: "127.0.0.1:8443".to_owned(),
                admin_listen: "127.0.0.1:8080".to_owned(),
                oidc_enabled: true,
                tls_enabled: true,
                gateway_jwt_enabled: false,
                usage_enabled: false,
                max_devices_per_user: 5,
            },
        );

        let Json(retired) = devices(
            State(state.clone()),
            HeaderMap::new(),
            Query(super::DeviceListQuery {
                state: Some("retired".to_owned()),
            }),
        )
        .await
        .expect("list retired devices");
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].id, "old");

        let Json(page) = device_audit(
            State(state),
            HeaderMap::new(),
            Path("new".to_owned()),
            Query(AuditQuery {
                subject: None,
                action: None,
                device_id: None,
                after: None,
                before: None,
                limit: Some(10),
            }),
        )
        .await
        .expect("read device audit");
        assert_eq!(page.events.len(), 2);
        assert!(
            page.events
                .iter()
                .any(|event| event.action == "device.re_enrolled")
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn admin_host_allowlist_is_loopback_only() {
        let allowed = super::allowed_admin_hosts("127.0.0.1:8080".parse().unwrap());
        assert!(super::host_allowed(Some("127.0.0.1:8080"), &allowed));
        assert!(super::host_allowed(Some("LOCALHOST:8080"), &allowed));
        assert!(super::host_allowed(Some("[::1]:8080"), &allowed));
        assert!(super::host_allowed(Some("127.0.0.1"), &allowed));
        assert!(!super::host_allowed(Some("evil.example:8080"), &allowed));
        assert!(!super::host_allowed(Some("127.0.0.1:9999"), &allowed));
        assert!(!super::host_allowed(
            Some("127.0.0.1:8080.evil.example"),
            &allowed
        ));
        assert!(!super::host_allowed(None, &allowed));
    }

    #[tokio::test]
    async fn admin_assets_refuse_framing() {
        use axum::http::{Uri, header};

        let response = super::asset(Uri::from_static("/")).await;
        assert_eq!(
            response
                .headers()
                .get(header::X_FRAME_OPTIONS)
                .and_then(|value| value.to_str().ok()),
            Some("DENY")
        );
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .and_then(|value| value.to_str().ok()),
            Some("frame-ancestors 'none'")
        );
    }

    #[tokio::test]
    async fn lifecycle_write_uses_loopback_csrf_authorization() {
        use axum::{
            Json,
            extract::{Path, State},
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
        };

        use crate::{
            admin_auth::AdminAuth, catalog::CatalogStore, daemon_config::DaemonConfigStore,
            database::Database,
        };

        use super::{AdminState, ControllerSettings, retire_device};

        let path = std::env::temp_dir().join(format!(
            "agentdesktop-admin-write-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let database = Database::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect database");
        database
            .enroll_device("device", "host", "issuer", "subject", None)
            .await
            .expect("enroll device");
        let state = AdminState::new(
            database.clone(),
            DaemonConfigStore::new(None),
            CatalogStore::empty(),
            AdminAuth::new(None, Vec::new()),
            ControllerSettings {
                fleet_listen: "127.0.0.1:8443".to_owned(),
                admin_listen: "127.0.0.1:8080".to_owned(),
                oidc_enabled: true,
                tls_enabled: true,
                gateway_jwt_enabled: false,
                usage_enabled: false,
                max_devices_per_user: 5,
            },
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-agentdesktop-admin", "1".parse().unwrap());
        let Json(result) = retire_device(State(state.clone()), headers, Path("device".to_owned()))
            .await
            .expect("loopback lifecycle write");
        assert_eq!(result.state, "retired");
        assert!(result.changed);

        let events = database
            .audit_events(&crate::database::AuditFilter {
                action: Some("device.retired".to_owned()),
                ..crate::database::AuditFilter::default()
            })
            .await
            .expect("read retire audit");
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].actor, "local#loopback-admin");

        let error = retire_device(State(state), HeaderMap::new(), Path("device".to_owned()))
            .await
            .expect_err("missing CSRF header must be rejected");
        assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);

        drop(database);
        let _ = std::fs::remove_file(path);
    }
}
