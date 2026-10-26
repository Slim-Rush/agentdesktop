//! Artifact verification for catalog-driven installs.
//!
//! The catalog binds a SHA-256 and an Ed25519 signature for every artifact.
//! Nothing is extracted or executed before both checks pass.

use agentdesktop_core::catalog::{
    CatalogEntry, CatalogPublicKey, CatalogSnapshot, CatalogVersion, sha256_hex,
};
use anyhow::{Context, ensure};
use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Verify `bytes` against the catalog entry's version and the pinned key used
/// to sign the catalog.
pub fn verify_artifact<'a>(
    snapshot: &CatalogSnapshot,
    entry: &'a CatalogEntry,
    version: &str,
    bytes: &[u8],
    keys: &[CatalogPublicKey],
) -> anyhow::Result<&'a CatalogVersion> {
    let expected = entry
        .versions
        .iter()
        .find(|candidate| candidate.version == version)
        .with_context(|| format!("unknown agent version {version:?}"))?;
    let digest = sha256_hex(bytes);
    ensure!(
        digest == expected.sha256,
        "checksum_mismatch: expected {}, got {digest}",
        expected.sha256
    );
    let key = keys
        .iter()
        .find(|key| key.key_id == snapshot.key_id)
        .with_context(|| format!("catalog key id {:?} is not pinned", snapshot.key_id))?;
    let public_key = BASE64
        .decode(key.public_key.as_bytes())
        .context("decode catalog public key")?;
    let signature = BASE64
        .decode(expected.signature.as_bytes())
        .context("decode artifact signature")?;
    UnparsedPublicKey::new(&ED25519, &public_key)
        .verify(bytes, &signature)
        .map_err(|_| anyhow::anyhow!("signature_invalid: artifact signature does not match"))?;
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use agentdesktop_core::catalog::{
        Catalog, CatalogEntry, CatalogPublicKey, CatalogSnapshot, CatalogVersion, DetectionRule,
        InstallKind, canonical_json,
    };
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
    use base64::Engine as _;

    use super::verify_artifact;

    fn fixture() -> (CatalogSnapshot, CatalogEntry, CatalogPublicKey, Vec<u8>) {
        let key_pair = Ed25519KeyPair::generate().expect("generate Ed25519 key");
        let public_key = key_pair.public_key().as_ref().to_vec();
        let bytes = b"artifact bytes".to_vec();
        let signature = key_pair.sign(&bytes);
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
                sha256: agentdesktop_core::catalog::sha256_hex(&bytes),
                signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
            }],
            detection: DetectionRule {
                paths: vec!["~/.local/bin/claude".to_owned()],
            },
        };
        let catalog = Catalog {
            catalog_id: "test".to_owned(),
            revision: 1,
            created_at: 1,
            expires_at: u64::MAX,
            entries: vec![entry.clone()],
        };
        let canonical_bytes = canonical_json(&catalog).unwrap();
        let snapshot = CatalogSnapshot {
            catalog,
            key_id: "test-key".to_owned(),
            sha256: agentdesktop_core::catalog::sha256_hex(&canonical_bytes),
            canonical_bytes,
            signed_catalog_json: Vec::new(),
        };
        (
            snapshot,
            entry,
            CatalogPublicKey {
                key_id: "test-key".to_owned(),
                public_key: base64::engine::general_purpose::STANDARD.encode(public_key),
            },
            bytes,
        )
    }

    #[test]
    fn a_matching_artifact_is_accepted() {
        let (snapshot, entry, key, bytes) = fixture();
        verify_artifact(&snapshot, &entry, "1.0.0", &bytes, &[key]).expect("verify");
    }

    #[test]
    fn a_checksum_mismatch_is_rejected() {
        let (snapshot, entry, key, mut bytes) = fixture();
        bytes[0] ^= 0x01;
        let error = verify_artifact(&snapshot, &entry, "1.0.0", &bytes, &[key]).unwrap_err();
        assert!(error.to_string().contains("checksum_mismatch"));
    }

    #[test]
    fn a_signature_mismatch_is_rejected() {
        let (snapshot, entry, key, _) = fixture();
        let error =
            verify_artifact(&snapshot, &entry, "1.0.0", b"other bytes", &[key]).unwrap_err();
        assert!(
            error.to_string().contains("checksum_mismatch")
                || error.to_string().contains("signature_invalid")
        );
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let (snapshot, entry, key, bytes) = fixture();
        let error = verify_artifact(&snapshot, &entry, "9.9.9", &bytes, &[key]).unwrap_err();
        assert!(error.to_string().contains("unknown agent version"));
    }

    #[test]
    fn an_unknown_catalog_key_is_rejected() {
        let (snapshot, entry, _key, bytes) = fixture();
        let error = verify_artifact(&snapshot, &entry, "1.0.0", &bytes, &[]).unwrap_err();
        assert!(error.to_string().contains("not pinned"));
    }
}
