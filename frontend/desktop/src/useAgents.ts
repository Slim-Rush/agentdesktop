import { useCallback, useEffect, useRef, useState } from "react";

import {
  getAgentConfig,
  getAgents,
  getJobs,
  getSyncStatus,
  refreshCatalog as requestCatalogRefresh,
  refreshInventory as requestInventoryRefresh,
  saveAgentConfig,
  startAgentJob,
} from "./backend";
import type {
  AgentAction,
  AgentJob,
  AgentSummary,
  DryRunPlan,
  SyncStatus,
} from "./types";

type Notice = { tone: "success" | "error"; message: string } | null;

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function terminal(state: number): boolean {
  return state === 6 || state === 7 || state === 8 || state === 9;
}

export function useAgents(enabled: boolean) {
  const [agents, setAgents] = useState<AgentSummary[] | null>(null);
  const [jobs, setJobs] = useState<AgentJob[]>([]);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [busyAgent, setBusyAgent] = useState<string | null>(null);
  const generation = useRef(0);

  const load = useCallback(async () => {
    const current = ++generation.current;
    setLoading(true);
    setError(null);
    try {
      const [nextAgents, nextJobs, nextSync] = await Promise.all([
        getAgents(),
        getJobs(),
        getSyncStatus(),
      ]);
      if (current !== generation.current) return;
      setAgents(nextAgents.agents);
      setJobs(nextJobs.jobs);
      setSync(nextSync);
    } catch (reason) {
      if (current === generation.current) setError(message(reason));
    } finally {
      if (current === generation.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (enabled) void load();
  }, [enabled, load]);

  const active = jobs.some((job) => !terminal(job.state));
  useEffect(() => {
    if (!enabled) return;
    let timer: number | undefined;
    const stop = () => {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
    };
    const tick = async () => {
      if (document.hidden) return;
      try {
        setJobs((await getJobs()).jobs);
      } catch {
        // Keep the last known job list; the next tick retries.
      }
    };
    const start = () => {
      stop();
      if (!document.hidden)
        timer = window.setInterval(tick, active ? 1000 : 5000);
    };
    start();
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        void tick();
        start();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, active]);

  const startJob = useCallback(
    async (agentId: string, action: AgentAction, version: string) => {
      setBusyAgent(agentId);
      setNotice(null);
      try {
        await startAgentJob(agentId, action, version);
        setNotice({
          tone: "success",
          message: `${action} started for ${agentId}`,
        });
        setJobs((await getJobs()).jobs);
      } catch (reason) {
        setNotice({ tone: "error", message: message(reason) });
      } finally {
        setBusyAgent(null);
      }
    },
    [],
  );

  const dryRunUninstall = useCallback(
    async (agentId: string, version: string): Promise<DryRunPlan> => {
      const plan = await startAgentJob(agentId, "uninstall", version, true);
      if (!("paths" in plan)) {
        throw new Error("daemon did not return an uninstall plan");
      }
      return plan;
    },
    [],
  );

  const refreshCatalog = useCallback(async () => {
    setNotice(null);
    try {
      await requestCatalogRefresh();
      setNotice({ tone: "success", message: "Catalog refresh requested" });
      await load();
    } catch (reason) {
      setNotice({ tone: "error", message: message(reason) });
    }
  }, [load]);

  const refreshInventory = useCallback(async () => {
    setNotice(null);
    try {
      await requestInventoryRefresh();
      setNotice({ tone: "success", message: "Inventory refresh requested" });
    } catch (reason) {
      setNotice({ tone: "error", message: message(reason) });
    }
  }, []);

  return {
    agents,
    jobs,
    sync,
    loading,
    error,
    notice,
    busyAgent,
    load,
    startJob,
    dryRunUninstall,
    getAgentConfig,
    saveAgentConfig,
    refreshCatalog,
    refreshInventory,
  };
}
