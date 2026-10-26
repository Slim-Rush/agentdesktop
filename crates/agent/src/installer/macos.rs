//! User-level installation on macOS. No elevation, no system paths.

use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use agentdesktop_core::catalog::CatalogEntry;
use anyhow::{Context, bail, ensure};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;

use super::{Installer, Receipt};
use crate::secure_fs;

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
struct PendingInstall {
    job_id: String,
    version: String,
    staging: PathBuf,
    receipt: Receipt,
}

/// Installs a `.tar.gz` containing one command-line binary into user-owned
/// locations under `home` and `state_dir`.
pub struct MacosInstaller {
    home: PathBuf,
    state_dir: PathBuf,
}

impl MacosInstaller {
    pub fn new(home: PathBuf, state_dir: PathBuf) -> Self {
        Self { home, state_dir }
    }

    pub fn receipt_path(&self, agent_id: &str) -> PathBuf {
        self.agent_dir(agent_id).join("receipt.json")
    }

    pub fn load_receipt(&self, agent_id: &str) -> Option<Receipt> {
        let bytes = fs::read(self.receipt_path(agent_id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Install or replace one version. Reinstalling an intact identical
    /// version is a no-op.
    pub fn install(
        &self,
        job_id: &str,
        entry: &CatalogEntry,
        version: &str,
        artifact: &[u8],
    ) -> anyhow::Result<Receipt> {
        ensure!(
            entry.install_kind == agentdesktop_core::catalog::InstallKind::User,
            "only user-level installs are supported"
        );
        let artifact_sha = sha256_hex(artifact);
        if let Some(receipt) = self.load_receipt(&entry.id)
            && receipt.version == version
            && receipt.artifact_sha256 == artifact_sha
            && receipt.binary_path.is_file()
        {
            return Ok(receipt);
        }

        let agent_dir = self.agent_dir(&entry.id);
        secure_fs::ensure_private_dir(&agent_dir)?;
        let staging = agent_dir.join(format!(".staging-{job_id}"));
        if staging.exists() {
            fs::remove_dir_all(&staging).ok();
        }
        secure_fs::ensure_private_dir(&staging)?;
        let binary_name = extract_archive(artifact, &staging)?;

        let version_dir = agent_dir.join(version);
        let binary_path = version_dir.join(&binary_name);
        let launcher_dir = self.home.join(".local/bin");
        let launcher_path = launcher_dir.join(&entry.id);

        let mut receipt = Receipt {
            agent_id: entry.id.clone(),
            version: version.to_owned(),
            binary_path: binary_path.clone(),
            launcher_path: launcher_path.clone(),
            artifact_sha256: artifact_sha,
            binary_sha256: String::new(),
            installed_at_unix_seconds: now_unix_seconds(),
        };
        let pending = PendingInstall {
            job_id: job_id.to_owned(),
            version: version.to_owned(),
            staging: staging.clone(),
            receipt: receipt.clone(),
        };
        secure_fs::atomic_write(
            &agent_dir.join("pending.json"),
            &serde_json::to_vec(&pending)?,
            0o600,
        )?;

        // Publish the payload, then the launcher, then the receipt (the commit
        // point). A failure before the receipt leaves the previous receipt and
        // launcher in place.
        if version_dir.exists() {
            let old = agent_dir.join(format!(".old-{job_id}"));
            if old.exists() {
                fs::remove_dir_all(&old).ok();
            }
            fs::rename(&version_dir, &old)?;
        }
        fs::rename(&staging, &version_dir)
            .with_context(|| format!("publish payload to {}", version_dir.display()))?;
        fs::create_dir_all(&launcher_dir)?;
        if launcher_path.symlink_metadata().is_ok() {
            fs::remove_file(&launcher_path).ok();
        }
        std::os::unix::fs::symlink(&binary_path, &launcher_path)
            .with_context(|| format!("link launcher at {}", launcher_path.display()))?;
        receipt.binary_sha256 = sha256_file(&binary_path)?;
        secure_fs::atomic_write(
            &self.receipt_path(&entry.id),
            &serde_json::to_vec(&receipt)?,
            0o600,
        )?;
        fs::remove_file(agent_dir.join("pending.json")).ok();
        Ok(receipt)
    }

    /// Remove the launcher and payload only when the current objects still
    /// match the receipt. `allow_modified` is an explicit user override.
    pub fn uninstall(&self, receipt: &Receipt, allow_modified: bool) -> anyhow::Result<()> {
        self.ensure_managed_path(&receipt.agent_id, &receipt.binary_path)?;
        self.ensure_managed_path(&receipt.agent_id, &receipt.launcher_path)?;

        if receipt.binary_path.is_file() {
            let digest = sha256_file(&receipt.binary_path)?;
            if digest != receipt.binary_sha256 && !allow_modified {
                bail!("receipt_conflict: installed binary was modified");
            }
        }
        if let Ok(metadata) = receipt.launcher_path.symlink_metadata() {
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&receipt.launcher_path)?;
                if target != receipt.binary_path && !allow_modified {
                    bail!("receipt_conflict: launcher points elsewhere");
                }
                fs::remove_file(&receipt.launcher_path)?;
            } else if !allow_modified {
                bail!("receipt_conflict: launcher is not a symlink");
            } else {
                fs::remove_file(&receipt.launcher_path)?;
            }
        }
        if let Some(version_dir) = receipt.binary_path.parent()
            && version_dir.starts_with(self.agent_dir(&receipt.agent_id))
        {
            fs::remove_dir_all(version_dir).ok();
        }
        fs::remove_file(self.receipt_path(&receipt.agent_id)).ok();
        Ok(())
    }

    fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.state_dir.join("installs").join(agent_id)
    }

