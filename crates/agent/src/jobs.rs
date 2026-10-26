//! Daemon-side job executor for catalog-driven install/uninstall.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use agentdesktop_core::catalog::{CatalogPublicKey, CatalogSnapshot};
use agentdesktop_proto::fleet::{AgentJob, JobAction, JobState, JobStatus};
use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

const MAX_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;

fn valid_job_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Download an artifact over HTTPS with a bounded size and timeout.
pub async fn download_artifact(url: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(url.starts_with("https://"), "artifact URL must use HTTPS");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARTIFACT_BYTES as u64)
    {
        bail!("artifact exceeds the size limit");
    }
    let bytes = response.bytes().await?;
    ensure!(
        bytes.len() <= MAX_ARTIFACT_BYTES,
        "artifact exceeds the size limit"
    );
    Ok(bytes.to_vec())
}

use crate::{installer::Installer, manifest::verify_artifact, secure_fs};

/// Executes one job at a time and persists terminal status for idempotency.
#[derive(Clone)]
pub struct JobExecutor {
    state_dir: PathBuf,
    active: Arc<Mutex<()>>,
}

impl JobExecutor {
    pub fn new(state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            active: Arc::default(),
        }
    }

    /// Execute `job`. `artifact` is the downloaded bytes for install/update;
    /// uninstall does not need it.
    pub fn execute(
        &self,
        job: &AgentJob,
        snapshot: &CatalogSnapshot,
        keys: &[CatalogPublicKey],
        artifact: Option<&[u8]>,
        installer: &dyn Installer,
    ) -> JobStatus {
        if !valid_job_identifier(&job.job_id) {
            return failed(job, "invalid_job", "job id contains unsafe characters");
        }
        if let Some(status) = self.load_status(&job.job_id)
            && is_terminal_status(status.state)
        {
            return status;
        }
        let _guard = match self.active.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return failed(job, "busy", "another job is already running");
            }
        };
        let status = self.execute_locked(job, snapshot, keys, artifact, installer);
        self.store_status(&status);
        status
    }

    fn execute_locked(
        &self,
        job: &AgentJob,
        snapshot: &CatalogSnapshot,
        keys: &[CatalogPublicKey],
        artifact: Option<&[u8]>,
        installer: &dyn Installer,
    ) -> JobStatus {
        if job.job_id.is_empty() {
            return failed(job, "invalid_job", "job id is empty");
        }
        if !valid_job_identifier(&job.agent_id) {
            return failed(job, "invalid_job", "agent id contains unsafe characters");
        }
        if job.catalog_id != snapshot.catalog.catalog_id {
            return failed(
                job,
                "catalog_changed",
                "job does not match the verified catalog id",
            );
        }
        if snapshot.catalog.is_expired(now_unix_seconds()) {
            return failed(job, "catalog_expired", "the verified catalog is expired");
        }
        if job.catalog_revision != snapshot.catalog.revision
            || job.catalog_sha256 != snapshot.sha256
        {
            return failed(
                job,
                "catalog_changed",
                "job does not match the verified catalog revision",
            );
        }
        if job.deadline_unix_seconds > 0 && now_unix_seconds() >= job.deadline_unix_seconds {
            return failed(job, "timeout", "job deadline has passed");
        }
        let action = JobAction::try_from(job.action).unwrap_or(JobAction::Unspecified);
        let Some(entry) = snapshot
            .catalog
            .entries
            .iter()
            .find(|entry| entry.id == job.agent_id)
        else {
            return failed(job, "unknown_agent", "agent is not in the catalog");
        };
        match action {
            JobAction::Install | JobAction::Update => {
                let Some(artifact) = artifact else {
                    return failed(job, "download_failed", "artifact bytes are missing");
                };
                let verified = match verify_artifact(snapshot, entry, &job.version, artifact, keys)
                {
                    Ok(version) => version,
                    Err(error) => {
                        let (code, message) = classify(&error);
                        return failed(job, code, &message);
                    }
                };
                match installer.install(&job.job_id, entry, &verified.version, artifact) {
                    Ok(_) => succeeded(job),
                    Err(error) => {
                        let (code, message) = classify(&error);
                        failed(job, code, &message)
                    }
                }
            }
            JobAction::Uninstall => {
                let Some(receipt) = installer.load_receipt(&job.agent_id) else {
                    return failed(job, "receipt_conflict", "no managed install receipt");
                };
                match installer.uninstall(&receipt, false) {
                    Ok(()) => succeeded(job),
                    Err(error) => {
                        let (code, message) = classify(&error);
                        failed(job, code, &message)
                    }
                }
            }
            JobAction::Unspecified => failed(job, "invalid_job", "job action is unspecified"),
        }
    }

    fn status_path(&self, job_id: &str) -> PathBuf {
        self.state_dir.join("jobs").join(format!("{job_id}.json"))
    }

    fn load_status(&self, job_id: &str) -> Option<JobStatus> {
        let bytes = std::fs::read(self.status_path(job_id)).ok()?;
        let stored: StoredStatus = serde_json::from_slice(&bytes).ok()?;
        Some(stored.into())
    }

    fn store_status(&self, status: &JobStatus) {
        if !valid_job_identifier(&status.job_id) {
            tracing::warn!(job_id = %status.job_id, "not persisting a job with an unsafe id");
            return;
        }
        let path = self.status_path(&status.job_id);
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        if let Err(error) = secure_fs::ensure_private_dir(directory) {
            tracing::warn!(error = %format!("{error:#}"), "could not create job state directory");
            return;
        }
        let stored = StoredStatus::from(status);
        match serde_json::to_vec(&stored) {
            Ok(bytes) => {
                if let Err(error) = secure_fs::atomic_write(&path, &bytes, 0o600) {
                    tracing::warn!(error = %format!("{error:#}"), "could not persist job status");
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not encode job status");
            }
        }
    }

    /// All persisted job statuses, newest id last. Used by the local API.
    pub fn list_statuses(&self) -> Vec<JobStatus> {
        let directory = self.state_dir.join("jobs");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut statuses: Vec<JobStatus> = entries
            .flatten()
            .filter_map(|entry| {
                let bytes = std::fs::read(entry.path()).ok()?;
                let stored: StoredStatus = serde_json::from_slice(&bytes).ok()?;
                Some(stored.into())
            })
            .collect();
        statuses.sort_by(|left, right| left.job_id.cmp(&right.job_id));
        statuses
    }
}

