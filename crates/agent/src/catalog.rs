//! Daemon-side signed catalog cache.
//!
//! The cache is versioned: a full catalog revision is written first, then a
//! small `current` pointer is atomically replaced. A crash leaves either the
//! complete old revision or the complete new one.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use agentdesktop_core::catalog::{
    CatalogPublicKey, CatalogSnapshot, MAX_CATALOG_BYTES, SignedCatalog,
};
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::secure_fs;

#[derive(Clone, Debug, Default)]
pub struct CatalogCache {
    snapshot: Option<Arc<CatalogSnapshot>>,
    last_sync: Option<u64>,
    stale: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub revision: u64,
    pub sha256: String,
    pub last_sync_unix_seconds: u64,
    pub stale: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogPointer {
    revision: u64,
    sha256: String,
    last_sync_unix_seconds: u64,
}

impl CatalogCache {
    /// Load the last committed revision. A missing or corrupt cache is empty;
    /// the next successful sync replaces it.
    pub fn load(state_dir: &Path, keys: &[CatalogPublicKey]) -> Self {
        let directory = catalog_dir(state_dir);
        let pointer_path = directory.join("current");
        let bytes = match std::fs::read(&pointer_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                warn!(error = %error, "could not read catalog pointer");
                return Self::default();
            }
        };
        let pointer: CatalogPointer = match serde_json::from_slice(&bytes) {
            Ok(pointer) => pointer,
            Err(error) => {
                warn!(error = %error, "ignoring corrupt catalog pointer");
                return Self::default();
            }
        };
        let signed_path = directory
            .join(pointer.revision.to_string())
            .join("signed.json");
        let signed_bytes = match std::fs::read(&signed_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(error = %error, "could not read cached catalog");
                return Self::default();
            }
        };
        let signed: SignedCatalog = match serde_json::from_slice(&signed_bytes) {
            Ok(signed) => signed,
            Err(error) => {
                warn!(error = %error, "ignoring corrupt cached catalog");
                return Self::default();
            }
        };
        match signed.verify(keys) {
            Ok(snapshot) if snapshot.sha256 == pointer.sha256 => Self {
                snapshot: Some(Arc::new(snapshot)),
                last_sync: Some(pointer.last_sync_unix_seconds),
                stale: false,
            },
            Ok(snapshot) => {
                warn!(
                    revision = snapshot.catalog.revision,
                    "cached catalog hash does not match its pointer"
                );
                Self::default()
            }
            Err(error) => {
                warn!(error = %error, "cached catalog failed verification");
                Self::default()
            }
        }
    }

    /// Verify and commit a signed catalog. The previous snapshot is retained
    /// when verification fails.
    pub fn replace(
        &mut self,
        state_dir: &Path,
        signed_json: &[u8],
        keys: &[CatalogPublicKey],
        now_unix_seconds: u64,
    ) -> anyhow::Result<Arc<CatalogSnapshot>> {
        ensure!(
            signed_json.len() <= MAX_CATALOG_BYTES,
            "catalog exceeds the {} byte limit",
            MAX_CATALOG_BYTES
        );
        let signed: SignedCatalog =
            serde_json::from_slice(signed_json).context("parse signed catalog")?;
        let snapshot = Arc::new(signed.verify(keys).context("verify signed catalog")?);
        ensure!(
            snapshot.catalog.revision > 0,
            "catalog revision must be positive"
        );
        if let Some(current) = self.snapshot.as_ref() {
            if snapshot.catalog.revision < current.catalog.revision {
                bail!(
                    "catalog revision {} is older than the current revision {}",
                    snapshot.catalog.revision,
                    current.catalog.revision
                );
            }
            if snapshot.catalog.revision == current.catalog.revision {
                if snapshot.sha256 == current.sha256 {
                    return Ok(Arc::clone(current));
                }
                bail!(
                    "catalog revision {} has a different hash",
                    snapshot.catalog.revision
                );
            }
        }

        let directory = catalog_dir(state_dir);
        secure_fs::ensure_private_dir(&directory)?;
        let revision_dir = directory.join(snapshot.catalog.revision.to_string());
        secure_fs::ensure_private_dir(&revision_dir)?;
        secure_fs::atomic_write(&revision_dir.join("signed.json"), signed_json, 0o600)?;
        let pointer = CatalogPointer {
            revision: snapshot.catalog.revision,
            sha256: snapshot.sha256.clone(),
            last_sync_unix_seconds: now_unix_seconds,
        };
        let pointer_bytes = serde_json::to_vec(&pointer).context("encode catalog pointer")?;
        secure_fs::atomic_write(&directory.join("current"), &pointer_bytes, 0o600)?;

        self.snapshot = Some(Arc::clone(&snapshot));
        self.last_sync = Some(now_unix_seconds);
        self.stale = false;
        Ok(snapshot)
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub fn snapshot(&self) -> Option<Arc<CatalogSnapshot>> {
        self.snapshot.clone()
    }

    pub fn status(&self) -> Option<CatalogStatus> {
        self.snapshot.as_ref().map(|snapshot| CatalogStatus {
            revision: snapshot.catalog.revision,
            sha256: snapshot.sha256.clone(),
            last_sync_unix_seconds: self.last_sync.unwrap_or(0),
            stale: self.stale,
        })
    }
}

fn catalog_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("catalog")
}

