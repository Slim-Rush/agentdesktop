use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use agentdesktop_core::model::{Agent, Discovery};
use tokio::sync::watch;

use super::*;

fn agent(kind: &str, executable: &Path) -> Agent {
    Agent {
        kind: kind.to_owned(),
        executable: executable.to_path_buf(),
        version: Some("1.0.0".to_owned()),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
    }
}

#[cfg(unix)]
fn fake_cli(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn action_ids(playbook: Playbook) -> Vec<String> {
    playbook
        .actions
        .into_iter()
        .map(|action| action.id)
        .collect()
}

fn summary(action: &str) -> ChatJobSummary {
    ChatJobSummary {
        job_id: "job-1".to_owned(),
        agent_id: "claude-code".to_owned(),
        version: "1.2.0".to_owned(),
        action: action.to_owned(),
        state: 7,
        error_code: "checksum_mismatch".to_owned(),
        error_message: "failed".to_owned(),
        diagnostics: "{}".to_owned(),
        catalog_revision: Some(1),
    }
}

#[test]
fn playbook_maps_each_stable_error_code() {
    for (code, expected) in [
        (
            "checksum_mismatch",
            vec!["refresh_catalog", "retry_install"],
        ),
        (
            "signature_invalid",
            vec!["refresh_catalog", "retry_install"],
        ),
        ("catalog_changed", vec!["refresh_catalog"]),
        ("catalog_expired", vec!["refresh_catalog"]),
        ("permission_denied", vec!["open_settings"]),
        ("extract_failed", vec!["retry_install"]),
        ("install_failed", vec!["retry_install"]),
        ("timeout", vec!["retry_install", "retry_uninstall"]),
    ] {
        assert_eq!(action_ids(playbook_for(code)), expected, "{code}");
    }
    for code in [
        "unsupported_platform",
        "disk_full",
        "receipt_conflict",
        "unknown_code",
    ] {
        assert!(playbook_for(code).actions.is_empty(), "{code}");
        assert!(!playbook_for(code).explanation.is_empty(), "{code}");
    }
}

#[test]
fn playbook_retry_actions_follow_the_failed_job_action() {
    assert_eq!(
        action_ids(playbook_for_job("timeout", "install")),
        vec!["retry_install"]
    );
    assert_eq!(
        action_ids(playbook_for_job("timeout", "update")),
        vec!["retry_install"]
    );
    assert_eq!(
        action_ids(playbook_for_job("timeout", "uninstall")),
        vec!["retry_uninstall"]
    );
    assert!(action_ids(playbook_for_job("timeout", "")).is_empty());
    assert_eq!(
        action_ids(playbook_for_job("checksum_mismatch", "uninstall")),
        vec!["refresh_catalog"]
    );
}

#[test]
fn redaction_removes_home_paths_tokens_and_environment_values() {
    let home = std::env::var("HOME").unwrap();
    let input = format!(
        "failed under {home}/Library/Logs/agentdesktop.log\n\
         OPENAI_API_KEY=sk-live-abcdefghijklmnop\n\
         Authorization: Bearer abcdefghijklmnopqrstuvwxyz\n\
         token=super-secret-value"
    );
    let redacted = redact(&input);

    assert!(!redacted.contains(&home));
    assert!(redacted.contains("~/Library/Logs/agentdesktop.log"));
    assert!(!redacted.contains("sk-live-abcdefghijklmnop"));
    assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(!redacted.contains("super-secret-value"));
    assert!(redacted.contains("OPENAI_API_KEY=<redacted>"));
}

#[test]
fn redaction_caps_oversized_context() {
    let text = "word ".repeat(MAX_CONTEXT_BYTES);
    let redacted = redact(&text);

    assert!(redacted.len() <= MAX_CONTEXT_BYTES + 64);
    assert!(redacted.contains("[truncated]"));
}

#[test]
fn only_verified_profiles_are_enabled() {
    let claude = provider_profiles()
        .iter()
        .find(|profile| profile.id == "claude-code")
        .unwrap();
    assert!(claude.enabled);
    let argv = claude.argv();
    assert_eq!(argv[0], "--print");
    assert!(argv.contains(&"--safe-mode".to_owned()));
    assert!(argv.contains(&"--no-session-persistence".to_owned()));
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == "--tools" && pair[1].is_empty())
    );
    assert!(claude.uses_stdin());

    let codex = provider_profiles()
        .iter()
        .find(|profile| profile.id == "codex")
        .unwrap();
    assert!(!codex.enabled);
    assert!(codex.unavailable_reason.is_some());
}

