import { CardHeader } from "@agentdesktop/ui";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  LoaderCircle,
  RefreshCw,
  Settings2,
  Trash2,
  Upload,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { redactPaths } from "../redact";
import type {
  AgentAction,
  AgentConfigResponse,
  AgentJob,
  AgentSummary,
  Discovery,
  DryRunPlan,
  SyncStatus,
} from "../types";

type Notice = { tone: "success" | "error"; message: string } | null;

export interface AgentsViewProps {
  agents: AgentSummary[] | null;
  discovery: Discovery | null;
  jobs: AgentJob[];
  sync: SyncStatus | null;
  platform: string;
  architecture: string;
  loading: boolean;
  error: string | null;
  notice: Notice;
  jobPollError: string | null;
  busyAgent: string | null;
  onStartJob: (agentId: string, action: AgentAction, version: string) => void;
  onDryRun: (agentId: string, version: string) => Promise<DryRunPlan>;
  getConfig: (agentId: string) => Promise<AgentConfigResponse>;
  saveConfig: (
    agentId: string,
    revision: number,
    config: unknown,
  ) => Promise<AgentConfigResponse>;
  onRefresh: () => void;
  onRefreshCatalog: () => void;
  onRefreshInventory: () => void;
}

function isTerminal(state: number): boolean {
  return state === 6 || state === 7 || state === 8 || state === 9;
}

function jobStateLabel(state: number): string {
  switch (state) {
    case 1:
      return "Queued";
    case 2:
      return "Dispatched";
    case 3:
      return "Accepted";
    case 4:
      return "Running";
    case 5:
      return "Cancelling";
    case 6:
      return "Installed";
    case 7:
      return "Failed";
    case 8:
      return "Cancelled";
    case 9:
      return "Expired";
    default:
      return "Unknown";
  }
}

function platformOf(value: string): string {
  return value === "macOS" ? "macos" : value.toLowerCase();
}

function stateFor(
  agent: AgentSummary,
  discovery: Discovery | null,
  platform: string,
  architecture: string,
): {
  label: string;
  tone: string;
  installed: boolean;
  managed: boolean;
  update: boolean;
  supported: boolean;
  installedVersion: string | null;
} {
  const found = discovery?.agents.find(
    (candidate) =>
      candidate.kind === agent.id || candidate.kind === agent.programKey,
  );
  const installed = Boolean(found);
  const managed = agent.configured !== null && agent.configured !== undefined;
  const latest = agent.versions[0]?.version ?? null;
  const installedVersion = found?.version ?? null;
  const supported =
    agent.platforms.includes(platform) &&
    (!agent.architectures.length ||
      !architecture ||
      agent.architectures.includes(architecture));
  const update = Boolean(
    installed && latest && installedVersion && installedVersion !== latest,
  );
  if (!supported) {
    return {
      label: "Unsupported on this device",
      tone: "muted",
      installed,
      managed,
      update: false,
      supported,
      installedVersion,
    };
  }
  if (update) {
    return {
      label: `Update available (${installedVersion} → ${latest})`,
      tone: "attention",
      installed,
      managed,
      update,
      supported,
      installedVersion,
    };
  }
  if (managed) {
    return {
      label: `Managed${installedVersion ? ` · ${installedVersion}` : ""}`,
      tone: "success",
      installed,
      managed,
      update,
      supported,
      installedVersion,
    };
  }
  if (installed) {
    return {
      label: `User installed${installedVersion ? ` · ${installedVersion}` : ""}`,
      tone: "neutral",
      installed,
      managed,
      update,
      supported,
      installedVersion,
    };
  }
  return {
    label: "Not installed",
    tone: "muted",
    installed,
    managed,
    update,
    supported,
    installedVersion,
  };
}