#[cfg(test)]
mod tests {
    use agentdesktop_core::catalog::{
        Catalog, CatalogEntry, CatalogPublicKey, CatalogVersion, DetectionRule, InstallKind,
        SignedCatalog, canonical_json,
    };
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
    use base64::Engine as _;
    use tempfile::tempdir;

    use super::CatalogCache;

    fn signed(revision: u64) -> (Vec<u8>, CatalogPublicKey) {
        signed_with(revision, "test")
    }

    fn signed_with(revision: u64, catalog_id: &str) -> (Vec<u8>, CatalogPublicKey) {
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        signed_with_key(revision, catalog_id, &key_pair)
    }

    fn signed_with_key(
        revision: u64,
        catalog_id: &str,
        key_pair: &Ed25519KeyPair,
    ) -> (Vec<u8>, CatalogPublicKey) {
        let public_key = key_pair.public_key().as_ref().to_vec();
        let catalog = Catalog {
            catalog_id: catalog_id.to_owned(),
            revision,
            created_at: 1,
            expires_at: u64::MAX,
            entries: vec![CatalogEntry {
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
                    signature: base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
                }],
                detection: DetectionRule {
                    paths: vec!["~/.local/bin/claude".to_owned()],
                },
            }],
        };
        let bytes = canonical_json(&catalog).unwrap();
        let signature = key_pair.sign(&bytes);
        let signed = SignedCatalog {
            catalog,
            key_id: "test-key".to_owned(),
            signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
        };
        (
            serde_json::to_vec(&signed).unwrap(),
            CatalogPublicKey {
                key_id: "test-key".to_owned(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
            },
        )
    }

    #[test]
    fn an_oversized_catalog_is_rejected() {
        let directory = tempdir().unwrap();
        let bytes = vec![0_u8; agentdesktop_core::catalog::MAX_CATALOG_BYTES + 1];
        let mut cache = CatalogCache::default();
        let error = cache.replace(directory.path(), &bytes, &[], 0).unwrap_err();
        assert!(error.to_string().contains("byte limit"));
    }

    #[test]
    fn a_committed_catalog_survives_reload() {
        let directory = tempdir().unwrap();
        let (bytes, key) = signed(1);
        let mut cache = CatalogCache::default();
        cache
            .replace(directory.path(), &bytes, std::slice::from_ref(&key), 10)
            .unwrap();
        let reloaded = CatalogCache::load(directory.path(), &[key]);
        let snapshot = reloaded.snapshot().expect("cached catalog");
        assert_eq!(snapshot.catalog.revision, 1);
        assert_eq!(reloaded.status().unwrap().last_sync_unix_seconds, 10);
    }

    #[test]
    fn a_tampered_catalog_keeps_the_previous_snapshot() {
        let directory = tempdir().unwrap();
        let (bytes, key) = signed(1);
        let mut cache = CatalogCache::default();
        cache
            .replace(directory.path(), &bytes, std::slice::from_ref(&key), 10)
            .unwrap();
        let mut tampered = bytes.clone();
        tampered[10] ^= 0x01;
        assert!(
            cache
                .replace(directory.path(), &tampered, &[key], 20)
                .is_err()
        );
        assert_eq!(cache.snapshot().unwrap().catalog.revision, 1);
    }

    #[test]
    fn a_failed_sync_can_mark_the_cache_stale() {
        let directory = tempdir().unwrap();
        let (bytes, key) = signed(1);
        let mut cache = CatalogCache::default();
        cache.replace(directory.path(), &bytes, &[key], 10).unwrap();
        cache.mark_stale();
        assert!(cache.status().unwrap().stale);
    }

    #[test]
    fn an_older_revision_is_rejected() {
        let directory = tempdir().unwrap();
        let (newer, newer_key) = signed(2);
        let (older, older_key) = signed(1);
        let mut cache = CatalogCache::default();
        cache
            .replace(
                directory.path(),
                &newer,
                std::slice::from_ref(&newer_key),
                10,
            )
            .unwrap();
        let error = cache
            .replace(directory.path(), &older, &[older_key, newer_key], 20)
            .unwrap_err();
        assert!(error.to_string().contains("older"));
        assert_eq!(cache.snapshot().unwrap().catalog.revision, 2);
    }

    #[test]
    fn the_same_revision_with_a_different_hash_is_rejected() {
        let directory = tempdir().unwrap();
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        let (first, key) = signed_with_key(1, "first", &key_pair);
        let (second, _) = signed_with_key(1, "second", &key_pair);
        let mut cache = CatalogCache::default();
        cache
            .replace(directory.path(), &first, std::slice::from_ref(&key), 10)
            .unwrap();
        let error = cache
            .replace(directory.path(), &second, &[key], 20)
            .unwrap_err();
        assert!(error.to_string().contains("different hash"));
        assert_eq!(cache.snapshot().unwrap().catalog.catalog_id, "first");
    }

    #[test]
    fn the_same_revision_with_the_same_hash_is_unchanged() {
        let directory = tempdir().unwrap();
        let (bytes, key) = signed(1);
        let mut cache = CatalogCache::default();
        cache
            .replace(directory.path(), &bytes, std::slice::from_ref(&key), 10)
            .unwrap();
        let snapshot = cache.replace(directory.path(), &bytes, &[key], 20).unwrap();
        assert_eq!(snapshot.catalog.revision, 1);
        assert_eq!(cache.status().unwrap().last_sync_unix_seconds, 10);
    }
}