#[test]
fn providers_require_an_installed_verified_profile() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let executable = fake_cli(dir.path(), "claude", "echo hi");
    #[cfg(not(unix))]
    let executable = dir.path().join("claude");

    let providers = resolve_providers(&Discovery {
        agents: vec![
            agent("claude-code", &executable),
            agent("codex", &executable),
        ],
        model_runtimes: Vec::new(),
    });
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "claude-code");
    assert!(providers[0].available);
    assert!(providers[0].read_only_mode.contains("no tools"));

    let missing = resolve_providers(&Discovery {
        agents: vec![agent("claude-code", &dir.path().join("absent"))],
        model_runtimes: Vec::new(),
    });
    assert!(missing.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn provider_receives_arguments_without_a_shell() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args.txt");
    let stdin_file = dir.path().join("stdin.txt");
    let sentinel = dir.path().join("sentinel");
    let executable = fake_cli(
        dir.path(),
        "echo-args",
        &format!(
            "printf '%s\\n' \"$@\" > '{}'; cat > '{}'; printf 'ok\\n'",
            args_file.display(),
            stdin_file.display()
        ),
    );
    let prompt = format!("hello; touch {}", sentinel.display());

    let output = run_provider(
        &executable,
        dir.path(),
        &["--print".to_owned()],
        Some(&prompt),
        Duration::from_secs(5),
        MAX_OUTPUT_BYTES,
        None,
    )
    .await
    .unwrap();

    let written = fs::read_to_string(args_file).unwrap();
    assert!(!written.contains(&prompt));
    assert_eq!(fs::read_to_string(stdin_file).unwrap(), prompt);
    assert!(!sentinel.exists());
    assert_eq!(output.text, "ok");
}

