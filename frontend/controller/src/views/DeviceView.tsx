import {
  CardHeader,
  friendlyTool,
  ModelRuntimeInventory,
  ToolIcon,
  ToolInventory,
  UsageBreakdownTable,
  UsageMetrics,
  UsageNotes,
} from "@agentdesktop/ui";
import {
  Archive,
  ArrowLeft,
  Ban,
  Box,
  ChartColumn,
  ChevronRight,
  CircleAlert,
  Code2,
  Cpu,
} from "lucide-react";
import { useEffect, useRef } from "react";

import {
  ConfigBadge,
  DefinitionList,
  DeviceStateBadge,
  OnlineBadge,
} from "../components/DeviceTable";
import {
  formatDate,
  formatTime,
  formatTimeMilliseconds,
  friendlyOs,
} from "../format";
import { Link } from "../router";
import type {
  AuditPage,
  DeviceDetail,
  ProgramStatus,
  UsageReport,
} from "../types";

export interface DeviceViewProps {
  audit: AuditPage | null;
  auditError: string | null;
  auditLoadError: string | null;
  auditLoading: boolean;
  auditLoadingMore: boolean;
  device: DeviceDetail;
  onAuditLoadMore?: () => void;
  onTransitionCancel: () => void;
  onTransitionConfirm: () => void;
  onTransitionRequest: (target: "retired" | "revoked") => void;
  transitionError: string | null;
  transitionTarget: "retired" | "revoked" | null;
  transitioning: boolean;
  /** This device's last 30 days, when the controller has usage. */
  usage?: UsageReport | null;
}

