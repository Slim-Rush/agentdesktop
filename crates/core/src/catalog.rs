//! Signed agent catalog shared by the controller and the daemon.
//!
//! The catalog is the only source of installable agent metadata. The controller
//! serves it; the daemon verifies the signature before using it. This module
//! owns the wire types, deterministic canonical JSON, and Ed25519 verification
//! so both sides use exactly the same rules.

use std::collections::BTreeMap;

use anyhow::{Context, ensure};
use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A catalog of installable agents. The signature covers exactly this value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub catalog_id: String,
    pub revision: u64,
    /// Unix seconds when the catalog was produced.
    pub created_at: u64,
    /// Unix seconds after which the catalog cannot authorize new jobs.
    pub expires_at: u64,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub display_name: String,
    /// The `ProgramsConfig` field this agent maps to (for example `claudeCode`).
    pub program_key: String,
    pub platforms: Vec<String>,
    pub architectures: Vec<String>,
    pub install_kind: InstallKind,
    pub versions: Vec<CatalogVersion>,
    pub detection: DetectionRule,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    User,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogVersion {
    pub version: String,
    /// HTTPS URL of the artifact.
    pub artifact_url: String,
    /// Lowercase hex SHA-256 of the artifact bytes.
    pub sha256: String,
    /// Base64 Ed25519 signature over the artifact bytes.
    pub signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectionRule {
    /// Paths relative to the user's home directory, or absolute paths.
    pub paths: Vec<String>,
}

/// A catalog plus the detached signature and key id.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedCatalog {
    pub catalog: Catalog,
    pub key_id: String,
    /// Base64 Ed25519 signature over `canonical_json(&catalog)`.
    pub signature: String,
}

/// A public key pinned by the daemon or the controller.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogPublicKey {
    pub key_id: String,
    /// Base64 raw 32-byte Ed25519 public key.
    pub public_key: String,
}

/// A verified catalog and the exact bytes its signature covers.
#[derive(Clone, Debug)]
pub struct CatalogSnapshot {
    pub catalog: Catalog,
    pub canonical_bytes: Vec<u8>,
    pub sha256: String,
    /// The signed envelope, suitable for serving to a daemon.
    pub signed_catalog_json: Vec<u8>,
}

impl Catalog {
    /// Whether this catalog may authorize new install/update jobs at `now`.
    pub fn is_expired(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at
    }
}

impl SignedCatalog {
    /// Verify the catalog against the pinned keys and return a snapshot.
    ///
    /// A key id that is not pinned is rejected even when the signature is
    /// otherwise valid. The caller is responsible for revision ordering.
    pub fn verify(&self, keys: &[CatalogPublicKey]) -> anyhow::Result<CatalogSnapshot> {
        validate_catalog(&self.catalog)?;

        let key = keys
            .iter()
            .find(|key| key.key_id == self.key_id)
            .with_context(|| format!("catalog key id {:?} is not pinned", self.key_id))?;
        let public_key = BASE64
            .decode(key.public_key.as_bytes())
            .context("decode catalog public key")?;
        let signature = BASE64
            .decode(self.signature.as_bytes())
            .context("decode catalog signature")?;
        let canonical_bytes = canonical_json(&self.catalog)?;

        UnparsedPublicKey::new(&ED25519, &public_key)
            .verify(&canonical_bytes, &signature)
            .map_err(|_| anyhow::anyhow!("catalog signature does not match"))?;

        Ok(CatalogSnapshot {
            sha256: sha256_hex(&canonical_bytes),
            catalog: self.catalog.clone(),
            canonical_bytes,
            signed_catalog_json: serde_json::to_vec(self)
                .context("encode signed catalog envelope")?,
        })
    }
}