#[cfg(unix)]
#[tokio::test]
async fn provider_timeout_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let executable = fake_cli(dir.path(), "slow", "sleep 5");
    let started = Instant::now();

    let result = run_provider(
        &executable,
        dir.path(),
        &[],
        None,
        Duration::from_millis(150),
        MAX_OUTPUT_BYTES,
        None,
    )
    .await;

    assert!(matches!(result, Err(ProviderFailure::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[tokio::test]
async fn provider_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let executable = fake_cli(dir.path(), "slow", "sleep 5");
    let (sender, receiver) = watch::channel(false);
    let handle = tokio::spawn(async move {
        run_provider(
            &executable,
            dir.path(),
            &[],
            None,
            Duration::from_secs(5),
            MAX_OUTPUT_BYTES,
            Some(receiver),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    sender.send(true).unwrap();
    let result = handle.await.unwrap();

    assert!(matches!(result, Err(ProviderFailure::Cancelled)));
}

#[cfg(unix)]
#[tokio::test]
async fn provider_output_is_capped() {
    let dir = tempfile::tempdir().unwrap();
    let executable = fake_cli(
        dir.path(),
        "loud",
        "i=0; while [ $i -lt 2000 ]; do printf 'abcdefghij'; i=$((i+1)); done",
    );

    let output = run_provider(
        &executable,
        dir.path(),
        &[],
        None,
        Duration::from_secs(5),
        100,
        None,
    )
    .await
    .unwrap();

    assert!(output.truncated);
    assert!(output.text.len() <= 100 + 64);
    assert!(output.text.contains("[output truncated]"));
}

#[cfg(unix)]
#[tokio::test]
async fn provider_uses_the_given_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let cwd_file = dir.path().join("cwd.txt");
    let executable = fake_cli(
        dir.path(),
        "pwd",
        &format!("pwd -P > '{}'; printf 'ok\\n'", cwd_file.display()),
    );

    run_provider(
        &executable,
        workdir.path(),
        &[],
        None,
        Duration::from_secs(5),
        MAX_OUTPUT_BYTES,
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        fs::canonicalize(fs::read_to_string(cwd_file).unwrap().trim()).unwrap(),
        fs::canonicalize(workdir.path()).unwrap()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn provider_failure_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let executable = fake_cli(dir.path(), "fail", "printf 'boom' >&2; exit 3");

    let result = run_provider(
        &executable,
        dir.path(),
        &[],
        None,
        Duration::from_secs(5),
        MAX_OUTPUT_BYTES,
        None,
    )
    .await;

    match result {
        Err(ProviderFailure::Failed(message)) => {
            assert!(message.contains("exited with"), "{message}");
            assert!(message.contains("boom"), "{message}");
        }
        other => panic!("expected a provider failure, got {other:?}"),
    }
}

#[test]
fn returned_actions_are_limited_to_the_playbook() {
    let allowed = playbook_for("checksum_mismatch").actions;
    let proposed = parse_action_ids(
        "Refresh and retry.\nACTION: retry_install\nACTION: launch_missiles",
        &allowed,
    );

    assert_eq!(
        proposed
            .into_iter()
            .map(|action| action.id)
            .collect::<Vec<_>>(),
        vec!["retry_install"]
    );
}

#[test]
fn context_parses_redacted_job_metadata() {
    let home = std::env::var("HOME").unwrap();
    let jobs = vec![DaemonJob {
        job_id: "job-1".to_owned(),
        state: 7,
        error_code: "checksum_mismatch".to_owned(),
        error_message: format!("failed under {home}/Library/Logs/app.log"),
        diagnostics_json: serde_json::json!({
            "agentId": "claude-code",
            "version": "1.2.0",
            "action": 1,
            "catalogRevision": 7,
            "stderrTail": format!("token=sk-live-abcdefghijklmnop at {home}/tmp/x"),
        })
        .to_string(),
    }];

    let context = build_context(
        Some("job-1"),
        &jobs,
        Some(7),
        false,
        3,
        vec![ChatAgentSummary {
            id: "claude-code".to_owned(),
            display_name: "Claude Code".to_owned(),
            configured: true,
        }],
        Vec::new(),
    )
    .unwrap();

    let job = context.job.as_ref().unwrap();
    assert_eq!(job.agent_id, "claude-code");
    assert_eq!(job.version, "1.2.0");
    assert_eq!(job.action, "install");
    assert_eq!(context.catalog_revision, Some(7));
    assert_eq!(job.catalog_revision, Some(7));
    assert_eq!(context.playbook.error_code, "checksum_mismatch");
    assert!(!context.preview.contains(&home));
    assert!(!context.preview.contains("sk-live-abcdefghijklmnop"));
    assert!(context.preview.contains("claude-code"));
}

#[test]
fn unknown_jobs_are_rejected() {
    let error = build_context(
        Some("job-9"),
        &[],
        Some(1),
        false,
        0,
        Vec::new(),
        Vec::new(),
    )
    .unwrap_err();
    assert!(error.contains("unknown job"));
}

#[test]
fn context_warns_when_daemon_data_is_unavailable() {
    let context = build_context(
        None,
        &[],
        None,
        false,
        0,
        Vec::new(),
        vec!["Daemon sync status is unavailable.".to_owned()],
    )
    .unwrap();

    assert_eq!(context.warnings.len(), 1);
    assert!(context.preview.contains("Unavailable context"));
    assert!(context.preview.contains("sync status is unavailable"));
}

#[test]
fn actions_are_validated_against_the_current_job() {
    let install = summary("install");
    assert!(matches!(
        validate_action("retry_install", &install),
        Ok(ActionKind::RetryInstall)
    ));
    assert!(validate_action("retry_uninstall", &install).is_err());
    assert!(validate_action("launch_missiles", &install).is_err());
    let mut running = install.clone();
    running.state = 4;
    assert!(validate_action("retry_install", &running).is_err());

    let uninstall = summary("uninstall");
    assert!(matches!(
        validate_action("retry_uninstall", &uninstall),
        Ok(ActionKind::RetryUninstall)
    ));
    assert!(matches!(
        validate_action("refresh_catalog", &install),
        Ok(ActionKind::RefreshCatalog)
    ));
}

#[test]
fn prompt_contains_read_only_rules_and_bounded_history() {
    let context = build_context(None, &[], Some(1), false, 0, Vec::new(), Vec::new()).unwrap();
    let history = vec![ChatMessageInput {
        role: "user".to_owned(),
        content: "earlier question".to_owned(),
    }];
    let prompt = build_prompt(&context, &history, "what now?");

    assert!(prompt.contains("read-only"));
    assert!(prompt.contains("ACTION:"));
    assert!(prompt.contains("earlier question"));
    assert!(prompt.contains("what now?"));
    assert!(prompt.len() <= MAX_PROMPT_BYTES);
}

#[test]
fn prompt_bounds_oversized_history() {
    let context = build_context(None, &[], Some(1), false, 0, Vec::new(), Vec::new()).unwrap();
    let history = vec![ChatMessageInput {
        role: "user".to_owned(),
        content: "x".repeat(MAX_MESSAGE_BYTES * 8),
    }];

    let prompt = build_prompt(&context, &history, "what now?");

    assert!(prompt.len() <= MAX_PROMPT_BYTES + 64);
    assert!(prompt.contains("what now?"));
}

#[test]
fn rule_based_reply_uses_the_playbook() {
    let context = build_context(None, &[], Some(1), false, 0, Vec::new(), Vec::new()).unwrap();
    let reply = rule_based_reply(&context);
    assert!(!reply.is_empty());
    assert!(reply.contains("No failed job"));
}

#[test]
fn audit_records_contain_only_action_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("assistant-audit.jsonl");
    let record = AuditRecord {
        timestamp_unix_seconds: 1,
        provider: "claude-code".to_owned(),
        job_id: "job-1".to_owned(),
        action: "retry_install".to_owned(),
        proposed: true,
        confirmed: true,
        outcome: "started".to_owned(),
    };

    append_audit(&path, &record).unwrap();
    append_audit(&path, &record).unwrap();

    let contents = fs::read_to_string(&path).unwrap();
    assert_eq!(contents.lines().count(), 2);
    let parsed: AuditRecord = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
    assert_eq!(parsed, record);
    assert!(!contents.contains("prompt"));
    assert!(!contents.contains("message"));

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn chat_runs_reject_duplicate_ids_and_cancel_once() {
    let runs = ChatRuns::default();
    let receiver = runs.register("chat-1").unwrap();
    assert!(runs.register("chat-1").is_err());
    assert!(runs.cancel("chat-1"));
    assert!(*receiver.borrow());
    runs.finish("chat-1");
    assert!(!runs.cancel("chat-1"));
    let _second = runs.register("chat-2").unwrap();
    let _third = runs.register("chat-3").unwrap();
    assert!(runs.register("chat-4").is_err());
    runs.finish("chat-2");
    assert!(runs.register("chat-4").is_ok());
    assert!(runs.register("../escape").is_err());
}
