import { invoke } from "@tauri-apps/api/core";

import type {
  AgentAction,
  AgentConfigResponse,
  AgentJob,
  AgentsResponse,
  Bootstrap,
  CatalogResponse,
  ChatActionResult,
  ChatContext,
  ChatMessage,
  ChatProviderInfo,
  ChatReply,
  ConnectorSnapshot,
  DeviceUsage,
  Discovery,
  DryRunPlan,
  JobsResponse,
  ManagedDeviceSnapshot,
  ProgramChoice,
  ProgramManagement,
  Settings,
  SyncStatus,
  UsageRange,
} from "./types";

export async function desktopReady(): Promise<void> {
  return invoke<void>("desktop_ready");
}

export async function getBootstrap(): Promise<Bootstrap> {
  return invoke<Bootstrap>("get_bootstrap");
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("save_settings", { settings });
}

export async function getConnectorStatus(): Promise<ConnectorSnapshot> {
  return invoke<ConnectorSnapshot>("get_connector_status");
}

export async function getManagedDeviceStatus(): Promise<ManagedDeviceSnapshot> {
  return invoke<ManagedDeviceSnapshot>("get_managed_device_status");
}

export async function getDiscovery(): Promise<Discovery> {
  return invoke<Discovery>("get_discovery");
}

export async function getDeviceUsage(
  range: UsageRange,
  utcOffsetMinutes: number,
): Promise<DeviceUsage> {
  return invoke<DeviceUsage>("get_device_usage", { range, utcOffsetMinutes });
}

export async function logoutManagedDevice(): Promise<void> {
  return invoke<void>("logout_managed_device");
}

export async function getProgramManagement(): Promise<ProgramManagement> {
  return invoke<ProgramManagement>("get_program_management");
}

export async function setProgramChoice(
  choice: ProgramChoice,
): Promise<ProgramManagement> {
  return invoke<ProgramManagement>("set_program_choice", { choice });
}

export async function setupManagedDevice(): Promise<ManagedDeviceSnapshot> {
  return invoke<ManagedDeviceSnapshot>("setup_managed_device");
}

export async function getCatalog(): Promise<CatalogResponse> {
  return invoke<CatalogResponse>("get_catalog");
}

export async function getAgents(): Promise<AgentsResponse> {
  return invoke<AgentsResponse>("get_agents");
}

export async function getJobs(): Promise<JobsResponse> {
  return invoke<JobsResponse>("get_jobs");
}

export async function getJob(jobId: string): Promise<AgentJob> {
  return invoke<AgentJob>("get_job", { jobId });
}

export async function startAgentJob(
  agentId: string,
  action: AgentAction,
  version: string,
  dryRun = false,
): Promise<AgentJob | DryRunPlan> {
  return invoke<AgentJob | DryRunPlan>("start_agent_job", {
    agentId,
    action,
    version,
    dryRun,
  });
}

export async function getAgentConfig(
  agentId: string,
): Promise<AgentConfigResponse> {
  return invoke<AgentConfigResponse>("get_agent_config", { agentId });
}

export async function saveAgentConfig(
  agentId: string,
  revision: number,
  config: unknown,
): Promise<AgentConfigResponse> {
  return invoke<AgentConfigResponse>("save_agent_config", {
    agentId,
    revision,
    config,
  });
}

export async function getChatProviders(): Promise<ChatProviderInfo[]> {
  return invoke<ChatProviderInfo[]>("get_chat_providers");
}

export async function getChatContext(
  jobId: string | null,
): Promise<ChatContext> {
  return invoke<ChatContext>("get_chat_context", { jobId });
}

export async function startChat(
  requestId: string,
  providerId: string,
  jobId: string | null,
  message: string,
  history: Array<Pick<ChatMessage, "role" | "content">>,
): Promise<ChatReply> {
  return invoke<ChatReply>("start_chat", {
    requestId,
    providerId,
    jobId,
    message,
    history,
  });
}

export async function cancelChat(requestId: string): Promise<void> {
  return invoke<void>("cancel_chat", { requestId });
}

export async function executeChatAction(
  actionId: string,
  jobId: string,
  provider: string,
): Promise<ChatActionResult> {
  return invoke<ChatActionResult>("execute_chat_action", {
    actionId,
    jobId,
    provider,
  });
}

export async function getSyncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>("get_sync_status");
}

export async function refreshCatalog(): Promise<void> {
  return invoke<void>("refresh_catalog");
}

export async function refreshInventory(): Promise<void> {
  return invoke<void>("refresh_inventory");
}