#[derive(Serialize, Deserialize)]
struct StoredStatus {
    job_id: String,
    attempt: u32,
    state: i32,
    progress_percent: u32,
    error_code: String,
    error_message: String,
    diagnostics_json: String,
}

impl From<&JobStatus> for StoredStatus {
    fn from(status: &JobStatus) -> Self {
        Self {
            job_id: status.job_id.clone(),
            attempt: status.attempt,
            state: status.state,
            progress_percent: status.progress_percent,
            error_code: status.error_code.clone(),
            error_message: status.error_message.clone(),
            diagnostics_json: status.diagnostics_json.clone(),
        }
    }
}

impl From<StoredStatus> for JobStatus {
    fn from(stored: StoredStatus) -> Self {
        Self {
            job_id: stored.job_id,
            attempt: stored.attempt,
            state: stored.state,
            progress_percent: stored.progress_percent,
            error_code: stored.error_code,
            error_message: stored.error_message,
            diagnostics_json: stored.diagnostics_json,
        }
    }
}

fn succeeded(job: &AgentJob) -> JobStatus {
    JobStatus {
        job_id: job.job_id.clone(),
        attempt: job.attempt,
        state: JobState::Succeeded as i32,
        progress_percent: 100,
        error_code: String::new(),
        error_message: String::new(),
        diagnostics_json: String::new(),
    }
}

fn failed(job: &AgentJob, code: &str, message: &str) -> JobStatus {
    JobStatus {
        job_id: job.job_id.clone(),
        attempt: job.attempt,
        state: JobState::Failed as i32,
        progress_percent: 0,
        error_code: code.to_owned(),
        error_message: message.to_owned(),
        diagnostics_json: diagnostics(),
    }
}