/// Deterministic JSON: object keys are sorted recursively. This is the byte
/// sequence both the publisher and the verifier sign.
pub fn canonical_json<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    let value = serde_json::to_value(value).context("serialize catalog for signing")?;
    let value = canonicalize(value);
    serde_json::to_vec(&value).context("encode canonical catalog")
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in map {
                sorted.insert(key, canonicalize(value));
            }
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn validate_catalog(catalog: &Catalog) -> anyhow::Result<()> {
    ensure!(!catalog.catalog_id.is_empty(), "catalog id is empty");
    ensure!(catalog.revision > 0, "catalog revision must be positive");
    ensure!(
        catalog.expires_at > catalog.created_at,
        "catalog expires_at must be after created_at"
    );
    for entry in &catalog.entries {
        ensure!(!entry.id.is_empty(), "catalog entry id is empty");
        ensure!(
            !entry.display_name.is_empty(),
            "catalog entry display name is empty"
        );
        ensure!(
            !entry.program_key.is_empty(),
            "catalog entry program_key is empty"
        );
        ensure!(
            !entry.versions.is_empty(),
            "catalog entry {} has no versions",
            entry.id
        );
        for version in &entry.versions {
            ensure!(
                version.artifact_url.starts_with("https://"),
                "catalog entry {} version {} is not HTTPS",
                entry.id,
                version.version
            );
            ensure!(
                version.sha256.len() == 64
                    && version.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "catalog entry {} version {} has an invalid sha256",
                entry.id,
                version.version
            );
            let signature = BASE64
                .decode(version.signature.as_bytes())
                .with_context(|| {
                    format!(
                        "catalog entry {} version {} has an invalid signature",
                        entry.id, version.version
                    )
                })?;
            ensure!(
                signature.len() == 64,
                "catalog entry {} version {} signature has the wrong length",
                entry.id,
                version.version
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
    use base64::Engine as _;
    use serde_json::json;

    use super::{
        Catalog, CatalogEntry, CatalogPublicKey, CatalogVersion, DetectionRule, InstallKind,
        SignedCatalog, canonical_json, sha256_hex,
    };

    fn catalog() -> Catalog {
        Catalog {
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
                    artifact_url: "https://example.com/claude-code.tar.gz".to_owned(),
                    sha256: "a".repeat(64),
                    signature: base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
                }],
                detection: DetectionRule {
                    paths: vec!["~/.local/bin/claude".to_owned()],
                },
            }],
        }
    }

    fn signed() -> (SignedCatalog, CatalogPublicKey) {
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        let public_key = key_pair.public_key().as_ref().to_vec();
        let catalog = catalog();
        let bytes = canonical_json(&catalog).expect("canonical catalog");
        let signature = key_pair.sign(&bytes);
        (
            SignedCatalog {
                catalog,
                key_id: "test-key".to_owned(),
                signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
            },
            CatalogPublicKey {
                key_id: "test-key".to_owned(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
            },
        )
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        let left = json!({"b": 1, "a": {"d": 2, "c": 3}});
        let right = json!({"a": {"c": 3, "d": 2}, "b": 1});
        assert_eq!(
            canonical_json(&left).unwrap(),
            canonical_json(&right).unwrap()
        );
    }

    #[test]
    fn a_valid_catalog_is_accepted() {
        let (catalog, key) = signed();
        let snapshot = catalog.verify(&[key]).expect("verify catalog");
        assert_eq!(snapshot.catalog.revision, 1);
        assert_eq!(snapshot.sha256, sha256_hex(&snapshot.canonical_bytes));
    }

    #[test]
    fn a_tampered_catalog_is_rejected() {
        let (mut catalog, key) = signed();
        catalog.catalog.entries[0].display_name = "Tampered".to_owned();
        let error = catalog.verify(&[key]).expect_err("reject tampered catalog");
        assert!(error.to_string().contains("signature"));
    }

    #[test]
    fn an_unknown_key_id_is_rejected() {
        let (mut catalog, key) = signed();
        catalog.key_id = "other-key".to_owned();
        let error = catalog.verify(&[key]).expect_err("reject unknown key");
        assert!(error.to_string().contains("not pinned"));
    }

    #[test]
    fn an_expired_catalog_cannot_authorize_new_jobs() {
        let mut catalog = catalog();
        catalog.expires_at = 10;
        assert!(catalog.is_expired(10));
        assert!(!catalog.is_expired(9));
    }
}