export function AgentsView({
  agents,
  discovery,
  jobs,
  sync,
  platform,
  architecture,
  loading,
  error,
  notice,
  jobPollError,
  busyAgent,
  onStartJob,
  onDryRun,
  getConfig,
  saveConfig,
  onRefresh,
  onRefreshCatalog,
  onRefreshInventory,
}: AgentsViewProps) {
  const [configAgent, setConfigAgent] = useState<AgentSummary | null>(null);
  const [configText, setConfigText] = useState("");
  const [configRevision, setConfigRevision] = useState(0);
  const [configError, setConfigError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [configBusy, setConfigBusy] = useState(false);
  const [uninstall, setUninstall] = useState<{
    agent: AgentSummary;
    plan: DryRunPlan;
  } | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const uninstallTrigger = useRef<HTMLElement | null>(null);
  const activeJob = jobs.find((job) => !isTerminal(job.state)) ?? null;
  const recentJobs = useMemo(
    () =>
      jobs
        .filter((job) => isTerminal(job.state))
        .slice(-3)
        .reverse(),
    [jobs],
  );
  const normalizedPlatform = platformOf(platform);

  useEffect(() => {
    if (!uninstall) return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const trigger = uninstallTrigger.current;
    confirmRef.current?.focus();

    const focusable = () =>
      Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setUninstall(null);
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusable();
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !dialog.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (active === last || !dialog.contains(active))
      ) {
        event.preventDefault();
        first.focus();
      }
    };

    const onFocusIn = (event: FocusEvent) => {
      if (!dialog.contains(event.target as Node)) {
        event.stopPropagation();
        confirmRef.current?.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusin", onFocusIn, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("focusin", onFocusIn, true);
      const triggerUsable =
        trigger?.isConnected &&
        !(trigger instanceof HTMLButtonElement && trigger.disabled);
      const target = triggerUsable
        ? trigger
        : document.querySelector<HTMLElement>(".page-heading button");
      target?.focus();
    };
  }, [uninstall]);

  async function openConfig(agent: AgentSummary) {
    setConfigAgent(agent);
    setConfigError(null);
    setConfigBusy(true);
    try {
      const response = await getConfig(agent.id);
      setConfigText(JSON.stringify(response.config ?? {}, null, 2));
      setConfigRevision(response.revision);
    } catch (reason) {
      setConfigError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setConfigBusy(false);
    }
  }

  async function saveConfigNow() {
    if (!configAgent) return;
    setConfigError(null);
    let parsed: unknown;
    try {
      parsed = JSON.parse(configText);
    } catch {
      setConfigError("Configuration must be valid JSON.");
      return;
    }
    setConfigBusy(true);
    try {
      const response = await saveConfig(configAgent.id, configRevision, parsed);
      setConfigRevision(response.revision);
      setConfigText(JSON.stringify(response.config ?? {}, null, 2));
    } catch (reason) {
      setConfigError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setConfigBusy(false);
    }
  }

  async function askUninstall(agent: AgentSummary) {
    const version = agent.versions[0]?.version;
    if (!version) return;
    setActionError(null);
    try {
      const plan = await onDryRun(agent.id, version);
      uninstallTrigger.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      setUninstall({ agent, plan });
    } catch (reason) {
      setActionError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  return (
    <div className="page-stack">
      <div className="page-heading">
        <div>
          <h2>Managed agents</h2>
          <p>
            Install, update, and configure the coding agents approved in this
            device's catalog.
          </p>
        </div>
        <div className="agent-actions">
          <button type="button" onClick={onRefresh} disabled={loading}>
            <RefreshCw size={16} /> Refresh
          </button>
          <button type="button" onClick={onRefreshCatalog}>
            Sync catalog
          </button>
          <button type="button" onClick={onRefreshInventory}>
            Refresh inventory
          </button>
        </div>
      </div>

      <section className="card">
        <CardHeader
          title="Catalog sync"
          description={
            sync?.catalog
              ? `Revision ${sync.catalog.revision} · last sync ${new Date(
                  sync.catalog.lastSyncUnixSeconds * 1000,
                ).toLocaleString()}${sync.catalog.stale ? " · stale" : ""}`
              : "No catalog has been synced yet."
          }
        />
        <p className="agent-muted">
          Local configuration revision: {sync?.localConfigRevision ?? 0}
        </p>
      </section>

      {activeJob || recentJobs.length ? (
        <section className="card" aria-live="polite" aria-atomic="true">
          <CardHeader
            title="Agent jobs"
            description={
              activeJob
                ? `${jobStateLabel(activeJob.state)} · ${activeJob.progressPercent}%`
                : "Recent job outcomes"
            }
          />
          <ul className="agent-job-list">
            {(activeJob ? [activeJob] : recentJobs).map((job) => (
              <li
                key={job.jobId}
                className={`agent-job agent-job-${job.state}`}
              >
                <span>{jobStateLabel(job.state)}</span>
                {job.errorCode ? <code>{job.errorCode}</code> : null}
                {job.errorMessage ? (
                  <span>{redactPaths(job.errorMessage)}</span>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {error || actionError ? (
        <div className="notice notice-error" role="alert">
          <AlertCircle size={16} /> {redactPaths(error ?? actionError ?? "")}
        </div>
      ) : notice ? (
        <div className={`notice notice-${notice.tone}`} role="status">
          {redactPaths(notice.message)}
        </div>
      ) : null}

      {jobPollError ? (
        <div className="notice notice-error" role="status">
          <AlertCircle size={16} /> Job updates paused:{" "}
          {redactPaths(jobPollError)}
        </div>
      ) : null}

      <section className="card table-card">
        <CardHeader
          title="Approved agents"
          description={
            agents ? `${agents.length} catalog entries` : "Loading catalog…"
          }
        />
        {loading && !agents ? (
          <div className="empty-inline">
            <LoaderCircle size={20} className="spin" />
            <div>
              <strong>Loading catalog</strong>
              <span>Reading the signed catalog from the daemon.</span>
            </div>
          </div>
        ) : agents?.length ? (
          <div className="agent-list">
            {agents.map((agent) => {
              const state = stateFor(
                agent,
                discovery,
                normalizedPlatform,
                architecture,
              );
              const latest = agent.versions[0]?.version ?? "";
              const busy = busyAgent === agent.id || Boolean(activeJob);
              return (
                <article key={agent.id} className="agent-row">
                  <div className="agent-row-main">
                    <div>
                      <h3>{agent.displayName}</h3>
                      <p className="agent-muted">
                        {agent.id} · {agent.programKey}
                      </p>
                    </div>
                    <span className={`agent-state agent-state-${state.tone}`}>
                      {state.tone === "success" ? (
                        <CheckCircle2 size={14} />
                      ) : null}
                      {state.label}
                    </span>
                  </div>
                  <div className="agent-row-actions">
                    {!state.supported ? (
                      <button type="button" disabled>
                        Unsupported
                      </button>
                    ) : state.update ? (
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onStartJob(agent.id, "update", latest)}
                      >
                        <Upload size={16} /> Update
                      </button>
                    ) : !state.installed ? (
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onStartJob(agent.id, "install", latest)}
                      >
                        <Download size={16} /> Install
                      </button>
                    ) : null}
                    {state.managed ? (
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => void askUninstall(agent)}
                      >
                        <Trash2 size={16} /> Uninstall
                      </button>
                    ) : null}
                    <button
                      type="button"
                      disabled={configBusy}
                      onClick={() => void openConfig(agent)}
                    >
                      <Settings2 size={16} /> Configure
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        ) : (
          <div className="empty-inline">
            <AlertCircle size={20} />
            <div>
              <strong>No catalog entries</strong>
              <span>Sync the catalog or check the daemon connection.</span>
            </div>
          </div>
        )}
      </section>

      {configAgent ? (
        <section className="card">
          <CardHeader
            title={`Configure ${configAgent.displayName}`}
            description={`Local configuration revision ${configRevision}. Secret-bearing keys are rejected by the daemon.`}
            action={
              <button type="button" onClick={() => setConfigAgent(null)}>
                Close
              </button>
            }
          />
          <label className="agent-config-label" htmlFor="agent-config">
            JSON configuration
          </label>
          <textarea
            id="agent-config"
            className="agent-config"
            value={configText}
            onChange={(event) => setConfigText(event.target.value)}
            spellCheck={false}
          />
          {configError ? (
            <p className="notice notice-error" role="alert">
              {redactPaths(configError)}
            </p>
          ) : null}
          <button
            type="button"
            disabled={configBusy}
            onClick={() => void saveConfigNow()}
          >
            {configBusy ? "Saving…" : "Save configuration"}
          </button>
        </section>
      ) : null}

      {uninstall ? (
        <div className="agent-modal-backdrop" role="presentation">
          <div
            ref={dialogRef}
            className="agent-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="uninstall-title"
            aria-describedby="uninstall-description"
          >
            <h3 id="uninstall-title">
              Uninstall {uninstall.agent.displayName}?
            </h3>
            <p id="uninstall-description">
              The daemon will remove these managed paths:
            </p>
            <ul>
              {uninstall.plan.paths.map((path) => (
                <li key={path}>
                  <code>{path}</code>
                </li>
              ))}
            </ul>
            <div className="agent-modal-actions">
              <button type="button" onClick={() => setUninstall(null)}>
                Cancel
              </button>
              <button
                ref={confirmRef}
                type="button"
                className="danger"
                onClick={() => {
                  const version = uninstall.agent.versions[0]?.version;
                  if (version)
                    onStartJob(uninstall.agent.id, "uninstall", version);
                  setUninstall(null);
                }}
              >
                Uninstall
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