fn diagnostics() -> String {
    serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
    .to_string()
}

fn classify(error: &anyhow::Error) -> (&'static str, String) {
    let message = format!("{error:#}");
    for code in [
        "checksum_mismatch",
        "signature_invalid",
        "unknown_agent",
        "catalog_changed",
        "receipt_conflict",
        "permission_denied",
        "disk_full",
        "extract_failed",
    ] {
        if message.contains(code) {
            return (code, message);
        }
    }
    ("install_failed", message)
}

fn is_terminal_status(state: i32) -> bool {
    matches!(
        JobState::try_from(state),
        Ok(JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Expired)
    )
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agentdesktop_core::catalog::{
        Catalog, CatalogEntry, CatalogPublicKey, CatalogSnapshot, CatalogVersion, DetectionRule,
        InstallKind, canonical_json, sha256_hex,
    };
    use agentdesktop_proto::fleet::{AgentJob, JobAction, JobState};
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
    use base64::Engine as _;
    use tempfile::tempdir;

    use super::JobExecutor;
    use crate::installer::{Installer, Receipt};

    #[derive(Default)]
    struct FakeInstaller {
        receipts: std::sync::Mutex<BTreeMap<String, Receipt>>,
        installed: std::sync::Mutex<Vec<String>>,
    }

    impl Installer for FakeInstaller {
        fn install(
            &self,
            _job_id: &str,
            entry: &CatalogEntry,
            version: &str,
            artifact: &[u8],
        ) -> anyhow::Result<Receipt> {
            self.installed
                .lock()
                .unwrap()
                .push(format!("{}@{version}", entry.id));
            let receipt = Receipt {
                agent_id: entry.id.clone(),
                version: version.to_owned(),
                binary_path: std::path::PathBuf::from("/tmp/binary"),
                launcher_path: std::path::PathBuf::from("/tmp/launcher"),
                artifact_sha256: sha256_hex(artifact),
                binary_sha256: sha256_hex(artifact),
                installed_at_unix_seconds: 1,
            };
            self.receipts
                .lock()
                .unwrap()
                .insert(entry.id.clone(), receipt.clone());
            Ok(receipt)
        }

        fn uninstall(&self, receipt: &Receipt, _allow_modified: bool) -> anyhow::Result<()> {
            self.receipts.lock().unwrap().remove(&receipt.agent_id);
            Ok(())
        }

        fn load_receipt(&self, agent_id: &str) -> Option<Receipt> {
            self.receipts.lock().unwrap().get(agent_id).cloned()
        }
    }

    fn fixture(artifact: &[u8]) -> (CatalogSnapshot, Vec<CatalogPublicKey>) {
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        let public_key = key_pair.public_key().as_ref().to_vec();
        let signature = key_pair.sign(artifact);
        let entry = CatalogEntry {
            id: "claude-code".to_owned(),
            display_name: "Claude Code".to_owned(),
            program_key: "claudeCode".to_owned(),
            platforms: vec!["macos".to_owned()],
            architectures: vec!["aarch64".to_owned()],
            install_kind: InstallKind::User,
            versions: vec![CatalogVersion {
                version: "1.0.0".to_owned(),
                artifact_url: "https://example.com/a.tar.gz".to_owned(),
                sha256: sha256_hex(artifact),
                signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
            }],
            detection: DetectionRule { paths: Vec::new() },
        };
        let catalog = Catalog {
            catalog_id: "test".to_owned(),
            revision: 1,
            created_at: 1,
            expires_at: u64::MAX,
            entries: vec![entry],
        };
        let canonical_bytes = canonical_json(&catalog).unwrap();
        let snapshot = CatalogSnapshot {
            catalog,
            key_id: "test-key".to_owned(),
            sha256: sha256_hex(&canonical_bytes),
            canonical_bytes,
            signed_catalog_json: Vec::new(),
        };
        (
            snapshot,
            vec![CatalogPublicKey {
                key_id: "test-key".to_owned(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
            }],
        )
    }

    fn job(snapshot: &CatalogSnapshot) -> AgentJob {
        AgentJob {
            job_id: "job-1".to_owned(),
            attempt: 1,
            idempotency_key: "idem-1".to_owned(),
            origin: "local".to_owned(),
            requested_by: "tester".to_owned(),
            catalog_id: snapshot.catalog.catalog_id.clone(),
            catalog_revision: snapshot.catalog.revision,
            catalog_sha256: snapshot.sha256.clone(),
            agent_id: "claude-code".to_owned(),
            version: "1.0.0".to_owned(),
            action: JobAction::Install as i32,
            deadline_unix_seconds: 0,
        }
    }

    #[test]
    fn a_catalog_binding_mismatch_fails_before_install() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let mut job = job(&snapshot);
        job.catalog_revision = 2;
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job,
            &snapshot,
            &keys,
            Some(b"artifact"),
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "catalog_changed");
        assert!(!status.diagnostics_json.is_empty());
    }

    #[test]
    fn a_failed_verification_does_not_call_the_installer() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let installer = FakeInstaller::default();
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job(&snapshot),
            &snapshot,
            &keys,
            Some(b"tampered"),
            &installer,
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert!(installer.installed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_terminal_status_is_replayed_for_a_duplicate_job() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let installer = FakeInstaller::default();
        let executor = JobExecutor::new(directory.path().to_owned());
        let first = executor.execute(
            &job(&snapshot),
            &snapshot,
            &keys,
            Some(b"artifact"),
            &installer,
        );
        assert_eq!(first.state, JobState::Succeeded as i32);
        let second = executor.execute(
            &job(&snapshot),
            &snapshot,
            &keys,
            Some(b"artifact"),
            &installer,
        );
        assert_eq!(second.state, first.state);
        assert_eq!(installer.installed.lock().unwrap().len(), 1);
    }

    #[test]
    fn uninstall_without_a_receipt_is_refused() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let mut job = job(&snapshot);
        job.action = JobAction::Uninstall as i32;
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job,
            &snapshot,
            &keys,
            None,
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "receipt_conflict");
    }

    #[tokio::test]
    async fn artifact_download_requires_https() {
        let error = super::download_artifact("http://example.com/a.tar.gz")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTPS"));
    }

    #[test]
    fn an_unsafe_job_id_is_rejected_without_persisting() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let mut job = job(&snapshot);
        job.job_id = "../../escape".to_owned();
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job,
            &snapshot,
            &keys,
            Some(b"artifact"),
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "invalid_job");
        assert!(
            !directory
                .path()
                .parent()
                .unwrap()
                .join("escape.json")
                .exists()
        );
    }

    #[test]
    fn an_unsafe_agent_id_is_rejected() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let mut job = job(&snapshot);
        job.agent_id = "../escape".to_owned();
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job,
            &snapshot,
            &keys,
            Some(b"artifact"),
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "invalid_job");
    }

    #[test]
    fn an_expired_catalog_cannot_authorize_a_job() {
        let directory = tempdir().unwrap();
        let (mut snapshot, keys) = fixture(b"artifact");
        snapshot.catalog.expires_at = 1;
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job(&snapshot),
            &snapshot,
            &keys,
            Some(b"artifact"),
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "catalog_expired");
    }

    #[test]
    fn a_catalog_id_mismatch_is_rejected() {
        let directory = tempdir().unwrap();
        let (snapshot, keys) = fixture(b"artifact");
        let mut job = job(&snapshot);
        job.catalog_id = "other".to_owned();
        let status = JobExecutor::new(directory.path().to_owned()).execute(
            &job,
            &snapshot,
            &keys,
            Some(b"artifact"),
            &FakeInstaller::default(),
        );
        assert_eq!(status.state, JobState::Failed as i32);
        assert_eq!(status.error_code, "catalog_changed");
    }
}
