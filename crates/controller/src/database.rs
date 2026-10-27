use anyhow::Context;
use sqlx::{Any, AnyPool, Transaction, any::AnyPoolOptions};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;

use agentdesktop_core::model::{LocalModel, McpServer, ModelRuntime, Skill};
use agentdesktop_proto::fleet::{
    AgentState, ConfigStatus, Hello, Inventory, ProgramState, TelemetryEvent, telemetry_event,
};
use agentdesktop_usage::UsageStore;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    device_identity::{hostnames_match, valid_previous_device_id},
    jobs::{JobRecord, JobState, NewJob, event_for_state, transition},
};

const MAX_AUDIT_EVENTS_PER_PAGE: u32 = 200;
const DEFAULT_AUDIT_EVENTS_PER_PAGE: u32 = 50;
const MAX_LINEAGE_DEVICES: usize = 32;

#[derive(Clone)]
pub struct Database {
    pool: AnyPool,
    backend: DatabaseBackend,
    enrollment_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    device_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseBackend {
    Sqlite,
    Postgres,
}

pub struct DevicePrincipal {
    pub issuer: String,
    pub subject: String,
    pub idp_claims: Option<BTreeMap<String, serde_json::Value>>,
    pub state: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Active,
    Retired,
    Revoked,
}

impl DeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
            Self::Revoked => "revoked",
        }
    }
}

pub struct EnrollmentRequest<'a> {
    pub device_id: &'a str,
    pub hostname: &'a str,
    pub issuer: &'a str,
    pub subject: &'a str,
    pub idp_claims: Option<&'a BTreeMap<String, serde_json::Value>>,
    pub previous_device_id: Option<&'a str>,
    pub max_devices_per_user: u32,
    pub actor: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnrollmentOutcome {
    Enrolled { linked: bool },
    LimitReached,
}

#[derive(Clone, Debug)]
pub struct DeviceTransitionResult {
    pub state: String,
    pub changed: bool,
}

#[derive(Debug)]
pub struct InvalidDeviceTransition;

impl std::fmt::Display for InvalidDeviceTransition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a revoked device cannot be reinstated")
    }
}

impl std::error::Error for InvalidDeviceTransition {}

#[derive(Debug)]
pub struct DeviceTransitionConflict;

impl std::fmt::Display for DeviceTransitionConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("device state changed concurrently")
    }
}

impl std::error::Error for DeviceTransitionConflict {}

#[derive(Clone, Debug)]
pub struct AuditFilter {
    pub subject: Option<String>,
    pub action: Option<String>,
    pub device_id: Option<String>,
    pub device_ids: Option<Vec<String>>,
    pub after: Option<i64>,
    pub before: Option<String>,
    pub limit: u32,
}

