use std::path::PathBuf;

use agentdesktop_core::catalog::CatalogEntry;
use serde::{Deserialize, Serialize};

/// What was installed, and the exact objects an uninstall may remove.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub agent_id: String,
    pub version: String,
    pub binary_path: PathBuf,
    pub launcher_path: PathBuf,
    pub artifact_sha256: String,
    pub binary_sha256: String,
    pub installed_at_unix_seconds: u64,
}

/// Platform installer used by the job executor.
pub trait Installer: Send + Sync {
    fn install(
        &self,
        job_id: &str,
        entry: &CatalogEntry,
        version: &str,
        artifact: &[u8],
    ) -> anyhow::Result<Receipt>;

    fn uninstall(&self, receipt: &Receipt, allow_modified: bool) -> anyhow::Result<()>;

    fn load_receipt(&self, agent_id: &str) -> Option<Receipt>;

    /// Recover a crash-interrupted install. The default is a no-op for
    /// platforms without a journal.
    fn recover(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Paths an uninstall would remove, without removing them.
    fn uninstall_plan(&self, receipt: &Receipt) -> Vec<PathBuf> {
        vec![receipt.binary_path.clone(), receipt.launcher_path.clone()]
    }
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos::MacosInstaller;
