use std::{net::SocketAddr, time::SystemTime};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
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
    database::{Database, DeviceDetail, DeviceSummary},
    gateway_jwt::{self, GatewayJwks},
    jobs::{JobRecord, NewJob},
    usage,
};

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
        .route("/api/v1/jobs/{job_id}", get(job_status))
        .route(
            "/api/v1/devices/{device_id}",
            get(device).delete(delete_device),
        )
        .route("/api/v1/settings", get(settings))
        .route("/api/v1/usage", get(usage_report))
        .fallback(get(asset))
        .with_state(state)
        .merge(gateway_jwt::routes(gateway_jwks));
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(listen = %address, "controller admin UI listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn overview(State(state): State<AdminState>) -> Result<Json<Overview>, AdminError> {
    let devices = state.database.list_devices().await?;
    let now = unix_time_seconds();
    let online_devices = devices
        .iter()
        .filter(|device| {
            device
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

async fn devices(State(state): State<AdminState>) -> Result<Json<Vec<DeviceSummary>>, AdminError> {
    Ok(Json(state.database.list_devices().await?))
}

async fn device(
    State(state): State<AdminState>,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceDetail>, AdminError> {
    state
        .database
        .get_device(&device_id)
        .await?
        .map(Json)
        .ok_or(AdminError::NotFound)
}

async fn delete_device(
    State(state): State<AdminState>,
    Path(device_id): Path<String>,
) -> Result<StatusCode, AdminError> {
    if !state.database.delete_device(&device_id).await? {
        return Err(AdminError::NotFound);
    }
    info!(%device_id, "deleted device from controller");
    Ok(StatusCode::NO_CONTENT)
}

async fn settings(State(state): State<AdminState>) -> Json<ControllerSettings> {
    Json(state.settings)
}

async fn usage_report(
    State(state): State<AdminState>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageReport>, AdminError> {
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
) -> Result<Json<ActiveDaemonConfig>, AdminError> {
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
    let actor = state.auth.authenticate(&headers).await?;
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
    state.auth.authenticate(&headers).await?;
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

enum AdminError {
    Internal(anyhow::Error),
    NotFound,
    BadRequest(&'static str),
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
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message).into_response(),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message).into_response(),
            Self::Internal(error) => {
                tracing::error!(%error, "admin API operation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "controller state error").into_response()
            }
        }
    }
}
