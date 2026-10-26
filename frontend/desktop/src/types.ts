import type {
  ColorMode,
  UsageBucket,
  UsageGroup,
  UsageTotals,
} from "@agentdesktop/ui";

export interface Settings {
  openOnStartup: boolean;
  colorMode: ColorMode;
}

export interface Bootstrap {
  settings: Settings;
  version: string;
  platform: string;
  architecture: string;
}

export interface PlatformCapabilities {
  os: string;
}

/**
 * The daemon's own live connection to the controller, independent of local
 * process health. `null` for a standalone (unmanaged) daemon, which has no
 * controller to connect to.
 */
export interface ControllerConnectionStatus {
  connected: boolean;
  /** Last time the controller stream was observed alive. */
  lastSeenUnixSeconds?: number;
  lastError?: ControllerConnectionError;
}

/**
 * Coarse reason for the most recent controller failure. The daemon keeps the
 * full error chain in its logs and never returns it to the local API.
 */
export type ControllerConnectionError =
  | "unreachable"
  | "identityRejected"
  | "sessionExpired"
  | "localError"
  | "unknown";

export interface DaemonInfo {
  version: string;
  scope: "user" | "system";
  configPath: string;
  stateDirectory: string;
  inventoryInterval: string;
  controller: {
    address: string;
    caCertificatePath: string | null;
    heartbeatInterval: string;
  } | null;
  llmProxy?: {
    /** Bound address when `bound`, configured address otherwise. */
    listen: string;
    bound: boolean;
    clientId: string;
    error?: string;
  };
  /** Receiver for usage from a gateway on this machine (`daemon.usage`). */
  usageReceiver?: {
    /** Bound address when `bound`, configured address otherwise. */
    listen: string;
    bound: boolean;
    error?: string;
  };
}

export interface ConnectorRuntime {
  mode: string;
  gateway: string;
  platform: PlatformCapabilities;
  daemon: DaemonInfo;
  controller: ControllerConnectionStatus | null;
}

export interface ConnectorSnapshot {
  state: "ready" | "attention" | "offline";
  detail: string | null;
  runtime: ConnectorRuntime | null;
}

export interface ManagedDeviceSnapshot {
  configured: boolean;
  organizationName: string | null;
  enrollment: string;
  detail: string | null;
}

export interface McpServer {
  name: string;
  transport: string;
  command?: string;
  url?: string;
  enabled: boolean;
  source: string;
}

export interface Skill {
  path: string;
  frontMatter: Record<string, unknown>;
}

export interface DiscoveredAgent {
  kind: string;
  executable: string;
  version: string | null;
  mcpServers?: McpServer[];
  skills?: Skill[];
}

export interface Discovery {
  agents: DiscoveredAgent[];
  modelRuntimes?: Array<{
    kind: string;
    models: Array<{ name: string }>;
  }>;
}

export type UsageRange = "24h" | "7d" | "30d";

/** LLM gateway usage recorded for this device by its controller or daemon. */
export interface DeviceUsage {
  /** False when the controller does not collect usage. */
  enabled: boolean;
  fromUnixSeconds: number;
  toUnixSeconds: number;
  bucketSeconds: number;
  totals: UsageTotals;
  buckets: UsageBucket[];
  byTool: UsageGroup[];
  byModel: UsageGroup[];
  /** Configured tools whose gateway traffic is not metered. */
  unmeteredTools?: string[];
}

/**
 * The configured programs the device user wants Agent Desktop to manage,
 * honored only while the policy allows a user choice. Program IDs are
 * provider IDs such as `claude-code`.
 */
export type ProgramChoice =
  | { mode: "all" }
  | { mode: "none" }
  | { mode: "selected"; programs: string[] };

/** One configured program and its last reconcile outcome. */
export interface ManagedProgram {
  /** Provider ID, for example `claude-code`. */
  program: string;
  /** False when the device user opted out of this program. */
  managed: boolean;
  /**
   * Outcome of the last apply (`applied`, `unchanged`, `opted_out`, …);
   * absent until the program has been applied.
   */
  state?: string;
  detail?: string;
}

/**
 * Which configured programs are managed on this device, and whether the
 * device user may change that.
 */
export interface ProgramManagement {
  userChoiceAllowed: boolean;
  /** The stored choice; ignored while `userChoiceAllowed` is false. */
  choice: ProgramChoice;
  /** Configured programs in reconcile order; empty before a configuration. */
  programs: ManagedProgram[];
}

/** One installable version in the signed agent catalog. */
export interface CatalogVersion {
  version: string;
  artifactUrl: string;
  sha256: string;
  signature: string;
}

export interface CatalogEntry {
  id: string;
  displayName: string;
  programKey: string;
  platforms: string[];
  architectures: string[];
  installKind: "user";
  versions: CatalogVersion[];
  detection: { paths: string[] };
}

export interface CatalogResponse {
  revision: number;
  sha256: string;
  entries: CatalogEntry[];
}

/** An agent from the catalog with the device's local configuration state. */
export interface AgentSummary extends CatalogEntry {
  configured: unknown;
}

export interface AgentsResponse {
  revision: number;
  agents: AgentSummary[];
}

/** Proto JobState numeric values (see fleet.proto). */
export type AgentJobState = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;

export interface AgentJob {
  jobId: string;
  attempt: number;
  state: AgentJobState;
  progressPercent: number;
  errorCode: string;
  errorMessage: string;
  diagnosticsJson: string;
}

export interface JobsResponse {
  jobs: AgentJob[];
}

export interface CatalogSyncStatus {
  revision: number;
  sha256: string;
  lastSyncUnixSeconds: number;
  stale: boolean;
}

export interface SyncStatus {
  catalog: CatalogSyncStatus | null;
  localConfigRevision: number;
}

export interface AgentConfigResponse {
  id: string;
  programKey: string;
  revision: number;
  config: unknown;
}

export type AgentAction = "install" | "update" | "uninstall";

/** Response to a dry-run uninstall job: paths that would be removed. */
export interface DryRunPlan {
  dryRun: true;
  paths: string[];
}

/** A discovered coding agent whose existing model configuration can answer chat. */
export interface ChatProviderInfo {
  id: string;
  displayName: string;
  available: boolean;
  readOnlyMode: string;
  unavailableReason: string | null;
}

export interface ChatAction {
  id: string;
  label: string;
  description: string;
}

export interface ChatPlaybook {
  errorCode: string;
  explanation: string;
  actions: ChatAction[];
}

export interface ChatJobSummary {
  jobId: string;
  agentId: string;
  version: string;
  action: string;
  state: number;
  errorCode: string;
  errorMessage: string;
  diagnostics: string;
  catalogRevision: number | null;
}

export interface ChatAgentSummary {
  id: string;
  displayName: string;
  configured: boolean;
}

/** Redacted data that may be sent to the selected local provider. */
export interface ChatContext {
  job: ChatJobSummary | null;
  recentJobs: ChatJobSummary[];
  agents: ChatAgentSummary[];
  catalogRevision: number | null;
  catalogStale: boolean;
  localConfigRevision: number;
  warnings: string[];
  preview: string;
  playbook: ChatPlaybook;
}

export type ChatRole = "user" | "assistant";

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  actions?: ChatAction[];
  tone?: "normal" | "error";
}

export interface ChatReply {
  message: string;
  proposedActions: ChatAction[];
  provider: string;
  ruleBased: boolean;
  sessionId: string;
}

export interface ChatActionResult {
  actionId: string;
  outcome: string;
  detail: string;
  navigate: string | null;
}