export function DeviceView({
  audit,
  auditError,
  auditLoadError,
  auditLoading,
  auditLoadingMore,
  device,
  onAuditLoadMore,
  onTransitionCancel,
  onTransitionConfirm,
  onTransitionRequest,
  transitionError,
  transitionTarget,
  transitioning,
  usage,
}: DeviceViewProps) {
  const modelCount = device.model_runtimes.reduce(
    (total, runtime) => total + runtime.models.length,
    0,
  );
  return (
    <div className="stack">
      <Link href="/devices" className="back-link">
        <ArrowLeft size={15} /> Back to devices
      </Link>
      <p className="sr-only" role="status" aria-live="polite">
        Device {device.hostname} is {device.state}.
      </p>
      <section className="device-hero">
        <div className="device-identity">
          <div>
            <h2>{device.hostname}</h2>
            <div className="device-badges">
              <OnlineBadge timestamp={device.last_seen_at} />
              <DeviceStateBadge state={device.state} />
            </div>
          </div>
          <p>{device.id}</p>
        </div>
      </section>
      <div className="detail-grid">
        <section className="card detail-card">
          <CardHeader
            title="Device information"
            description="Reported by the local agent"
          />
          <DefinitionList
            entries={[
              ["Operating system", friendlyOs(device.os)],
              ["Architecture", device.architecture || "Unknown"],
              ["Agent version", device.agent_version || "Unknown"],
              ["Last seen", formatTime(device.last_seen_at)],
              ["Enrolled", formatDate(device.created_at)],
            ]}
          />
        </section>
        <section className="card detail-card">
          <CardHeader
            title="Configuration state"
            description="Latest reconciliation reported by the agent"
          />
          <div className="config-state-large">
            <ConfigBadge
              state={device.config_state}
              revision={device.config_revision}
            />
          </div>
          <DefinitionList
            entries={[
              ["Last update", formatTime(device.config_updated_at)],
              [
                "Desired revision",
                device.config_revision
                  ? `Revision ${device.config_revision}`
                  : "Not reported",
              ],
            ]}
          />
          {device.config_error && (
            <div className="error-callout">
              <CircleAlert size={16} />
              <span>{device.config_error}</span>
            </div>
          )}
        </section>
      </div>
      <section className="card detail-card device-lifecycle">
        <CardHeader
          title="Device lifecycle"
          description="Access state and identity continuity"
        />
        <div className="device-lifecycle-summary">
          <DeviceStateBadge state={device.state} />
          {device.retired_at ? (
            <span className="device-muted">
              Left active {formatTime(device.retired_at)}
            </span>
          ) : (
            <span className="device-muted">
              {device.active_devices_for_subject} active device
              {device.active_devices_for_subject === 1 ? "" : "s"} for this user
            </span>
          )}
        </div>
        <DefinitionList
          entries={[
            [
              "Active devices for this user",
              String(device.active_devices_for_subject),
            ],
            ["Replaces", device.replaces_device_id ?? "No predecessor"],
            ["Superseded by", device.superseded_by ?? "Not replaced"],
            ["Retired at", formatTime(device.retired_at)],
          ]}
        />
        <div className="device-lifecycle-actions">
          {device.state === "active" ? (
            <button
              type="button"
              className="button secondary"
              onClick={() => onTransitionRequest("retired")}
            >
              <Archive size={14} /> Retire device
            </button>
          ) : null}
          {device.state !== "revoked" ? (
            <button
              type="button"
              className="destructive-button"
              onClick={() => onTransitionRequest("revoked")}
            >
              <Ban size={14} /> Revoke device
            </button>
          ) : null}
        </div>
        <p className="device-muted">
          Retired and revoked devices lose fleet access. Their jobs, usage, and
          audit history are kept.
        </p>
      </section>
      {device.programs_reported !== null && (
        <section className="card table-card">
          <CardHeader
            title="Managed programs"
            description="Outcome of the latest apply, per program"
          />
          {!device.programs_reported ? (
            <p className="empty-state compact">
              Per-program status is not reported by this agent version
            </p>
          ) : device.programs.length ? (
            <div className="event-list">
              {device.programs.map((program) => (
                <div className="event-row" key={program.program}>
                  <span className="event-source">
                    <ToolIcon kind={program.program} />
                    <strong>{friendlyTool(program.program)}</strong>
                  </span>
                  <span className="program-state">
                    <ProgramBadge state={program.state} />
                    {program.detail && (
                      <code title={program.detail}>{program.detail}</code>
                    )}
                  </span>
                  <time>
                    {program.revision !== device.config_revision
                      ? `r${program.revision} · `
                      : ""}
                    {formatTime(program.updated_at)}
                  </time>
                </div>
              ))}
            </div>
          ) : (
            <p className="empty-state compact">No managed programs</p>
          )}
        </section>
      )}
      {usage && (usage.enabled || usage.totals.requests > 0) ? (
        <DeviceUsage deviceId={device.id} usage={usage} />
      ) : null}
      <section className="card table-card">
        <CardHeader
          title="Recent activity"
          description={`${device.recent_events.length} recent telemetry event${device.recent_events.length === 1 ? "" : "s"}`}
        />
        {device.recent_events.length ? (
          <div className="event-list">
            {device.recent_events.map((event) => (
              <div className="event-row" key={event.id}>
                <span className="event-source">
                  <ToolIcon kind={event.payload.clientId ?? ""} />
                  <span>
                    <strong>
                      {event.payload.toolName ??
                        friendlyEvent(event.event_type)}
                    </strong>
                    <small>
                      {friendlyTool(event.payload.clientId ?? "Unknown")}
                    </small>
                  </span>
                </span>
                <code title={telemetryDetail(event.payload)}>
                  {telemetryDetail(event.payload)}
                </code>
                <time
                  dateTime={new Date(event.timestamp_unix_ms).toISOString()}
                >
                  {formatTimeMilliseconds(event.timestamp_unix_ms)}
                </time>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty-inline">
            <Code2 size={20} />
            <span>No telemetry has been reported by this device.</span>
          </div>
        )}
      </section>
      <section className="card table-card">
        <CardHeader
          title="Discovered developer tools"
          description={`${device.discoveries.length} installation${device.discoveries.length === 1 ? "" : "s"} reported`}
        />
        {device.discoveries.length ? (
          <div className="tool-inventory">
            {device.discoveries.map((item) => (
              <ToolInventory
                key={`${item.kind}-${item.path}`}
                discovery={item}
              />
            ))}
          </div>
        ) : (
          <div className="empty-inline">
            <Box size={20} />
            <span>No tools have been reported by this device.</span>
          </div>
        )}
      </section>
      <section className="card table-card">
        <CardHeader
          title="Local models"
          description={`${modelCount} model${modelCount === 1 ? "" : "s"} reported`}
        />
        {device.model_runtimes.length ? (
          <div className="model-runtime-inventory">
            {device.model_runtimes.map((runtime) => (
              <ModelRuntimeInventory key={runtime.kind} runtime={runtime} />
            ))}
          </div>
        ) : (
          <div className="empty-inline">
            <Cpu size={20} />
            <span>No local models have been reported by this device.</span>
          </div>
        )}
      </section>
      <section className="card table-card">
        <CardHeader
          title="Audit history"
          description="Lifecycle and job events across this device's lineage"
        />
        {auditLoadError ? (
          <div className="error-callout">
            <CircleAlert size={16} />
            <span>{auditLoadError}</span>
          </div>
        ) : null}
        {auditLoading ? (
          <p className="empty-state compact">Loading audit history…</p>
        ) : auditError ? (
          <div className="error-callout">
            <CircleAlert size={16} />
            <span>{auditError}</span>
          </div>
        ) : audit?.events.length ? (
          <>
            <div className="audit-list">
              {audit.events.map((event) => (
                <div className="audit-row" key={event.id}>
                  <span className="audit-action">{event.action}</span>
                  <span className="audit-actor">{event.actor}</span>
                  <code title={JSON.stringify(event.payload)}>
                    {summarizePayload(event.payload)}
                  </code>
                  <time>{formatTime(event.created_at)}</time>
                </div>
              ))}
            </div>
            {audit.nextCursor && onAuditLoadMore ? (
              <button
                type="button"
                className="button secondary"
                disabled={auditLoadingMore}
                onClick={onAuditLoadMore}
              >
                {auditLoadingMore ? "Loading…" : "Load more"}
              </button>
            ) : null}
          </>
        ) : (
          <p className="empty-state compact">
            No audit events have been recorded for this device.
          </p>
        )}
      </section>
      <section className="card detail-card">
        <CardHeader
          title="Enrollment identity"
          description="Identity captured when this device joined the fleet"
        />
        <DefinitionList
          entries={[
            ["Issuer", device.enrolled_by_issuer || "Enrollment token"],
            ["Subject", device.enrolled_by_subject || "Device credential"],
          ]}
        />
      </section>
      {transitionTarget ? (
        <TransitionDeviceDialog
          hostname={device.hostname}
          target={transitionTarget}
          busy={transitioning}
          error={transitionError}
          onCancel={onTransitionCancel}
          onConfirm={onTransitionConfirm}
        />
      ) : null}
    </div>
  );
}

function DeviceUsage({
  deviceId,
  usage,
}: {
  deviceId: string;
  usage: UsageReport;
}) {
  const tools = usage.breakdowns.tool ?? [];
  const models = usage.breakdowns.model ?? [];
  return (
    <section className="card table-card device-usage">
      <CardHeader
        title="Usage"
        description="Last 30 days"
        action={
          <Link
            href={`/usage?${new URLSearchParams({ range: "30d", device: deviceId })}`}
            className="text-link"
          >
            View in Usage <ChevronRight size={14} />
          </Link>
        }
      />
      {usage.totals.requests ? (
        <>
          <UsageMetrics totals={usage.totals} />
          <UsageNotes totals={usage.totals} />
          <UsageBreakdownTable
            dimension="tool"
            label="Usage by tool"
            rows={tools}
          />
          <UsageBreakdownTable
            dimension="model"
            label="Usage by model"
            rows={models}
          />
        </>
      ) : (
        <div className="empty-inline">
          <ChartColumn size={20} />
          <span>No LLM gateway usage in the last 30 days.</span>
        </div>
      )}
    </section>
  );
}

function TransitionDeviceDialog({
  hostname,
  target,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  hostname: string;
  target: "retired" | "revoked";
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    dialog.current?.showModal();
    confirm.current?.focus();
    return () => dialog.current?.close();
  }, []);
  const retiring = target === "retired";
  return (
    <dialog
      ref={dialog}
      className="delete-dialog"
      aria-labelledby="transition-device-title"
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
    >
      <div className="dialog-body">
        <h2 id="transition-device-title">
          {retiring ? "Retire" : "Revoke"} {hostname || "this device"}?
        </h2>
        <p>
          {retiring
            ? "The device loses fleet access immediately. It can be re-enrolled with a new device record, and its jobs, usage, and audit history stay available."
            : "The device is permanently revoked. It cannot be auto-linked by a re-enrollment or reinstated from this screen; its history is kept."}
        </p>
        {error && <div className="dialog-error">{error}</div>}
      </div>
      <div className="dialog-actions">
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          ref={confirm}
          type="button"
          className="destructive-button"
          disabled={busy}
          onClick={onConfirm}
        >
          {busy
            ? retiring
              ? "Retiring…"
              : "Revoking…"
            : retiring
              ? "Retire device"
              : "Revoke device"}
        </button>
      </div>
    </dialog>
  );
}

function summarizePayload(payload: Record<string, unknown>) {
  const encoded = JSON.stringify(payload);
  return encoded.length > 120 ? `${encoded.slice(0, 117)}…` : encoded;
}

function friendlyEvent(kind: string) {
  const names: Record<string, string> = {
    "session.new": "New session",
    "tool.use": "Tool use",
    sessionNew: "New session",
    toolUse: "Tool use",
  };
  return names[kind] ?? kind;
}

function telemetryDetail(
  payload: DeviceDetail["recent_events"][number]["payload"],
) {
  if (payload.sessionId) return payload.sessionId;
  const value = payload.toolInput;
  if (value === undefined) return "No input reported";
  const encoded = JSON.stringify(value);
  return encoded.length > 180 ? `${encoded.slice(0, 177)}…` : encoded;
}

const programStateLabels: Record<ProgramStatus["state"], string> = {
  applied: "Applied",
  unchanged: "Unchanged",
  removed: "Removed",
  conflict: "Conflict",
  inactive: "Inactive",
  blocked: "Blocked",
  opted_out: "Opted out",
  failed: "Failed",
  unspecified: "Unknown",
};

const programStateDescriptions: Partial<
  Record<ProgramStatus["state"], string>
> = {
  opted_out: "The device user opted out of this program",
};

function ProgramBadge({ state }: { state: ProgramStatus["state"] }) {
  const tone =
    state === "conflict" || state === "failed" || state === "blocked"
      ? "danger"
      : state === "inactive"
        ? "warning"
        : "neutral";
  return (
    <span className={`badge ${tone}`} title={programStateDescriptions[state]}>
      {programStateLabels[state] ?? state}
    </span>
  );
}