    fn ensure_managed_path(&self, agent_id: &str, path: &Path) -> anyhow::Result<()> {
        let agent_dir = self.agent_dir(agent_id);
        let launcher_dir = self.home.join(".local/bin");
        ensure!(
            path.starts_with(&agent_dir) || path.starts_with(&launcher_dir),
            "refusing to remove a path outside the managed roots: {}",
            path.display()
        );
        Ok(())
    }

    /// Recover installs interrupted between the pending journal write and the
    /// committed receipt. A committed receipt means roll forward cleanup; a
    /// missing/mismatched receipt means roll back to the previous version.
    pub fn recover(&self) -> anyhow::Result<()> {
        let installs = self.state_dir.join("installs");
        let Ok(entries) = fs::read_dir(&installs) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let agent_dir = entry.path();
            if !agent_dir.is_dir() {
                continue;
            }
            let pending_path = agent_dir.join("pending.json");
            let Ok(bytes) = fs::read(&pending_path) else {
                continue;
            };
            let pending: PendingInstall = match serde_json::from_slice(&bytes) {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::warn!(
                        path = %pending_path.display(),
                        error = %error,
                        "discarding a corrupt pending install journal"
                    );
                    fs::remove_file(&pending_path).ok();
                    continue;
                }
            };
            let committed = self
                .load_receipt(&pending.receipt.agent_id)
                .is_some_and(|receipt| receipt.version == pending.version);
            if committed {
                fs::remove_file(&pending_path).ok();
                fs::remove_dir_all(&pending.staging).ok();
                remove_old_dirs(&agent_dir);
                continue;
            }
            fs::remove_dir_all(&pending.staging).ok();
            let version_dir = agent_dir.join(&pending.version);
            if version_dir.exists() {
                fs::remove_dir_all(&version_dir).ok();
            }
            restore_old_dir(&agent_dir, &version_dir);
            fs::remove_file(&pending_path).ok();
        }
        Ok(())
    }
}

