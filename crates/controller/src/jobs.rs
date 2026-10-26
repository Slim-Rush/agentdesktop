//! Controller-side job state machine for user-level agent install/uninstall.

use agentdesktop_proto::fleet::{
    JobAction, JobAction as ProtoJobAction, JobState as ProtoJobState,
};

/// Input for creating a job. The controller assigns the id and timestamps.
#[derive(Clone, Debug)]
pub struct NewJob {
    pub device_id: String,
    pub idempotency_key: String,
    pub origin: String,
    pub requested_by: String,
    pub catalog_id: String,
    pub catalog_revision: i64,
    pub catalog_sha256: String,
    pub agent_id: String,
    pub version: String,
    pub action: JobAction,
    pub deadline_unix_seconds: i64,
}

/// A persisted job row.
#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow)]
pub struct JobRecord {
    pub id: String,
    pub device_id: String,
    pub attempt: i64,
    pub idempotency_key: String,
    pub origin: String,
    pub requested_by: String,
    pub catalog_id: String,
    pub catalog_revision: i64,
    pub catalog_sha256: String,
    pub agent_id: String,
    pub version: String,
    pub action: i64,
    pub state: i64,
    pub deadline_unix_seconds: i64,
    pub error_code: String,
    pub error_message: String,
    pub diagnostics_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl JobRecord {
    pub fn state(&self) -> Option<JobState> {
        JobState::from_proto(self.state as i32)
    }

    pub fn to_proto(&self) -> agentdesktop_proto::fleet::AgentJob {
        agentdesktop_proto::fleet::AgentJob {
            job_id: self.id.clone(),
            attempt: u32::try_from(self.attempt).unwrap_or(u32::MAX),
            idempotency_key: self.idempotency_key.clone(),
            origin: self.origin.clone(),
            requested_by: self.requested_by.clone(),
            catalog_id: self.catalog_id.clone(),
            catalog_revision: u64::try_from(self.catalog_revision).unwrap_or(0),
            catalog_sha256: self.catalog_sha256.clone(),
            agent_id: self.agent_id.clone(),
            version: self.version.clone(),
            action: self.action as i32,
            deadline_unix_seconds: u64::try_from(self.deadline_unix_seconds).unwrap_or(0),
        }
    }
}

/// The lifecycle state stored in `agent_jobs.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Dispatched,
    Accepted,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}

/// Events that can move a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobEvent {
    Dispatch,
    Accept,
    Start,
    Succeed,
    Fail,
    Cancel,
    RequestCancel,
    Expire,
    Reconnect,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatched => "dispatched",
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::CancelRequested => "cancelRequested",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired
        )
    }

    pub fn to_proto(self) -> ProtoJobState {
        match self {
            Self::Queued => ProtoJobState::Queued,
            Self::Dispatched => ProtoJobState::Dispatched,
            Self::Accepted => ProtoJobState::Accepted,
            Self::Running => ProtoJobState::Running,
            Self::CancelRequested => ProtoJobState::CancelRequested,
            Self::Succeeded => ProtoJobState::Succeeded,
            Self::Failed => ProtoJobState::Failed,
            Self::Cancelled => ProtoJobState::Cancelled,
            Self::Expired => ProtoJobState::Expired,
        }
    }

    pub fn proto_value(self) -> i64 {
        self.to_proto() as i64
    }

    pub fn from_proto(value: i32) -> Option<Self> {
        match ProtoJobState::try_from(value).ok()? {
            ProtoJobState::Queued => Some(Self::Queued),
            ProtoJobState::Dispatched => Some(Self::Dispatched),
            ProtoJobState::Accepted => Some(Self::Accepted),
            ProtoJobState::Running => Some(Self::Running),
            ProtoJobState::CancelRequested => Some(Self::CancelRequested),
            ProtoJobState::Succeeded => Some(Self::Succeeded),
            ProtoJobState::Failed => Some(Self::Failed),
            ProtoJobState::Cancelled => Some(Self::Cancelled),
            ProtoJobState::Expired => Some(Self::Expired),
            ProtoJobState::Unspecified => None,
        }
    }
}