impl Default for AuditFilter {
    fn default() -> Self {
        Self {
            subject: None,
            action: None,
            device_id: None,
            device_ids: None,
            after: None,
            before: None,
            limit: DEFAULT_AUDIT_EVENTS_PER_PAGE,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditEventRecord {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub device_id: String,
    pub subject: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventPage {
    pub events: Vec<AuditEventRecord>,
    pub next_cursor: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AuditEventRow {
    id: String,
    actor: String,
    action: String,
    device_id: String,
    subject: String,
    payload_json: String,
    created_at: i64,
}

impl TryFrom<AuditEventRow> for AuditEventRecord {
    type Error = anyhow::Error;

    fn try_from(row: AuditEventRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            actor: row.actor,
            action: row.action,
            device_id: row.device_id,
            subject: row.subject,
            payload: serde_json::from_str(&row.payload_json)
                .context("decode audit event payload")?,
            created_at: row.created_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceSummary {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub architecture: String,
    pub agent_version: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub enrolled_by_issuer: String,
    pub enrolled_by_subject: String,
    pub state: String,
    pub config_revision: Option<i64>,
    pub config_state: Option<i64>,
    pub config_error: Option<String>,
    pub config_updated_at: Option<i64>,
    pub discovery_count: i64,
    pub installed_tools: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct DeviceRow {
    id: String,
    hostname: String,
    os: String,
    architecture: String,
    agent_version: String,
    created_at: i64,
    last_seen_at: Option<i64>,
    enrolled_by_issuer: String,
    enrolled_by_subject: String,
    state: String,
    config_revision: Option<i64>,
    config_state: Option<i64>,
    config_error: Option<String>,
    config_updated_at: Option<i64>,
    discovery_count: i64,
}

impl From<DeviceRow> for DeviceSummary {
    fn from(row: DeviceRow) -> Self {
        Self {
            id: row.id,
            hostname: row.hostname,
            os: row.os,
            architecture: row.architecture,
            agent_version: row.agent_version,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            enrolled_by_issuer: row.enrolled_by_issuer,
            enrolled_by_subject: row.enrolled_by_subject,
            state: row.state,
            config_revision: row.config_revision,
            config_state: row.config_state,
            config_error: row.config_error,
            config_updated_at: row.config_updated_at,
            discovery_count: row.discovery_count,
            installed_tools: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceDiscovery {
    pub kind: String,
    pub version: String,
    pub path: String,
    pub mcp_servers: Vec<McpServer>,
    pub skills: Vec<Skill>,
}

#[derive(sqlx::FromRow)]
struct DeviceDiscoveryRow {
    kind: String,
    version: String,
    path: String,
    mcp_servers_json: String,
    skills_json: String,
}

impl TryFrom<DeviceDiscoveryRow> for DeviceDiscovery {
    type Error = anyhow::Error;

    fn try_from(row: DeviceDiscoveryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: row.kind,
            version: row.version,
            path: row.path,
            mcp_servers: serde_json::from_str(&row.mcp_servers_json)
                .context("decode discovered MCP servers")?,
            skills: serde_json::from_str(&row.skills_json).context("decode discovered skills")?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceDetail {
    #[serde(flatten)]
    pub device: DeviceSummary,
    pub retired_at: Option<i64>,
    pub superseded_by: Option<String>,
    pub replaces_device_id: Option<String>,
    pub active_devices_for_subject: i64,
    pub discoveries: Vec<DeviceDiscovery>,
    pub model_runtimes: Vec<ModelRuntime>,
    pub recent_events: Vec<TelemetryEventRecord>,
    /// Per-program configuration status, in the order the agent reported it.
    pub programs: Vec<ProgramStatusRecord>,
    /// `None` when this device has no configuration status at all; `Some(_)`
    /// once one has been reported, reflecting the reporting agent's
    /// `ConfigStatus.programs_reported`.
    pub programs_reported: Option<bool>,
}

/// Bounds on what an agent reports per program, as for telemetry fields.
pub(crate) fn validate_program_status(status: &ConfigStatus) -> anyhow::Result<()> {
    anyhow::ensure!(
        status.programs.len() <= 32,
        "too many programs in configuration status"
    );
    let mut seen = std::collections::BTreeSet::new();
    for program in &status.programs {
        anyhow::ensure!(
            !program.program.is_empty()
                && program.program.len() <= 64
                && program
                    .program
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "invalid program in configuration status"
        );
        anyhow::ensure!(
            seen.insert(program.program.as_str()),
            "duplicate program in configuration status"
        );
        anyhow::ensure!(
            program.detail.len() <= 2048,
            "program detail too long in configuration status"
        );
    }
    Ok(())
}

/// The stored name of a program state; values this controller does not know
/// are `unspecified`.
fn program_state_name(state: i64) -> &'static str {
    match i32::try_from(state)
        .ok()
        .and_then(|state| ProgramState::try_from(state).ok())
    {
        Some(ProgramState::Applied) => "applied",
        Some(ProgramState::Unchanged) => "unchanged",
        Some(ProgramState::Removed) => "removed",
        Some(ProgramState::Conflict) => "conflict",
        Some(ProgramState::Inactive) => "inactive",
        Some(ProgramState::Blocked) => "blocked",
        Some(ProgramState::OptedOut) => "opted_out",
        Some(ProgramState::Failed) => "failed",
        Some(ProgramState::Unspecified) | None => "unspecified",
    }
}

#[derive(sqlx::FromRow)]
struct ProgramStatusRow {
    program: String,
    state: i64,
    detail: String,
    revision: i64,
    updated_at: i64,
}

/// One managed program's stored configuration status. `revision` is the
/// controller revision the report answered; it does not change across hot
/// reloads of the same revision.
#[derive(Clone, Debug, Serialize)]
pub struct ProgramStatusRecord {
    pub program: String,
    /// `applied`, `unchanged`, `removed`, `conflict`, `inactive`, `blocked`,
    /// `failed`, or `unspecified` for a state this controller does not know.
    pub state: String,
    pub detail: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct ModelRuntimeRow {
    kind: String,
    models_json: String,
}

impl TryFrom<ModelRuntimeRow> for ModelRuntime {
    type Error = anyhow::Error;

    fn try_from(row: ModelRuntimeRow) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: row.kind,
            models: serde_json::from_str(&row.models_json)
                .context("decode discovered local models")?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TelemetryEventRecord {
    pub id: String,
    pub timestamp_unix_ms: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct TelemetryEventRow {
    id: String,
    timestamp_unix_ms: i64,
    event_type: String,
    payload_json: String,
}

impl TryFrom<TelemetryEventRow> for TelemetryEventRecord {
    type Error = anyhow::Error;

    fn try_from(row: TelemetryEventRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            timestamp_unix_ms: row.timestamp_unix_ms,
            event_type: row.event_type,
            payload: serde_json::from_str(&row.payload_json).context("decode telemetry payload")?,
        })
    }
}

impl Database {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .with_context(|| format!("connect to database {url}"))?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("run database migrations")?;
        Ok(Self {
            pool,
            backend: if url.starts_with("postgres") {
                DatabaseBackend::Postgres
            } else {
                DatabaseBackend::Sqlite
            },
            enrollment_locks: Arc::new(Mutex::new(HashMap::new())),
            device_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn subject_lock(&self, issuer: &str, subject: &str) -> Arc<AsyncMutex<()>> {
        let key = format!("{issuer}:{subject}");
        let mut locks = self
            .enrollment_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.entry(key).or_default().clone()
    }

    fn device_lock(&self, device_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .device_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.entry(device_id.to_owned()).or_default().clone()
    }

    /// Counts devices in the `active` lifecycle state for one OIDC subject.
    pub async fn active_device_count(&self, issuer: &str, subject: &str) -> anyhow::Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM devices
             WHERE enrolled_by_issuer = $1 AND enrolled_by_subject = $2 AND state = 'active'",
        )
        .bind(issuer)
        .bind(subject)
        .fetch_one(&self.pool)
        .await
        .context("count active devices")
    }

    pub async fn device_state(&self, device_id: &str) -> anyhow::Result<Option<String>> {
        sqlx::query_scalar("SELECT state FROM devices WHERE id = $1")
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await
            .context("load device lifecycle state")
    }

    pub(crate) fn pool(&self) -> &AnyPool {
        &self.pool
    }

    /// The usage tables of this database.
    pub fn usage(&self) -> UsageStore {
        UsageStore::new(self.pool.clone())
    }

    pub async fn enroll_device(
        &self,
        device_id: &str,
        hostname: &str,
        enrolled_by_issuer: &str,
        enrolled_by_subject: &str,
        idp_claims: Option<&BTreeMap<String, serde_json::Value>>,
    ) -> anyhow::Result<()> {
        let idp_claims_json = idp_claims
            .map(serde_json::to_string)
            .transpose()
            .context("encode verified IdP claims")?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO devices
                (id, hostname, created_at, last_seen_at, enrolled_by_issuer,
                 enrolled_by_subject, idp_claims_json)
             VALUES ($1, $2, $3, $3, $4, $5, $6)",
        )
        .bind(device_id)
        .bind(hostname)
        .bind(unix_time_seconds())
        .bind(enrolled_by_issuer)
        .bind(enrolled_by_subject)
        .bind(idp_claims_json)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Counts, links, audits, and inserts a device in one serialized
    /// transaction. A limit rejection commits only the audit row.
    pub async fn enroll_device_with_policy(
        &self,
        request: EnrollmentRequest<'_>,
    ) -> anyhow::Result<EnrollmentOutcome> {
        let lock = self.subject_lock(request.issuer, request.subject);
        let _guard = lock.lock().await;

        let idp_claims_json = request
            .idp_claims
            .map(serde_json::to_string)
            .transpose()
            .context("encode verified IdP claims")?;
        let now = unix_time_seconds();
        // Lock order: subject -> device -> transaction. `set_device_state`
        // takes the device lock before its transaction, so taking the
        // transaction first here would invert the order and risk exhausting
        // the connection pool when both paths wait on each other.
        let previous_id = valid_previous_device_id(request.previous_device_id);
        let device_lock = previous_id.map(|id| self.device_lock(id));
        let _device_guard = match &device_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let mut transaction = self.pool.begin().await?;
        if self.backend == DatabaseBackend::Postgres {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(format!("{}:{}", request.issuer, request.subject))
                .execute(&mut *transaction)
                .await?;
        }

        let mut linked = false;
        if let Some(previous_id) = previous_id {
            let previous: Option<(String, String, String, String, Option<String>)> =
                sqlx::query_as(
                    "SELECT enrolled_by_issuer, enrolled_by_subject, hostname, state, superseded_by
                 FROM devices WHERE id = $1",
                )
                .bind(previous_id)
                .fetch_optional(&mut *transaction)
                .await?;
            let decision = match previous {
                None => Some("unknown_device"),
                Some((issuer, subject, hostname, state, superseded_by)) => {
                    if issuer != request.issuer || subject != request.subject {
                        Some("subject_mismatch")
                    } else if !hostnames_match(&hostname, request.hostname) {
                        Some("hostname_mismatch")
                    } else if state == DeviceState::Revoked.as_str() {
                        Some("device_revoked")
                    } else if state != DeviceState::Active.as_str()
                        && state != DeviceState::Retired.as_str()
                    {
                        Some("state_unknown")
                    } else if superseded_by.is_some() {
                        Some("already_superseded")
                    } else {
                        None
                    }
                }
            };
            let link_decision = match decision {
                Some(reason) => Some(reason),
                None => {
                    link_previous_device(&mut transaction, previous_id, request.device_id, now)
                        .await?
                }
            };
            match link_decision {
                Some(reason) => {
                    insert_audit_tx(
                        &mut transaction,
                        request.actor,
                        "device.link_rejected",
                        "",
                        request.subject,
                        &serde_json::json!({
                            "previousDeviceId": previous_id,
                            "reason": reason,
                        }),
                    )
                    .await?;
                }
                None => linked = true,
            }
        }

        if !linked {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM devices
                 WHERE enrolled_by_issuer = $1 AND enrolled_by_subject = $2
                   AND state = 'active'",
            )
            .bind(request.issuer)
            .bind(request.subject)
            .fetch_one(&mut *transaction)
            .await?;
            if active >= i64::from(request.max_devices_per_user) {
                insert_audit_tx(
                    &mut transaction,
                    request.actor,
                    "device.limit_rejected",
                    "",
                    request.subject,
                    &serde_json::json!({
                        "limit": request.max_devices_per_user,
                        "hostname": request.hostname,
                    }),
                )
                .await?;
                transaction.commit().await?;
                return Ok(EnrollmentOutcome::LimitReached);
            }
        }

        sqlx::query(
            "INSERT INTO devices
                (id, hostname, created_at, last_seen_at, enrolled_by_issuer,
                 enrolled_by_subject, idp_claims_json, state, replaces_device_id)
             VALUES ($1, $2, $3, $3, $4, $5, $6, 'active', $7)",
        )
        .bind(request.device_id)
        .bind(request.hostname)
        .bind(now)
        .bind(request.issuer)
        .bind(request.subject)
        .bind(idp_claims_json)
        .bind(linked.then_some(request.previous_device_id).flatten())
        .execute(&mut *transaction)
        .await?;
        insert_audit_tx(
            &mut transaction,
            request.actor,
            if linked {
                "device.re_enrolled"
            } else {
                "device.enrolled"
            },
            request.device_id,
            request.subject,
            &serde_json::json!({
                "hostname": request.hostname,
                "previousDeviceId": request.previous_device_id.filter(|_| linked),
            }),
        )
        .await?;
        transaction.commit().await?;
        Ok(EnrollmentOutcome::Enrolled { linked })
    }

    /// Moves a device to `retired` or `revoked`. Repeating the same state is
    /// an idempotent success; a revoked device cannot return to `retired`.
    pub async fn set_device_state(
        &self,
        device_id: &str,
        target: DeviceState,
        actor: &str,
    ) -> anyhow::Result<Option<DeviceTransitionResult>> {
        let lock = self.device_lock(device_id);
        let _guard = lock.lock().await;
        let mut transaction = self.pool.begin().await?;
        let current: Option<(String, String)> =
            sqlx::query_as("SELECT state, enrolled_by_subject FROM devices WHERE id = $1")
                .bind(device_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some((current_state, subject)) = current else {
            return Ok(None);
        };
        if current_state == target.as_str() {
            transaction.commit().await?;
            return Ok(Some(DeviceTransitionResult {
                state: current_state,
                changed: false,
            }));
        }
        if current_state == DeviceState::Revoked.as_str() && target != DeviceState::Revoked {
            return Err(InvalidDeviceTransition.into());
        }
        let updated = sqlx::query(
            "UPDATE devices SET state = $2, retired_at = COALESCE(retired_at, $3)
             WHERE id = $1 AND state = $4",
        )
        .bind(device_id)
        .bind(target.as_str())
        .bind(unix_time_seconds())
        .bind(&current_state)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() == 0 {
            let current: Option<String> =
                sqlx::query_scalar("SELECT state FROM devices WHERE id = $1")
                    .bind(device_id)
                    .fetch_optional(&mut *transaction)
                    .await?;
            match current {
                Some(state) if state == target.as_str() => {
                    transaction.commit().await?;
                    return Ok(Some(DeviceTransitionResult {
                        state,
                        changed: false,
                    }));
                }
                None => return Ok(None),
                _ => return Err(DeviceTransitionConflict.into()),
            }
        }
        let action = if target == DeviceState::Revoked {
            "device.revoked"
        } else {
            "device.retired"
        };
        insert_audit_tx(
            &mut transaction,
            actor,
            action,
            device_id,
            &subject,
            &serde_json::json!({
                "previousState": current_state,
                "state": target.as_str(),
            }),
        )
        .await?;
        transaction.commit().await?;
        Ok(Some(DeviceTransitionResult {
            state: target.as_str().to_owned(),
            changed: true,
        }))
    }

    /// The requested device plus its replacements and successors, bounded so
    /// a corrupted chain cannot grow an audit query without limit.
    pub async fn device_history(&self, device_id: &str) -> anyhow::Result<Vec<String>> {
        let mut history = vec![device_id.to_owned()];
        let mut cursor = Some(device_id.to_owned());
        while history.len() < MAX_LINEAGE_DEVICES {
            let Some(current) = cursor else {
                break;
            };
            let previous: Option<Option<String>> =
                sqlx::query_scalar("SELECT replaces_device_id FROM devices WHERE id = $1")
                    .bind(&current)
                    .fetch_optional(&self.pool)
                    .await?;
            match previous.flatten() {
                Some(previous) if !history.contains(&previous) => {
                    history.push(previous.clone());
                    cursor = Some(previous);
                }
                _ => break,
            }
        }
        let mut cursor = Some(device_id.to_owned());
        while history.len() < MAX_LINEAGE_DEVICES {
            let Some(current) = cursor else {
                break;
            };
            let successor: Option<Option<String>> =
                sqlx::query_scalar("SELECT superseded_by FROM devices WHERE id = $1")
                    .bind(&current)
                    .fetch_optional(&self.pool)
                    .await?;
            match successor.flatten() {
                Some(successor) if !history.contains(&successor) => {
                    history.push(successor.clone());
                    cursor = Some(successor);
                }
                _ => break,
            }
        }
        Ok(history)
    }

    /// Reads audit rows newest first. `before` is the opaque cursor returned
    /// by the previous page and `limit` is clamped to 1..=200.
    pub async fn audit_events(&self, filter: &AuditFilter) -> anyhow::Result<AuditEventPage> {
        let limit = filter.limit.clamp(1, MAX_AUDIT_EVENTS_PER_PAGE);
        if filter
            .device_ids
            .as_ref()
            .is_some_and(|device_ids| device_ids.is_empty())
        {
            return Ok(AuditEventPage {
                events: Vec::new(),
                next_cursor: None,
            });
        }
        let (before_created_at, before_id) = match &filter.before {
            Some(before) => parse_audit_cursor(before)?,
            None => (0, String::new()),
        };
        // Explicit `$N` placeholders keep the query portable across the Any
        // driver's SQLite and PostgreSQL connections.
        let mut sql = String::from(
            "SELECT id, actor, action, device_id, subject, payload_json, created_at
             FROM audit_events
             WHERE ($1 IS NULL OR subject = $1)
               AND ($2 IS NULL OR action = $2)
               AND ($3 IS NULL OR device_id = $3)
               AND ($4 = 0 OR created_at >= $4)
               AND ($5 = 0 OR created_at < $5 OR (created_at = $5 AND id < $6))",
        );
        let device_ids = filter.device_ids.as_deref().unwrap_or_default();
        if !device_ids.is_empty() {
            sql.push_str(" AND device_id IN (");
            for index in 0..device_ids.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push_str(&format!("${}", 7 + index));
            }
            sql.push(')');
        }
        sql.push_str(&format!(
            " ORDER BY created_at DESC, id DESC LIMIT ${}",
            7 + device_ids.len()
        ));

        // Only placeholder numbers are interpolated; every value is bound.
        let mut query = sqlx::query_as::<_, AuditEventRow>(sqlx::AssertSqlSafe(sql))
            .bind(filter.subject.clone())
            .bind(filter.action.clone())
            .bind(filter.device_id.clone())
            .bind(filter.after.unwrap_or(0))
            .bind(before_created_at)
            .bind(before_id);
        for device_id in device_ids {
            query = query.bind(device_id.clone());
        }
        let rows: Vec<AuditEventRow> = query
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await?;
        let has_more = rows.len() > limit as usize;
        let events: Vec<AuditEventRecord> = rows
            .into_iter()
            .take(limit as usize)
            .map(AuditEventRecord::try_from)
            .collect::<anyhow::Result<_>>()?;
        let next_cursor = has_more
            .then(|| events.last())
            .flatten()
            .map(|event| format!("{}:{}", event.created_at, event.id));
        Ok(AuditEventPage {
            events,
            next_cursor,
        })
    }

    pub async fn device_exists(&self, device_id: &str) -> anyhow::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM devices WHERE id = $1")
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn device_principal(&self, device_id: &str) -> anyhow::Result<DevicePrincipal> {
        let (issuer, subject, idp_claims_json, state): (String, String, Option<String>, String) =
            sqlx::query_as(
                "SELECT enrolled_by_issuer, enrolled_by_subject, idp_claims_json, state
                 FROM devices WHERE id = $1",
            )
            .bind(device_id)
            .fetch_one(&self.pool)
            .await
            .context("load device enrollment principal")?;
        let idp_claims = idp_claims_json
            .map(|claims| serde_json::from_str(&claims).context("decode verified IdP claims"))
            .transpose()?;
        Ok(DevicePrincipal {
            issuer,
            subject,
            idp_claims,
            state,
        })
    }

    pub async fn list_devices(&self) -> anyhow::Result<Vec<DeviceSummary>> {
        self.list_devices_by_state(None).await
    }

    pub async fn list_devices_by_state(
        &self,
        state: Option<&str>,
    ) -> anyhow::Result<Vec<DeviceSummary>> {
        let rows: Vec<DeviceRow> = sqlx::query_as(
            "SELECT d.id, d.hostname, d.os, d.architecture, d.agent_version,
                    d.created_at, d.last_seen_at, d.enrolled_by_issuer,
                    d.enrolled_by_subject, d.state, cs.revision AS config_revision,
                    cs.state AS config_state, cs.error AS config_error,
                    cs.updated_at AS config_updated_at, COUNT(x.kind) AS discovery_count
             FROM devices d
             LEFT JOIN device_config_status cs ON cs.device_id = d.id
             LEFT JOIN discoveries x ON x.device_id = d.id
             WHERE ($1 IS NULL OR d.state = $1)
             GROUP BY d.id, d.hostname, d.os, d.architecture, d.agent_version,
                      d.created_at, d.last_seen_at, d.enrolled_by_issuer,
                      d.enrolled_by_subject, d.state, cs.revision, cs.state, cs.error,
                      cs.updated_at
             ORDER BY d.last_seen_at DESC, d.hostname ASC",
        )
        .bind(state)
        .fetch_all(&self.pool)
        .await
        .context("list devices")?;
        let mut devices: Vec<DeviceSummary> = rows.into_iter().map(Into::into).collect();
        let discovered_tools: Vec<(String, String)> =
            sqlx::query_as("SELECT device_id, kind FROM discoveries ORDER BY device_id, kind")
                .fetch_all(&self.pool)
                .await
                .context("list installed developer tools")?;
        let mut tools_by_device: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (device_id, kind) in discovered_tools {
            tools_by_device.entry(device_id).or_default().push(kind);
        }
        for device in &mut devices {
            device.installed_tools = tools_by_device.remove(&device.id).unwrap_or_default();
        }
        Ok(devices)
    }

    pub async fn get_device(&self, device_id: &str) -> anyhow::Result<Option<DeviceDetail>> {
        let device: Option<DeviceRow> = sqlx::query_as(
            "SELECT d.id, d.hostname, d.os, d.architecture, d.agent_version,
                    d.created_at, d.last_seen_at, d.enrolled_by_issuer,
                    d.enrolled_by_subject, d.state, cs.revision AS config_revision,
                    cs.state AS config_state, cs.error AS config_error,
                    cs.updated_at AS config_updated_at, COUNT(x.kind) AS discovery_count
             FROM devices d
             LEFT JOIN device_config_status cs ON cs.device_id = d.id
             LEFT JOIN discoveries x ON x.device_id = d.id
             WHERE d.id = $1
             GROUP BY d.id, d.hostname, d.os, d.architecture, d.agent_version,
                      d.created_at, d.last_seen_at, d.enrolled_by_issuer,
                      d.enrolled_by_subject, d.state, cs.revision, cs.state, cs.error,
                      cs.updated_at",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .context("load device")?;
        let Some(device) = device else {
            return Ok(None);
        };
        let (retired_at, superseded_by, replaces_device_id): (
            Option<i64>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT retired_at, superseded_by, replaces_device_id FROM devices WHERE id = $1",
        )
        .bind(device_id)
        .fetch_one(&self.pool)
        .await
        .context("load device lifecycle metadata")?;
        let rows: Vec<DeviceDiscoveryRow> = sqlx::query_as(
            "SELECT kind, version, path, mcp_servers_json, skills_json FROM discoveries
             WHERE device_id = $1 ORDER BY kind ASC, path ASC",
        )
        .bind(device_id)
        .fetch_all(&self.pool)
        .await
        .context("load device discoveries")?;
        let discoveries: Vec<DeviceDiscovery> = rows
            .into_iter()
            .map(DeviceDiscovery::try_from)
            .collect::<anyhow::Result<_>>()?;
        let rows: Vec<ModelRuntimeRow> = sqlx::query_as(
            "SELECT kind, models_json FROM model_runtimes
             WHERE device_id = $1 ORDER BY kind ASC",
        )
        .bind(device_id)
        .fetch_all(&self.pool)
        .await
        .context("load discovered model runtimes")?;
        let model_runtimes = rows
            .into_iter()
            .map(ModelRuntime::try_from)
            .collect::<anyhow::Result<_>>()?;
        let mut device: DeviceSummary = device.into();
        device.installed_tools = discoveries
            .iter()
            .map(|discovery: &DeviceDiscovery| discovery.kind.clone())
            .collect();
        let recent_events = self.recent_telemetry(device_id, 50).await?;
        let programs_reported: Option<i64> = sqlx::query_scalar(
            "SELECT programs_reported FROM device_config_status WHERE device_id = $1",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .context("load configuration status reporting")?;
        let rows: Vec<ProgramStatusRow> = sqlx::query_as(
            "SELECT program, state, detail, revision, updated_at FROM device_program_status
             WHERE device_id = $1 ORDER BY position ASC",
        )
        .bind(device_id)
        .fetch_all(&self.pool)
        .await
        .context("load program configuration status")?;
        let programs = rows
            .into_iter()
            .map(|row| ProgramStatusRecord {
                program: row.program,
                state: program_state_name(row.state).to_owned(),
                detail: row.detail,
                revision: row.revision,
                updated_at: row.updated_at,
            })
            .collect();
        let active_devices_for_subject = if device.enrolled_by_subject.is_empty() {
            0
        } else {
            self.active_device_count(&device.enrolled_by_issuer, &device.enrolled_by_subject)
                .await?
        };
        Ok(Some(DeviceDetail {
            device,
            retired_at,
            superseded_by,
            replaces_device_id,
            active_devices_for_subject,
            discoveries,
            model_runtimes,
            recent_events,
            programs,
            programs_reported: programs_reported.map(|reported| reported != 0),
        }))
    }

    pub async fn delete_device(&self, device_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM devices WHERE id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await
            .context("delete device")?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_hello(&self, device_id: &str, hello: &Hello) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE devices
             SET hostname = $2, os = $3, architecture = $4, agent_version = $5,
                 last_seen_at = $6
             WHERE id = $1",
        )
        .bind(device_id)
        .bind(&hello.hostname)
        .bind(&hello.os)
        .bind(&hello.architecture)
        .bind(&hello.agent_version)
        .bind(unix_time_seconds())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_heartbeat(&self, device_id: &str, timestamp: u64) -> anyhow::Result<()> {
        sqlx::query("UPDATE devices SET last_seen_at = $2 WHERE id = $1")
            .bind(device_id)
            .bind(i64::try_from(timestamp).unwrap_or(i64::MAX))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Stores the catalog and per-agent configuration state reported by a device.
    pub async fn update_agent_state(
        &self,
        device_id: &str,
        state: &AgentState,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            state.catalog_sha256.len() == 64
                && state
                    .catalog_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "invalid catalog sha256"
        );
        anyhow::ensure!(
            state.agents.len() <= 64,
            "too many agent states in one report"
        );
        for agent in &state.agents {
            anyhow::ensure!(
                !agent.agent_id.is_empty()
                    && agent.agent_id.len() <= 64
                    && !agent.program_key.is_empty()
                    && agent.program_key.len() <= 64
                    && agent.detail.len() <= 2048,
                "invalid agent state entry"
            );
        }
        let agents_json = serde_json::to_string(
            &state
                .agents
                .iter()
                .map(|agent| {
                    serde_json::json!({
                        "agentId": agent.agent_id,
                        "programKey": agent.program_key,
                        "state": agent.state,
                        "detail": agent.detail,
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        let now = unix_time_seconds();
        sqlx::query(
            "INSERT INTO device_agent_state
                (device_id, catalog_revision, catalog_sha256, last_sync_at, stale,
                 agents_json, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (device_id) DO UPDATE SET
                catalog_revision = EXCLUDED.catalog_revision,
                catalog_sha256 = EXCLUDED.catalog_sha256,
                last_sync_at = EXCLUDED.last_sync_at,
                stale = EXCLUDED.stale,
                agents_json = EXCLUDED.agents_json,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(device_id)
        .bind(i64::try_from(state.catalog_revision).unwrap_or(i64::MAX))
        .bind(&state.catalog_sha256)
        .bind(i64::try_from(state.last_sync_unix_seconds).unwrap_or(i64::MAX))
        .bind(if state.stale { 1_i64 } else { 0_i64 })
        .bind(agents_json)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Creates a queued job, or returns the existing one for the same
    /// `(device_id, idempotency_key)` pair.
    pub async fn create_job(&self, job: &NewJob) -> anyhow::Result<JobRecord> {
        let id = Uuid::new_v4().to_string();
        let now = unix_time_seconds();
        sqlx::query(
            "INSERT INTO agent_jobs
                (id, device_id, attempt, idempotency_key, origin, requested_by,
                 catalog_id, catalog_revision, catalog_sha256, agent_id, version,
                 action, state, deadline_unix_seconds, error_code, error_message,
                 diagnostics_json, created_at, updated_at)
             VALUES ($1,$2,1,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'','','',$14,$14)
             ON CONFLICT (device_id, idempotency_key) DO NOTHING",
        )
        .bind(&id)
        .bind(&job.device_id)
        .bind(&job.idempotency_key)
        .bind(&job.origin)
        .bind(&job.requested_by)
        .bind(&job.catalog_id)
        .bind(job.catalog_revision)
        .bind(&job.catalog_sha256)
        .bind(&job.agent_id)
        .bind(&job.version)
        .bind(job.action as i64)
        .bind(JobState::Queued.proto_value())
        .bind(job.deadline_unix_seconds)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.job_by_idempotency(&job.device_id, &job.idempotency_key)
            .await?
            .context("job disappeared after insert")
    }

    pub async fn job_by_idempotency(
        &self,
        device_id: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<Option<JobRecord>> {
        Ok(sqlx::query_as::<_, JobRecord>(
            "SELECT * FROM agent_jobs WHERE device_id = $1 AND idempotency_key = $2",
        )
        .bind(device_id)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn job(&self, id: &str) -> anyhow::Result<Option<JobRecord>> {
        Ok(
            sqlx::query_as::<_, JobRecord>("SELECT * FROM agent_jobs WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Jobs a reconnecting or newly connected device still needs to run.
    pub async fn pending_jobs(&self, device_id: &str) -> anyhow::Result<Vec<JobRecord>> {
        Ok(sqlx::query_as::<_, JobRecord>(
            "SELECT * FROM agent_jobs
             WHERE device_id = $1 AND state NOT IN ($2,$3,$4,$5)
             ORDER BY created_at",
        )
        .bind(device_id)
        .bind(JobState::Succeeded.proto_value())
        .bind(JobState::Failed.proto_value())
        .bind(JobState::Cancelled.proto_value())
        .bind(JobState::Expired.proto_value())
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn mark_job_dispatched(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE agent_jobs SET state = $2, updated_at = $3
             WHERE id = $1 AND state = $4",
        )
        .bind(id)
        .bind(JobState::Dispatched.proto_value())
        .bind(unix_time_seconds())
        .bind(JobState::Queued.proto_value())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Updates a non-terminal job only when the reporting attempt is current.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_job_status(
        &self,
        device_id: &str,
        job_id: &str,
        attempt: u32,
        state: JobState,
        error_code: &str,
        error_message: &str,
        diagnostics_json: &str,
    ) -> anyhow::Result<bool> {
        let Some(current) = self.job(job_id).await? else {
            return Ok(false);
        };
        let Some(current_state) = current.state() else {
            return Ok(false);
        };
        let Some(event) = event_for_state(state) else {
            return Ok(false);
        };
        if transition(current_state, event) != Some(state) {
            return Ok(false);
        }
        let result = sqlx::query(
            "UPDATE agent_jobs
             SET state = $4, error_code = $5, error_message = $6,
                 diagnostics_json = $7, updated_at = $8
             WHERE id = $1 AND device_id = $2 AND attempt = $3
               AND state NOT IN ($9,$10,$11,$12)",
        )
        .bind(job_id)
        .bind(device_id)
        .bind(i64::from(attempt))
        .bind(state.proto_value())
        .bind(error_code)
        .bind(error_message)
        .bind(diagnostics_json)
        .bind(unix_time_seconds())
        .bind(JobState::Succeeded.proto_value())
        .bind(JobState::Failed.proto_value())
        .bind(JobState::Cancelled.proto_value())
        .bind(JobState::Expired.proto_value())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn insert_audit(
        &self,
        actor: &str,
        action: &str,
        device_id: &str,
        payload: &serde_json::Value,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO audit_events
                (id, actor, action, device_id, subject, payload_json, created_at)
             VALUES ($1,$2,$3,$4,
                     COALESCE((SELECT enrolled_by_subject FROM devices WHERE id = $4), ''),
                     $5,$6)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(actor)
        .bind(action)
        .bind(device_id)
        .bind(serde_json::to_string(payload)?)
        .bind(unix_time_seconds())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn replace_inventory(
        &self,
        device_id: &str,
        inventory: &Inventory,
    ) -> anyhow::Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM discoveries WHERE device_id = $1")
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        for discovery in &inventory.discoveries {
            let mcp_servers: Vec<McpServer> = discovery
                .mcp_servers
                .iter()
                .map(|server| McpServer {
                    name: server.name.clone(),
                    transport: server.transport.clone(),
                    command: (!server.command.is_empty()).then(|| server.command.clone()),
                    url: (!server.url.is_empty()).then(|| server.url.clone()),
                    enabled: server.enabled,
                    source: PathBuf::from(&server.source),
                })
                .collect();
            let skills: Vec<Skill> = discovery
                .skills
                .iter()
                .map(|skill| {
                    let front_matter = serde_json::from_slice(&skill.front_matter_json)
                        .context("decode reported skill front matter")?;
                    Ok(Skill {
                        path: PathBuf::from(&skill.path),
                        front_matter,
                    })
                })
                .collect::<anyhow::Result<_>>()?;
            let mcp_servers_json =
                serde_json::to_string(&mcp_servers).context("encode discovered MCP servers")?;
            let skills_json = serde_json::to_string(&skills).context("encode discovered skills")?;
            sqlx::query(
                "INSERT INTO discoveries
                    (device_id, kind, version, path, mcp_servers_json, skills_json)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(device_id)
            .bind(&discovery.kind)
            .bind(&discovery.version)
            .bind(&discovery.path)
            .bind(mcp_servers_json)
            .bind(skills_json)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("DELETE FROM model_runtimes WHERE device_id = $1")
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        for runtime in &inventory.model_runtimes {
            let models = runtime
                .models
                .iter()
                .map(|model| LocalModel {
                    name: model.name.clone(),
                })
                .collect::<Vec<_>>();
            let models_json =
                serde_json::to_string(&models).context("encode discovered local models")?;
            sqlx::query(
                "INSERT INTO model_runtimes (device_id, kind, models_json)
                 VALUES ($1, $2, $3)",
            )
            .bind(device_id)
            .bind(&runtime.kind)
            .bind(models_json)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Stores only the device-wide status (revision, state, error) and keeps
    /// the program rows and `programs_reported` of the last accepted report.
    pub async fn update_device_config_state(
        &self,
        device_id: &str,
        status: &ConfigStatus,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO device_config_status
                (device_id, revision, state, error, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (device_id) DO UPDATE SET
                revision = excluded.revision,
                state = excluded.state,
                error = excluded.error,
                updated_at = excluded.updated_at",
        )
        .bind(device_id)
        .bind(i64::try_from(status.revision).unwrap_or(i64::MAX))
        .bind(i64::from(status.state))
        .bind(&status.error)
        .bind(unix_time_seconds())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_config_status(
        &self,
        device_id: &str,
        status: &ConfigStatus,
    ) -> anyhow::Result<()> {
        validate_program_status(status)?;
        let now = unix_time_seconds();
        let revision = i64::try_from(status.revision).unwrap_or(i64::MAX);
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO device_config_status
                (device_id, revision, state, error, updated_at, programs_reported)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (device_id) DO UPDATE SET
                revision = excluded.revision,
                state = excluded.state,
                error = excluded.error,
                updated_at = excluded.updated_at,
                programs_reported = excluded.programs_reported",
        )
        .bind(device_id)
        .bind(revision)
        .bind(i64::from(status.state))
        .bind(&status.error)
        .bind(now)
        .bind(i64::from(status.programs_reported))
        .execute(&mut *transaction)
        .await?;
        // The report replaces the previous one: a program missing from it is
        // gone from the table.
        sqlx::query("DELETE FROM device_program_status WHERE device_id = $1")
            .bind(device_id)
            .execute(&mut *transaction)
            .await?;
        for (position, program) in status.programs.iter().enumerate() {
            sqlx::query(
                "INSERT INTO device_program_status
                    (device_id, program, position, revision, state, detail, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(device_id)
            .bind(&program.program)
            .bind(i64::try_from(position).unwrap_or(i64::MAX))
            .bind(revision)
            .bind(i64::from(program.state))
            .bind(&program.detail)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn insert_telemetry(
        &self,
        device_id: &str,
        telemetry: &TelemetryEvent,
    ) -> anyhow::Result<()> {
        let (event_type, payload) =
            match telemetry.event.as_ref().context("telemetry has no event")? {
                telemetry_event::Event::SessionNew(event) => {
                    if event.client_id.is_empty() || event.client_id.len() > 64 {
                        anyhow::bail!("invalid telemetry client ID");
                    }
                    if event.session_id.is_empty() || event.session_id.len() > 256 {
                        anyhow::bail!("invalid telemetry session ID");
                    }
                    (
                        "session.new",
                        serde_json::json!({
                            "clientId": event.client_id,
                            "sessionId": event.session_id,
                        }),
                    )
                }
                telemetry_event::Event::ToolUse(event) => {
                    if event.client_id.is_empty() || event.client_id.len() > 64 {
                        anyhow::bail!("invalid telemetry client ID");
                    }
                    if event.tool_name.is_empty() || event.tool_name.len() > 128 {
                        anyhow::bail!("invalid telemetry tool name");
                    }
                    if event.tool_use_id.len() > 256 {
                        anyhow::bail!("invalid telemetry tool use ID");
                    }
                    if event.input_json.len() > 256 * 1024 {
                        anyhow::bail!("telemetry tool input is too large");
                    }
                    let tool_input = (!event.input_json.is_empty())
                        .then(|| {
                            serde_json::from_slice(&event.input_json)
                                .context("decode telemetry tool input")
                        })
                        .transpose()?;
                    let mut payload = serde_json::json!({
                        "clientId": event.client_id,
                        "toolName": event.tool_name,
                        "toolUseId": (!event.tool_use_id.is_empty()).then_some(&event.tool_use_id),
                    });
                    if let Some(tool_input) = tool_input {
                        payload["toolInput"] = tool_input;
                    }
                    ("tool.use", payload)
                }
            };
        sqlx::query(
            "INSERT INTO telemetry_events
                (id, device_id, timestamp_unix_ms, event_type, payload_json)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(device_id)
        .bind(i64::try_from(telemetry.timestamp_unix_ms).unwrap_or(i64::MAX))
        .bind(event_type)
        .bind(serde_json::to_string(&payload).context("encode telemetry payload")?)
        .execute(&self.pool)
        .await
        .context("store telemetry event")?;
        Ok(())
    }

    async fn recent_telemetry(
        &self,
        device_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<TelemetryEventRecord>> {
        let rows: Vec<TelemetryEventRow> = sqlx::query_as(
            "SELECT id, timestamp_unix_ms, event_type, payload_json
             FROM telemetry_events
             WHERE device_id = $1
             ORDER BY timestamp_unix_ms DESC
             LIMIT $2",
        )
        .bind(device_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("load recent telemetry")?;
        rows.into_iter()
            .map(TelemetryEventRecord::try_from)
            .collect()
    }
}

fn parse_audit_cursor(cursor: &str) -> anyhow::Result<(i64, String)> {
    let (created_at, id) = cursor
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid audit cursor"))?;
    Ok((
        created_at
            .parse()
            .context("invalid audit cursor timestamp")?,
        id.to_owned(),
    ))
}

async fn insert_audit_tx(
    transaction: &mut Transaction<'_, Any>,
    actor: &str,
    action: &str,
    device_id: &str,
    subject: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audit_events
            (id, actor, action, device_id, subject, payload_json, created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(actor)
    .bind(action)
    .bind(device_id)
    .bind(subject)
    .bind(serde_json::to_string(payload)?)
    .bind(unix_time_seconds())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Retires the predecessor only if it is still linkable at write time. The
/// caller's earlier SELECT is only a fast path: an admin revoke or another
/// enrollment can change the row between the SELECT and this UPDATE.
async fn link_previous_device(
    transaction: &mut Transaction<'_, Any>,
    previous_id: &str,
    new_device_id: &str,
    retired_at: i64,
) -> anyhow::Result<Option<&'static str>> {
    let updated = sqlx::query(
        "UPDATE devices
         SET state = 'retired', retired_at = $2, superseded_by = $3
         WHERE id = $1 AND state IN ('active', 'retired') AND superseded_by IS NULL",
    )
    .bind(previous_id)
    .bind(retired_at)
    .bind(new_device_id)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() == 1 {
        return Ok(None);
    }

    let current: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT state, superseded_by FROM devices WHERE id = $1")
            .bind(previous_id)
            .fetch_optional(&mut **transaction)
            .await?;
    let reason = match current {
        None => "unknown_device",
        Some((state, _)) if state == DeviceState::Revoked.as_str() => "device_revoked",
        Some((state, Some(_)))
            if state == DeviceState::Active.as_str() || state == DeviceState::Retired.as_str() =>
        {
            "already_superseded"
        }
        Some((state, _))
            if state != DeviceState::Active.as_str() && state != DeviceState::Retired.as_str() =>
        {
            "state_unknown"
        }
        Some(_) => "state_unknown",
    };
    Ok(Some(reason))
}

fn unix_time_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use agentdesktop_proto::fleet::{
        ConfigState, ConfigStatus, Discovery, Inventory, JobAction, LocalModel, McpServer,
        ModelRuntime, ProgramState, ProgramStatus, Skill, TelemetryEvent, ToolUseEvent,
        telemetry_event,
    };
    use sqlx::any::AnyPoolOptions;

    use super::{
        AuditFilter, Database, DeviceState, EnrollmentOutcome, EnrollmentRequest,
        link_previous_device,
    };
    use crate::jobs::{JobState, NewJob};

    #[tokio::test]
    async fn jobs_are_idempotent_and_only_a_non_terminal_attempt_updates() {
        let path = std::env::temp_dir().join(format!(
            "agentdesktop-jobs-test-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let database = Database::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect test database");
        database
            .enroll_device("device", "host", "issuer", "subject", None)
            .await
            .expect("enroll device");
        assert!(database.device_exists("device").await.unwrap());
        assert!(!database.device_exists("missing").await.unwrap());
        let job = NewJob {
            device_id: "device".to_owned(),
            idempotency_key: "idem".to_owned(),
            origin: "admin".to_owned(),
            requested_by: "subject".to_owned(),
            catalog_id: "catalog".to_owned(),
            catalog_revision: 1,
            catalog_sha256: "a".repeat(64),
            agent_id: "claude-code".to_owned(),
            version: "1.0.0".to_owned(),
            action: JobAction::Install,
            deadline_unix_seconds: 0,
        };
        let first = database.create_job(&job).await.expect("create job");
        let second = database.create_job(&job).await.expect("idempotent job");
        assert_eq!(first.id, second.id);
        assert_eq!(database.pending_jobs("device").await.unwrap().len(), 1);

        database.mark_job_dispatched(&first.id).await.unwrap();
        // A daemon cannot skip straight from dispatched to succeeded.
        assert!(
            !database
                .update_job_status("device", &first.id, 1, JobState::Succeeded, "", "", "",)
                .await
                .unwrap()
        );
        for state in [JobState::Accepted, JobState::Running, JobState::Succeeded] {
            assert!(
                database
                    .update_job_status("device", &first.id, 1, state, "", "", "",)
                    .await
                    .unwrap()
            );
        }
        assert!(
            !database
                .update_job_status("device", &first.id, 1, JobState::Failed, "late", "late", "",)
                .await
                .unwrap()
        );
        assert!(database.pending_jobs("device").await.unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn persists_device_inventory_and_telemetry() {
        let path = std::env::temp_dir().join(format!(
            "agentdesktop-inventory-test-{}.db",
            std::process::id()
        ));
        let database = Database::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect test database");
        let idp_claims = BTreeMap::from([
            ("sub".to_owned(), serde_json::json!("subject")),
            ("email".to_owned(), serde_json::json!("john@example.com")),
        ]);
        database
            .enroll_device("device", "host", "issuer", "subject", Some(&idp_claims))
            .await
            .expect("enroll device");
        let front_matter = BTreeMap::from([
            ("name", serde_json::json!("llm-research")),
            ("description", serde_json::json!("Compare gateways")),
        ]);
        database
            .replace_inventory(
                "device",
                &Inventory {
                    discoveries: vec![Discovery {
                        kind: "codex".to_owned(),
                        version: "1.0.0".to_owned(),
                        path: "/bin/codex".to_owned(),
                        mcp_servers: vec![McpServer {
                            name: "docs".to_owned(),
                            transport: "http".to_owned(),
                            url: "https://example.com/mcp".to_owned(),
                            enabled: true,
                            source: "/home/user/.codex/config.toml".to_owned(),
                            ..Default::default()
                        }],
                        skills: vec![Skill {
                            path: "/home/user/.codex/skills/llm-research/SKILL.md".to_owned(),
                            front_matter_json: serde_json::to_vec(&front_matter).unwrap(),
                        }],
                    }],
                    model_runtimes: vec![ModelRuntime {
                        kind: "ollama".to_owned(),
                        models: vec![LocalModel {
                            name: "qwen3:8b".to_owned(),
                        }],
                    }],
                },
            )
            .await
            .expect("store inventory");
        database
            .insert_telemetry(
                "device",
                &TelemetryEvent {
                    timestamp_unix_ms: 1_700_000_000_000,
                    event: Some(telemetry_event::Event::ToolUse(ToolUseEvent {
                        client_id: "claude-code".to_owned(),
                        tool_name: "Bash".to_owned(),
                        tool_use_id: "tool-1".to_owned(),
                        input_json: serde_json::to_vec(&serde_json::json!({
                            "command": "cargo test"
                        }))
                        .unwrap(),
                    })),
                },
            )
            .await
            .expect("store telemetry");

        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(device.discoveries[0].mcp_servers[0].name, "docs");
        assert_eq!(
            device.discoveries[0].skills[0].path,
            PathBuf::from("/home/user/.codex/skills/llm-research/SKILL.md")
        );
        assert_eq!(
            device.discoveries[0].skills[0].front_matter["name"],
            "llm-research"
        );
        assert_eq!(device.model_runtimes[0].kind, "ollama");
        assert_eq!(device.model_runtimes[0].models[0].name, "qwen3:8b");
        assert_eq!(device.recent_events.len(), 1);
        assert_eq!(device.recent_events[0].event_type, "tool.use");
        assert_eq!(device.recent_events[0].payload["toolName"], "Bash");
        let principal = database
            .device_principal("device")
            .await
            .expect("load device principal");
        assert_eq!(principal.idp_claims.unwrap()["email"], "john@example.com");

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    // --- Per-program configuration status -----------------------------------

    async fn empty_test_database(name: &str) -> (Database, PathBuf) {
        let (database, path) = test_database(name).await;
        database
            .delete_device("device")
            .await
            .expect("remove the helper device");
        (database, path)
    }

    async fn test_database(name: &str) -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "agentdesktop-program-status-{name}-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let database = Database::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect test database");
        database
            .enroll_device("device", "host", "issuer", "subject", None)
            .await
            .expect("enroll device");
        (database, path)
    }

    fn program_status(program: &str, state: ProgramState, detail: &str) -> ProgramStatus {
        ProgramStatus {
            program: program.to_owned(),
            state: state.into(),
            detail: detail.to_owned(),
        }
    }

    fn config_status(programs: Vec<ProgramStatus>, programs_reported: bool) -> ConfigStatus {
        ConfigStatus {
            revision: 1,
            state: ConfigState::Applied.into(),
            error: String::new(),
            programs,
            programs_reported,
        }
    }

    #[tokio::test]
    async fn program_status_rows_are_stored_replaced_and_ordered_by_position() {
        let (database, path) = test_database("stores-and-replaces").await;

        database
            .update_config_status(
                "device",
                &config_status(
                    vec![
                        program_status(
                            "vscode",
                            ProgramState::Conflict,
                            "conflict at settings.json",
                        ),
                        program_status("claude-code", ProgramState::Applied, ""),
                    ],
                    true,
                ),
            )
            .await
            .expect("store the first report");
        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(
            device
                .programs
                .iter()
                .map(|program| program.program.as_str())
                .collect::<Vec<_>>(),
            vec!["vscode", "claude-code"],
            "rows come back ordered by position, the order they were reported in"
        );

        // A program missing from the next report is deleted, not left stale.
        database
            .update_config_status(
                "device",
                &config_status(
                    vec![program_status("claude-code", ProgramState::Unchanged, "")],
                    true,
                ),
            )
            .await
            .expect("store the second report");
        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(
            device
                .programs
                .iter()
                .map(|program| program.program.as_str())
                .collect::<Vec<_>>(),
            vec!["claude-code"],
            "vscode was replaced away, not appended to"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn an_opted_out_program_reads_back_by_name() {
        let (database, path) = test_database("opted-out").await;

        database
            .update_config_status(
                "device",
                &config_status(
                    vec![program_status("codex", ProgramState::OptedOut, "")],
                    true,
                ),
            )
            .await
            .expect("store the report");
        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(device.programs[0].state, "opted_out");

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn programs_reported_is_none_without_a_status_and_the_stored_flag_once_one_exists() {
        let (database, path) = test_database("programs-reported").await;

        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(
            device.programs_reported, None,
            "no configuration status has been reported yet"
        );

        database
            .update_config_status("device", &config_status(Vec::new(), false))
            .await
            .expect("store a status without per-program reporting");
        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(
            device.programs_reported,
            Some(false),
            "an old agent's status carries no programs"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn update_config_status_rejects_out_of_bounds_program_status() {
        let (database, path) = test_database("bounds-rejected").await;

        let too_many = (0..33)
            .map(|index| program_status(&format!("program-{index}"), ProgramState::Applied, ""))
            .collect();
        assert!(
            database
                .update_config_status("device", &config_status(too_many, true))
                .await
                .is_err(),
            "more than 32 programs must be rejected"
        );

        assert!(
            database
                .update_config_status(
                    "device",
                    &config_status(
                        vec![program_status("Not-Valid", ProgramState::Applied, "")],
                        true
                    ),
                )
                .await
                .is_err(),
            "a program outside [a-z0-9-] must be rejected"
        );

        assert!(
            database
                .update_config_status(
                    "device",
                    &config_status(
                        vec![program_status(&"a".repeat(65), ProgramState::Applied, "")],
                        true
                    ),
                )
                .await
                .is_err(),
            "a program longer than 64 bytes must be rejected"
        );

        assert!(
            database
                .update_config_status(
                    "device",
                    &config_status(
                        vec![program_status(
                            "claude-code",
                            ProgramState::Applied,
                            &"x".repeat(2049)
                        )],
                        true
                    ),
                )
                .await
                .is_err(),
            "a detail longer than 2048 bytes must be rejected"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn unknown_program_state_round_trips_as_unspecified() {
        let (database, path) = test_database("unknown-state").await;

        let mut status = program_status("claude-code", ProgramState::Applied, "");
        status.state = 99; // Not a defined ProgramState value.
        database
            .update_config_status("device", &config_status(vec![status], true))
            .await
            .expect("store a status with a future/unknown state");

        let device = database
            .get_device("device")
            .await
            .expect("load device")
            .expect("device exists");
        assert_eq!(device.programs[0].state, "unspecified");

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn device_limit_allows_fifth_and_rejects_sixth() {
        let (database, path) = empty_test_database("device-limit").await;
        for index in 0..4 {
            database
                .enroll_device(
                    &format!("active-{index}"),
                    "host",
                    "issuer",
                    "subject",
                    None,
                )
                .await
                .expect("enroll active device");
        }

        let fifth = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "fifth",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: None,
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("enroll fifth device");
        assert_eq!(fifth, EnrollmentOutcome::Enrolled { linked: false });

        let sixth = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "sixth",
                hostname: "other-host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: None,
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("evaluate sixth device");
        assert_eq!(sixth, EnrollmentOutcome::LimitReached);
        assert!(!database.device_exists("sixth").await.unwrap());
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            5
        );

        let events = database
            .audit_events(&AuditFilter {
                action: Some("device.limit_rejected".to_owned()),
                ..AuditFilter::default()
            })
            .await
            .expect("read limit audit");
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].subject, "subject");

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn linked_re_enrollment_retires_the_predecessor_without_counting() {
        let (database, path) = empty_test_database("device-relink").await;
        database
            .enroll_device("old", "host", "issuer", "subject", None)
            .await
            .unwrap();

        let outcome = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "new",
                hostname: "  HOST  ",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: Some("old"),
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("link re-enrollment");
        assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: true });

        let old = database.get_device("old").await.unwrap().unwrap();
        assert_eq!(old.device.state, "retired");
        assert_eq!(old.superseded_by.as_deref(), Some("new"));
        assert!(old.retired_at.is_some());
        let new = database.get_device("new").await.unwrap().unwrap();
        assert_eq!(new.replaces_device_id.as_deref(), Some("old"));
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            1
        );

        let events = database
            .audit_events(&AuditFilter {
                action: Some("device.re_enrolled".to_owned()),
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(events.events.len(), 1);

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_replacement_at_the_limit_does_not_increase_the_active_count() {
        let (database, path) = empty_test_database("device-relink-limit").await;
        for index in 0..4 {
            database
                .enroll_device(
                    &format!("active-{index}"),
                    "host",
                    "issuer",
                    "subject",
                    None,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            4
        );

        let outcome = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "new",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: Some("active-0"),
                max_devices_per_user: 4,
                actor: "test",
            })
            .await
            .expect("replacement at the limit");
        assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: true });
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            4
        );
        assert_eq!(
            database
                .get_device("active-0")
                .await
                .unwrap()
                .unwrap()
                .device
                .state,
            "retired"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn invalid_lineage_is_rejected_with_a_stable_reason() {
        let cases = [
            (
                "subject-mismatch",
                "other-issuer",
                "host",
                "active",
                "subject_mismatch",
            ),
            (
                "hostname-mismatch",
                "issuer",
                "other-host",
                "active",
                "hostname_mismatch",
            ),
            (
                "revoked-previous",
                "issuer",
                "host",
                "revoked",
                "device_revoked",
            ),
        ];
        for (index, (name, issuer, hostname, previous_state, reason)) in
            cases.into_iter().enumerate()
        {
            let (database, path) = empty_test_database(name).await;
            database
                .enroll_device("old", "host", "issuer", "subject", None)
                .await
                .unwrap();
            if previous_state == "revoked" {
                database
                    .set_device_state("old", DeviceState::Revoked, "test")
                    .await
                    .unwrap();
            }

            let outcome = database
                .enroll_device_with_policy(EnrollmentRequest {
                    device_id: "new",
                    hostname,
                    issuer,
                    subject: "subject",
                    idp_claims: None,
                    previous_device_id: Some("old"),
                    max_devices_per_user: 5,
                    actor: "test",
                })
                .await
                .expect("evaluate lineage");
            assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: false });

            let old = database.get_device("old").await.unwrap().unwrap();
            assert_eq!(old.device.state, previous_state);
            assert_eq!(old.superseded_by, None);
            let new = database.get_device("new").await.unwrap().unwrap();
            assert_eq!(new.replaces_device_id, None);

            let events = database
                .audit_events(&AuditFilter {
                    action: Some("device.link_rejected".to_owned()),
                    ..AuditFilter::default()
                })
                .await
                .unwrap();
            assert_eq!(events.events.len(), 1, "case {index}");
            assert_eq!(events.events[0].payload["reason"], reason);

            drop(database);
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn retired_and_revoked_devices_do_not_count_against_the_limit() {
        let (database, path) = empty_test_database("device-limit-states").await;
        for index in 0..5 {
            database
                .enroll_device(
                    &format!("active-{index}"),
                    "host",
                    "issuer",
                    "subject",
                    None,
                )
                .await
                .unwrap();
        }
        database
            .set_device_state("active-0", DeviceState::Retired, "test")
            .await
            .unwrap();
        database
            .set_device_state("active-1", DeviceState::Revoked, "test")
            .await
            .unwrap();
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            3
        );

        for index in 0..2 {
            let outcome = database
                .enroll_device_with_policy(EnrollmentRequest {
                    device_id: &format!("replacement-{index}"),
                    hostname: "host",
                    issuer: "issuer",
                    subject: "subject",
                    idp_claims: None,
                    previous_device_id: None,
                    max_devices_per_user: 5,
                    actor: "test",
                })
                .await
                .unwrap();
            assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: false });
        }

        let rejected = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "overflow",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: None,
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .unwrap();
        assert_eq!(rejected, EnrollmentOutcome::LimitReached);
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            5
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn retirement_preserves_child_rows_and_is_idempotent() {
        let (database, path) = empty_test_database("device-retire-preserves").await;
        database
            .enroll_device("device", "host", "issuer", "subject", None)
            .await
            .unwrap();
        database
            .create_job(&NewJob {
                device_id: "device".to_owned(),
                idempotency_key: "idem".to_owned(),
                origin: "admin".to_owned(),
                requested_by: "subject".to_owned(),
                catalog_id: "catalog".to_owned(),
                catalog_revision: 1,
                catalog_sha256: "a".repeat(64),
                agent_id: "claude-code".to_owned(),
                version: "1.0.0".to_owned(),
                action: JobAction::Install,
                deadline_unix_seconds: 0,
            })
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO telemetry_events
                (id, device_id, timestamp_unix_ms, event_type, payload_json)
             VALUES ('event', 'device', 1, 'tool.use', '{}')",
        )
        .execute(&database.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO discoveries
                (device_id, kind, version, path, mcp_servers_json, skills_json)
             VALUES ('device', 'codex', '1.0.0', '/bin/codex', '[]', '[]')",
        )
        .execute(&database.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO usage_rollups
                (bucket_start, bucket_seconds, subject, device_id, client_id, provider,
                 model, billing, requests, errors, input_tokens, output_tokens,
                 cache_read_tokens, cache_write_tokens, cost_nanos, unpriced_requests)
             VALUES (1, 3600, 'subject', 'device', 'claude-code', 'anthropic',
                     'model', 'api', 1, 0, 1, 1, 0, 0, 0, 0)",
        )
        .execute(&database.pool)
        .await
        .unwrap();

        let first = database
            .set_device_state("device", DeviceState::Retired, "test")
            .await
            .unwrap()
            .expect("retire device");
        assert!(first.changed);
        assert_eq!(first.state, "retired");
        let second = database
            .set_device_state("device", DeviceState::Retired, "test")
            .await
            .unwrap()
            .expect("retire device again");
        assert!(!second.changed, "retirement is idempotent");
        let retired = database
            .list_devices_by_state(Some("retired"))
            .await
            .expect("filter retired devices");
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].id, "device");

        let count_jobs: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_jobs WHERE device_id = $1")
                .bind("device")
                .fetch_one(&database.pool)
                .await
                .expect("count jobs");
        let count_telemetry: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM telemetry_events WHERE device_id = $1")
                .bind("device")
                .fetch_one(&database.pool)
                .await
                .expect("count telemetry");
        let count_discoveries: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM discoveries WHERE device_id = $1")
                .bind("device")
                .fetch_one(&database.pool)
                .await
                .expect("count discoveries");
        let count_usage: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM usage_rollups WHERE device_id = $1")
                .bind("device")
                .fetch_one(&database.pool)
                .await
                .expect("count usage");
        assert_eq!(count_jobs, 1);
        assert_eq!(count_telemetry, 1);
        assert_eq!(count_discoveries, 1);
        assert_eq!(count_usage, 1);
        assert_eq!(
            database
                .audit_events(&AuditFilter {
                    action: Some("device.retired".to_owned()),
                    ..AuditFilter::default()
                })
                .await
                .unwrap()
                .events
                .len(),
            1,
            "an idempotent retire does not duplicate the audit event"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn rejected_lineage_is_not_attributed_to_the_previous_device() {
        let (database, path) = empty_test_database("device-rejection-attribution").await;
        database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "victim",
                hostname: "victim-host",
                issuer: "issuer",
                subject: "victim-subject",
                idp_claims: None,
                previous_device_id: None,
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .expect("enroll victim device");

        let outcome = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "attacker",
                hostname: "attacker-host",
                issuer: "issuer",
                subject: "attacker-subject",
                idp_claims: None,
                previous_device_id: Some("victim"),
                max_devices_per_user: 1,
                actor: "test",
            })
            .await
            .expect("rejected link enrollment");
        assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: false });

        let rejected = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "attacker-2",
                hostname: "attacker-host",
                issuer: "issuer",
                subject: "attacker-subject",
                idp_claims: None,
                previous_device_id: Some("victim"),
                max_devices_per_user: 1,
                actor: "test",
            })
            .await
            .expect("limit-rejected enrollment");
        assert_eq!(rejected, EnrollmentOutcome::LimitReached);

        let victim_history = database.device_history("victim").await.unwrap();
        let victim_events = database
            .audit_events(&AuditFilter {
                device_ids: Some(victim_history),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert!(
            !victim_events
                .events
                .iter()
                .any(|event| event.action == "device.link_rejected"
                    || event.action == "device.limit_rejected"),
            "another subject's rejection must not enter the victim's device history"
        );

        let attacker_events = database
            .audit_events(&AuditFilter {
                subject: Some("attacker-subject".to_owned()),
                action: Some("device.link_rejected".to_owned()),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(attacker_events.events.len(), 2);
        assert!(
            attacker_events
                .events
                .iter()
                .all(|event| event.device_id.is_empty())
        );

        let limit_events = database
            .audit_events(&AuditFilter {
                subject: Some("attacker-subject".to_owned()),
                action: Some("device.limit_rejected".to_owned()),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(limit_events.events.len(), 1);
        assert_eq!(limit_events.events[0].device_id, "");

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn link_previous_device_rechecks_state_at_write_time() {
        let (database, path) = empty_test_database("device-link-conditional").await;
        database
            .enroll_device("active", "host", "issuer", "subject", None)
            .await
            .unwrap();
        database
            .enroll_device("revoked", "host", "issuer", "subject", None)
            .await
            .unwrap();
        database
            .set_device_state("revoked", DeviceState::Revoked, "test")
            .await
            .unwrap();
        database
            .enroll_device("superseded", "host", "issuer", "subject", None)
            .await
            .unwrap();
        sqlx::query("UPDATE devices SET superseded_by = 'other' WHERE id = 'superseded'")
            .execute(&database.pool)
            .await
            .unwrap();
        database
            .enroll_device("quarantined", "host", "issuer", "subject", None)
            .await
            .unwrap();
        sqlx::query("UPDATE devices SET state = 'quarantined' WHERE id = 'quarantined'")
            .execute(&database.pool)
            .await
            .unwrap();

        let now = super::unix_time_seconds();
        let mut transaction = database.pool.begin().await.unwrap();
        assert_eq!(
            link_previous_device(&mut transaction, "active", "new", now)
                .await
                .unwrap(),
            None
        );
        let state: String = sqlx::query_scalar("SELECT state FROM devices WHERE id = 'active'")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
        assert_eq!(state, "retired");
        transaction.rollback().await.unwrap();

        let mut transaction = database.pool.begin().await.unwrap();
        assert_eq!(
            link_previous_device(&mut transaction, "revoked", "new", now)
                .await
                .unwrap(),
            Some("device_revoked")
        );
        transaction.rollback().await.unwrap();
        assert_eq!(
            database
                .get_device("revoked")
                .await
                .unwrap()
                .unwrap()
                .device
                .state,
            "revoked",
            "a concurrent revoke must not be overwritten by a link"
        );

        let mut transaction = database.pool.begin().await.unwrap();
        assert_eq!(
            link_previous_device(&mut transaction, "superseded", "new", now)
                .await
                .unwrap(),
            Some("already_superseded")
        );
        transaction.rollback().await.unwrap();

        let mut transaction = database.pool.begin().await.unwrap();
        assert_eq!(
            link_previous_device(&mut transaction, "quarantined", "new", now)
                .await
                .unwrap(),
            Some("state_unknown")
        );
        transaction.rollback().await.unwrap();

        let mut transaction = database.pool.begin().await.unwrap();
        assert_eq!(
            link_previous_device(&mut transaction, "missing", "new", now)
                .await
                .unwrap(),
            Some("unknown_device")
        );
        transaction.rollback().await.unwrap();

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn link_requires_exactly_active_or_retired() {
        let (database, path) = empty_test_database("device-link-states").await;
        database
            .enroll_device("quarantined", "host", "issuer", "subject", None)
            .await
            .unwrap();
        sqlx::query("UPDATE devices SET state = 'quarantined' WHERE id = 'quarantined'")
            .execute(&database.pool)
            .await
            .unwrap();

        let outcome = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "new",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: Some("quarantined"),
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .unwrap();
        assert_eq!(outcome, EnrollmentOutcome::Enrolled { linked: false });
        assert_eq!(
            database
                .get_device("quarantined")
                .await
                .unwrap()
                .unwrap()
                .device
                .state,
            "quarantined"
        );
        let events = database
            .audit_events(&AuditFilter {
                action: Some("device.link_rejected".to_owned()),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(events.events[0].payload["reason"], "state_unknown");

        let (database, path_two) = empty_test_database("device-link-superseded").await;
        database
            .enroll_device("old", "host", "issuer", "subject", None)
            .await
            .unwrap();
        let linked = database
            .enroll_device_with_policy(EnrollmentRequest {
                device_id: "middle",
                hostname: "host",
                issuer: "issuer",
                subject: "subject",
                idp_claims: None,
                previous_device_id: Some("old"),
                max_devices_per_user: 5,
                actor: "test",
            })
            .await
            .unwrap();
        assert_eq!(linked, EnrollmentOutcome::Enrolled { linked: true });

        let replay = database
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
            .unwrap();
        assert_eq!(replay, EnrollmentOutcome::Enrolled { linked: false });
        let events = database
            .audit_events(&AuditFilter {
                action: Some("device.link_rejected".to_owned()),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(events.events[0].payload["reason"], "already_superseded");

        drop(database);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path_two);
    }

    #[tokio::test]
    async fn concurrent_retires_write_one_audit_event() {
        let (database, path) = empty_test_database("device-concurrent-retire").await;
        database
            .enroll_device("device", "host", "issuer", "subject", None)
            .await
            .unwrap();

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let database = database.clone();
            tasks.spawn(async move {
                database
                    .set_device_state("device", DeviceState::Retired, "test")
                    .await
            });
        }
        let mut changed = 0;
        let mut unchanged = 0;
        while let Some(result) = tasks.join_next().await {
            let transition = result
                .expect("transition task")
                .expect("transition")
                .expect("device exists");
            if transition.changed {
                changed += 1;
            } else {
                unchanged += 1;
            }
        }
        assert_eq!(changed, 1);
        assert_eq!(unchanged, 1);

        let events = database
            .audit_events(&AuditFilter {
                action: Some("device.retired".to_owned()),
                limit: 50,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(events.events.len(), 1);

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn audit_history_follows_the_replacement_chain_and_paginates() {
        let (database, path) = empty_test_database("device-audit-chain").await;
        for (device_id, previous, hostname) in [
            ("old", None, "host"),
            ("middle", Some("old"), "host"),
            ("new", Some("middle"), "host"),
        ] {
            let outcome = database
                .enroll_device_with_policy(EnrollmentRequest {
                    device_id,
                    hostname,
                    issuer: "issuer",
                    subject: "subject",
                    idp_claims: None,
                    previous_device_id: previous,
                    max_devices_per_user: 5,
                    actor: "test",
                })
                .await
                .unwrap();
            assert_eq!(
                outcome,
                EnrollmentOutcome::Enrolled {
                    linked: previous.is_some()
                }
            );
        }

        let history = database.device_history("new").await.unwrap();
        assert_eq!(history.len(), 3);
        assert!(history.contains(&"new".to_owned()));
        assert!(history.contains(&"middle".to_owned()));
        assert!(history.contains(&"old".to_owned()));

        let first_page = database
            .audit_events(&AuditFilter {
                device_ids: Some(history.clone()),
                limit: 2,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(first_page.events.len(), 2);
        let cursor = first_page.next_cursor.expect("first page has a cursor");

        let second_page = database
            .audit_events(&AuditFilter {
                device_ids: Some(history),
                before: Some(cursor),
                limit: 10,
                ..AuditFilter::default()
            })
            .await
            .unwrap();
        assert_eq!(second_page.events.len(), 1);
        assert_eq!(first_page.events.len() + second_page.events.len(), 3);
        assert!(
            first_page
                .events
                .iter()
                .all(|event| !event.payload.to_string().contains("secret"))
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn migration_admits_existing_devices_and_audit_rows() {
        sqlx::any::install_default_drivers();
        let path = std::env::temp_dir().join(format!(
            "agentdesktop-migration-test-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect migration database");
        sqlx::raw_sql(include_str!("../migrations/0001_controller.sql"))
            .execute(&pool)
            .await
            .expect("create device schema");
        sqlx::raw_sql(include_str!("../migrations/0007_audit_events.sql"))
            .execute(&pool)
            .await
            .expect("create audit schema");
        sqlx::query(
            "INSERT INTO devices (id, hostname, created_at, enrolled_by_issuer, enrolled_by_subject)
             VALUES ('legacy', 'host', 1, 'issuer', 'subject')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO audit_events (id, actor, action, device_id, payload_json, created_at)
             VALUES ('event', 'actor', 'job.create', 'legacy', '{}', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/0008_device_lifecycle.sql"))
            .execute(&pool)
            .await
            .expect("apply lifecycle migration");

        let (state, replaces): (String, Option<String>) =
            sqlx::query_as("SELECT state, replaces_device_id FROM devices WHERE id = 'legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "active");
        assert_eq!(replaces, None);
        let (subject, device_id): (String, String) =
            sqlx::query_as("SELECT subject, device_id FROM audit_events WHERE id = 'event'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(subject, "subject");
        assert_eq!(device_id, "legacy");
        let index_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name IN
                ('devices_subject_state_idx', 'devices_superseded_by_idx',
                 'audit_events_subject_time_idx', 'audit_events_action_time_idx')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(index_count, 4);

        drop(pool);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn concurrent_enrollments_cannot_exceed_the_limit() {
        let (database, path) = empty_test_database("device-limit-concurrent").await;
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..6 {
            let database = database.clone();
            tasks.spawn(async move {
                database
                    .enroll_device_with_policy(EnrollmentRequest {
                        device_id: &format!("concurrent-{index}"),
                        hostname: &format!("host-{index}"),
                        issuer: "issuer",
                        subject: "subject",
                        idp_claims: None,
                        previous_device_id: None,
                        max_devices_per_user: 5,
                        actor: "test",
                    })
                    .await
            });
        }
        let mut enrolled = 0;
        let mut rejected = 0;
        while let Some(result) = tasks.join_next().await {
            match result.expect("enrollment task").expect("enrollment") {
                EnrollmentOutcome::Enrolled { .. } => enrolled += 1,
                EnrollmentOutcome::LimitReached => rejected += 1,
            }
        }
        assert_eq!(enrolled, 5);
        assert_eq!(rejected, 1);
        assert_eq!(
            database
                .active_device_count("issuer", "subject")
                .await
                .unwrap(),
            5
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn program_status_rows_cascade_on_device_delete() {
        let (database, path) = test_database("cascade-delete").await;
        database
            .update_config_status(
                "device",
                &config_status(
                    vec![program_status("claude-code", ProgramState::Applied, "")],
                    true,
                ),
            )
            .await
            .expect("store a report");

        let count_rows = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM device_program_status WHERE device_id = $1",
            )
            .bind("device")
            .fetch_one(&database.pool)
            .await
            .expect("count device_program_status rows")
        };
        assert_eq!(count_rows().await, 1);

        assert!(
            database
                .delete_device("device")
                .await
                .expect("delete device")
        );
        assert_eq!(
            count_rows().await,
            0,
            "device_program_status cascades on device delete"
        );

        drop(database);
        let _ = std::fs::remove_file(path);
    }
}