fn remove_old_dirs(agent_dir: &Path) {
    let Ok(entries) = fs::read_dir(agent_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(".old-") {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
}

fn restore_old_dir(agent_dir: &Path, version_dir: &Path) {
    let Ok(entries) = fs::read_dir(agent_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(".old-") {
            fs::rename(entry.path(), version_dir).ok();
            break;
        }
    }
}

impl Installer for MacosInstaller {
    fn install(
        &self,
        job_id: &str,
        entry: &CatalogEntry,
        version: &str,
        artifact: &[u8],
    ) -> anyhow::Result<Receipt> {
        MacosInstaller::install(self, job_id, entry, version, artifact)
    }

    fn uninstall(&self, receipt: &Receipt, allow_modified: bool) -> anyhow::Result<()> {
        MacosInstaller::uninstall(self, receipt, allow_modified)
    }

    fn load_receipt(&self, agent_id: &str) -> Option<Receipt> {
        MacosInstaller::load_receipt(self, agent_id)
    }

    fn recover(&self) -> anyhow::Result<()> {
        MacosInstaller::recover(self)
    }

    fn uninstall_plan(&self, receipt: &Receipt) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if receipt.launcher_path.symlink_metadata().is_ok() {
            paths.push(receipt.launcher_path.clone());
        }
        if let Some(version_dir) = receipt.binary_path.parent()
            && version_dir.exists()
        {
            paths.push(version_dir.to_path_buf());
        }
        let receipt_path = self.receipt_path(&receipt.agent_id);
        if receipt_path.exists() {
            paths.push(receipt_path);
        }
        paths
    }
}

fn extract_archive(artifact: &[u8], staging: &Path) -> anyhow::Result<String> {
    extract_archive_with_limit(artifact, staging, MAX_UNCOMPRESSED_BYTES)
}

fn extract_archive_with_limit(
    artifact: &[u8],
    staging: &Path,
    limit: u64,
) -> anyhow::Result<String> {
    let decoder = GzDecoder::new(artifact);
    let mut archive = Archive::new(decoder);
    let mut files = Vec::new();
    let mut total = 0_u64;
    for (index, entry) in archive.entries()?.enumerate() {
        ensure!(index < MAX_ARCHIVE_ENTRIES, "archive has too many entries");
        let mut entry = entry?;
        let size = entry.header().size()?;
        total = total.checked_add(size).context("archive size overflow")?;
        ensure!(
            total <= limit,
            "archive exceeds the uncompressed size limit"
        );
        let path = entry.path()?.into_owned();
        ensure!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_))),
            "archive entry escapes the staging directory: {}",
            path.display()
        );
        let file_type = entry.header().entry_type();
        ensure!(file_type.is_file(), "only regular files are supported");
        let target = staging.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        files.push(path);
    }
    ensure!(!files.is_empty(), "archive contains no files");
    Ok(files
        .iter()
        .find(|path| path.extension().is_none())
        .unwrap_or(&files[0])
        .to_string_lossy()
        .into_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    Ok(output)
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use agentdesktop_core::catalog::{CatalogEntry, CatalogVersion, DetectionRule, InstallKind};
    use flate2::{Compression, write::GzEncoder};
    use tempfile::tempdir;

    use super::{MacosInstaller, PendingInstall, Receipt};

    fn entry() -> CatalogEntry {
        CatalogEntry {
            id: "claude-code".to_owned(),
            display_name: "Claude Code".to_owned(),
            program_key: "claudeCode".to_owned(),
            platforms: vec!["macos".to_owned()],
            architectures: vec!["aarch64".to_owned()],
            install_kind: InstallKind::User,
            versions: vec![CatalogVersion {
                version: "1.0.0".to_owned(),
                artifact_url: "https://example.com/a.tar.gz".to_owned(),
                sha256: "a".repeat(64),
                signature: "a".to_owned(),
            }],
            detection: DetectionRule { paths: Vec::new() },
        }
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            for (path, contents) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, path, *contents).unwrap();
            }
            builder.finish().unwrap();
        }
        encoder.finish().unwrap()
    }

    #[test]
    fn installs_and_uninstalls_a_user_binary() {
        let home = tempdir().unwrap();
        let state = tempdir().unwrap();
        let installer = MacosInstaller::new(home.path().to_owned(), state.path().to_owned());
        let artifact = archive(&[("bin/claude", b"#!/bin/sh\necho hi\n")]);
        let receipt = installer
            .install("job-1", &entry(), "1.0.0", &artifact)
            .unwrap();
        assert!(receipt.binary_path.is_file());
        assert_eq!(
            std::fs::read_link(&receipt.launcher_path).unwrap(),
            receipt.binary_path
        );
        installer.uninstall(&receipt, false).unwrap();
        assert!(!receipt.binary_path.exists());
        assert!(!receipt.launcher_path.exists());
    }

    fn malicious_archive() -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let mut header = tar::Header::new_gnu();
            {
                let gnu = header.as_gnu_mut().unwrap();
                let name = b"../escape";
                gnu.name[..name.len()].copy_from_slice(name);
            }
            header.set_size(1);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &b"x"[..]).unwrap();
            builder.finish().unwrap();
        }
        encoder.finish().unwrap()
    }

    #[test]
    fn a_path_traversal_archive_is_rejected() {
        let home = tempdir().unwrap();
        let state = tempdir().unwrap();
        let installer = MacosInstaller::new(home.path().to_owned(), state.path().to_owned());
        let error = installer
            .install("job-1", &entry(), "1.0.0", &malicious_archive())
            .unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn a_modified_binary_refuses_uninstall_without_override() {
        let home = tempdir().unwrap();
        let state = tempdir().unwrap();
        let installer = MacosInstaller::new(home.path().to_owned(), state.path().to_owned());
        let artifact = archive(&[("claude", b"original")]);
        let receipt = installer
            .install("job-1", &entry(), "1.0.0", &artifact)
            .unwrap();
        std::fs::write(&receipt.binary_path, b"modified").unwrap();
        let error = installer.uninstall(&receipt, false).unwrap_err();
        assert!(error.to_string().contains("receipt_conflict"));
        installer.uninstall(&receipt, true).unwrap();
    }

    #[test]
    fn an_empty_archive_is_rejected() {
        let home = tempdir().unwrap();
        let state = tempdir().unwrap();
        let installer = MacosInstaller::new(home.path().to_owned(), state.path().to_owned());
        let artifact = archive(&[]);
        let error = installer
            .install("job-1", &entry(), "1.0.0", &artifact)
            .unwrap_err();
        assert!(error.to_string().contains("no files"));
    }

    #[test]
    fn an_oversized_archive_is_rejected() {
        let staging = tempdir().unwrap();
        let artifact = archive(&[("claude", b"0123456789")]);
        let error = super::extract_archive_with_limit(&artifact, staging.path(), 5).unwrap_err();
        assert!(error.to_string().contains("uncompressed size"));
    }

    #[test]
    fn recovery_rolls_back_an_uncommitted_pending_install() {
        let home = tempdir().unwrap();
        let state = tempdir().unwrap();
        let installer = MacosInstaller::new(home.path().to_owned(), state.path().to_owned());
        let agent_dir = installer.agent_dir("claude-code");
        let version_dir = agent_dir.join("1.0.0");
        let staging = agent_dir.join(".staging-job-1");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let receipt = Receipt {
            agent_id: "claude-code".to_owned(),
            version: "1.0.0".to_owned(),
            binary_path: version_dir.join("claude"),
            launcher_path: home.path().join(".local/bin/claude-code"),
            artifact_sha256: "a".repeat(64),
            binary_sha256: "b".repeat(64),
            installed_at_unix_seconds: 1,
        };
        let pending = PendingInstall {
            job_id: "job-1".to_owned(),
            version: "1.0.0".to_owned(),
            staging: staging.clone(),
            receipt,
        };
        std::fs::write(
            agent_dir.join("pending.json"),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();

        installer.recover().unwrap();

        assert!(!staging.exists());
        assert!(!version_dir.exists());
        assert!(!agent_dir.join("pending.json").exists());
    }
}
