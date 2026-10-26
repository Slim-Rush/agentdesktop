//! Loads and serves the signed agent catalog.

use std::{path::Path, sync::Arc};

use agentdesktop_core::catalog::{
    CatalogPublicKey, CatalogSnapshot, MAX_CATALOG_BYTES, SignedCatalog,
};
use anyhow::{Context, ensure};

/// The verified catalog served to enrolled devices. The file is loaded once at
/// startup; a later upload or admin API is a separate Story.
#[derive(Clone, Default)]
pub struct CatalogStore {
    snapshot: Option<Arc<CatalogSnapshot>>,
}

impl CatalogStore {
    /// Load a signed catalog file and verify it against the pinned keys.
    pub fn load(path: &Path, keys: &[CatalogPublicKey]) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read agent catalog {}", path.display()))?;
        ensure!(
            bytes.len() <= MAX_CATALOG_BYTES,
            "agent catalog exceeds the {} byte limit",
            MAX_CATALOG_BYTES
        );
        let signed: SignedCatalog =
            serde_json::from_slice(&bytes).context("parse signed agent catalog")?;
        let snapshot = signed.verify(keys).context("verify signed agent catalog")?;
        Ok(Self {
            snapshot: Some(Arc::new(snapshot)),
        })
    }

    /// No catalog is configured; catalog RPCs return `failed_precondition`.
    pub fn empty() -> Self {
        Self { snapshot: None }
    }

    pub fn current(&self) -> Option<Arc<CatalogSnapshot>> {
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use agentdesktop_core::catalog::{
        Catalog, CatalogEntry, CatalogPublicKey, CatalogVersion, DetectionRule, InstallKind,
        SignedCatalog, canonical_json,
    };
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
    use base64::Engine as _;

    use super::CatalogStore;

    fn write_catalog() -> (std::path::PathBuf, CatalogPublicKey) {
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        let public_key = key_pair.public_key().as_ref().to_vec();
        let catalog = Catalog {
            catalog_id: "test".to_owned(),
            revision: 1,
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
        let bytes = canonical_json(&catalog).expect("canonical catalog");
        let signature = key_pair.sign(&bytes);
        let signed = SignedCatalog {
            catalog,
            key_id: "test-key".to_owned(),
            signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
        };
        let path = std::env::temp_dir().join(format!(
            "agentdesktop-catalog-{}-{}.json",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::write(&path, serde_json::to_vec(&signed).unwrap()).unwrap();
        (
            path,
            CatalogPublicKey {
                key_id: "test-key".to_owned(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
            },
        )
    }

    #[test]
    fn a_verified_catalog_is_served() {
        let (path, key) = write_catalog();
        let store = CatalogStore::load(&path, &[key]).expect("load catalog");
        let snapshot = store.current().expect("catalog present");
        assert_eq!(snapshot.catalog.revision, 1);
        assert!(!snapshot.signed_catalog_json.is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_empty_store_serves_nothing() {
        assert!(CatalogStore::empty().current().is_none());
    }

    #[test]
    fn a_tampered_file_is_rejected() {
        let (path, key) = write_catalog();
        let mut bytes = std::fs::read(&path).unwrap();
        let position = bytes.len() / 2;
        bytes[position] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();
        assert!(CatalogStore::load(&path, &[key]).is_err());
        std::fs::remove_file(path).ok();
    }
}