/// The next state, or `None` when the transition is not allowed.
///
/// The table is deliberately explicit: a lost acknowledgement, a duplicate
/// dispatch, and a late result from an older attempt must not move a job
/// backwards or overwrite a terminal state.
pub fn transition(state: JobState, event: JobEvent) -> Option<JobState> {
    use JobEvent as E;
    use JobState as S;
    match (state, event) {
        (S::Queued, E::Dispatch) => Some(S::Dispatched),
        (S::Queued, E::Cancel) => Some(S::Cancelled),
        (S::Queued, E::Expire) => Some(S::Expired),
        (S::Dispatched, E::Accept) => Some(S::Accepted),
        (S::Dispatched, E::Expire) => Some(S::Expired),
        (S::Dispatched, E::Cancel | E::RequestCancel) => Some(S::CancelRequested),
        (S::Accepted, E::Start) => Some(S::Running),
        (S::Accepted, E::Cancel) => Some(S::Cancelled),
        (S::Accepted, E::RequestCancel) => Some(S::CancelRequested),
        (S::Running, E::Succeed) => Some(S::Succeeded),
        (S::Running, E::Fail) => Some(S::Failed),
        (S::Running, E::RequestCancel) => Some(S::CancelRequested),
        // Before the commit point a cancel wins; after it the daemon reports
        // success/failure and the cancel is ignored (recorded by the caller).
        (S::CancelRequested, E::Cancel) => Some(S::Cancelled),
        (S::CancelRequested, E::Succeed) => Some(S::Succeeded),
        (S::CancelRequested, E::Fail) => Some(S::Failed),
        // Reconnect replays the current state; duplicate delivery is a no-op.
        (
            S::Queued | S::Dispatched | S::Accepted | S::Running | S::CancelRequested,
            E::Reconnect,
        ) => Some(state),
        (terminal, E::Reconnect) if terminal.is_terminal() => Some(terminal),
        _ => None,
    }
}

/// Stable error codes used in `JobStatus.error_code`.
pub mod error_code {
    pub const UNKNOWN_AGENT: &str = "unknown_agent";
    pub const CATALOG_CHANGED: &str = "catalog_changed";
    pub const CHECKSUM_MISMATCH: &str = "checksum_mismatch";
    pub const SIGNATURE_INVALID: &str = "signature_invalid";
    pub const UNSUPPORTED_PLATFORM: &str = "unsupported_platform";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const DISK_FULL: &str = "disk_full";
    pub const EXTRACT_FAILED: &str = "extract_failed";
    pub const INSTALL_FAILED: &str = "install_failed";
    pub const RECEIPT_CONFLICT: &str = "receipt_conflict";
    pub const INTERRUPTED: &str = "interrupted";
    pub const TIMEOUT: &str = "timeout";
}

pub fn action_to_proto(action: ProtoJobAction) -> &'static str {
    match action {
        ProtoJobAction::Install => "install",
        ProtoJobAction::Uninstall => "uninstall",
        ProtoJobAction::Update => "update",
        ProtoJobAction::Unspecified => "unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::{JobEvent as E, JobState as S, transition};

    #[test]
    fn the_happy_path_reaches_succeeded() {
        let mut state = S::Queued;
        for event in [E::Dispatch, E::Accept, E::Start, E::Succeed] {
            state = transition(state, event).expect("valid transition");
        }
        assert_eq!(state, S::Succeeded);
        assert!(state.is_terminal());
    }

    #[test]
    fn a_queued_job_can_cancel_or_expire() {
        assert_eq!(transition(S::Queued, E::Cancel), Some(S::Cancelled));
        assert_eq!(transition(S::Queued, E::Expire), Some(S::Expired));
        assert_eq!(transition(S::Dispatched, E::Expire), Some(S::Expired));
    }

    #[test]
    fn a_running_cancel_is_decided_by_the_commit_point() {
        assert_eq!(
            transition(S::Running, E::RequestCancel),
            Some(S::CancelRequested)
        );
        assert_eq!(
            transition(S::CancelRequested, E::Cancel),
            Some(S::Cancelled)
        );
        assert_eq!(
            transition(S::CancelRequested, E::Succeed),
            Some(S::Succeeded)
        );
    }

    #[test]
    fn terminal_states_never_move_on_duplicate_or_reconnect() {
        for state in [S::Succeeded, S::Failed, S::Cancelled, S::Expired] {
            assert_eq!(transition(state, E::Reconnect), Some(state));
            assert_eq!(transition(state, E::Dispatch), None);
            assert_eq!(transition(state, E::Succeed), None);
        }
    }

    #[test]
    fn an_accepted_job_can_be_cancelled_before_start() {
        assert_eq!(transition(S::Accepted, E::Cancel), Some(S::Cancelled));
        assert_eq!(
            transition(S::Accepted, E::RequestCancel),
            Some(S::CancelRequested)
        );
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        assert_eq!(transition(S::Queued, E::Succeed), None);
        assert_eq!(transition(S::Dispatched, E::Start), None);
        assert_eq!(transition(S::Running, E::Dispatch), None);
    }
}
