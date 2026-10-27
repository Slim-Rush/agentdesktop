import type { UsageDimension } from "@agentdesktop/ui";
import { useEffect, useState } from "react";

import { adminWrite, useApi } from "./api";
import { ControllerShell } from "./components/ControllerShell";
import { ErrorState, NotFound, PageSkeleton } from "./components/ViewStates";
import { usePath } from "./router";
import type {
  ActiveDaemonConfig,
  AuditEvent,
  AuditPage,
  ControllerSettings,
  Device,
  DeviceDetail,
  Overview,
  UsageRange,
  UsageReport,
} from "./types";
import { type UsageFilters, usageDimensions } from "./usage";
import { ConfigurationView } from "./views/ConfigurationView";
import { DevicesView } from "./views/DevicesView";
import { DeviceView } from "./views/DeviceView";
import { OverviewView } from "./views/OverviewView";
import { SettingsView } from "./views/SettingsView";
import { UsageView } from "./views/UsageView";

export function App() {
  const path = usePath();

  return (
    <ControllerShell path={path} onRefresh={() => window.location.reload()}>
      <ControllerRoute path={path} />
    </ControllerShell>
  );
}

function ControllerRoute({ path }: { path: string }) {
  if (path === "/") return <OverviewPage />;
  if (path === "/devices") return <DevicesPage />;
  if (path.startsWith("/devices/")) {
    const id = decodeURIComponent(path.slice(9));
    // A new device starts without the previous device's data.
    return <DevicePage key={id} id={id} />;
  }
  if (path === "/configuration") return <ConfigurationPage />;
  if (path === "/usage") return <UsagePage />;
  if (path === "/settings") return <SettingsPage />;
  return <NotFound />;
}

function OverviewPage() {
  const query = useApi<Overview>("/api/v1/overview");
  if (query.loading) return <PageSkeleton />;
  if (query.error || !query.data) return <ErrorState message={query.error} />;
  return <OverviewView data={query.data} />;
}

function DevicesPage() {
  const query = useApi<Device[]>("/api/v1/devices");
  return (
    <DevicesView
      devices={query.data ?? []}
      error={query.error}
      loading={query.loading}
    />
  );
}

function DevicePage({ id }: { id: string }) {
  const query = useApi<DeviceDetail>(
    `/api/v1/devices/${encodeURIComponent(id)}`,
  );
  const audit = useApi<AuditPage>(
    `/api/v1/devices/${encodeURIComponent(id)}/audit?limit=50`,
  );
  const usage = useApi<UsageReport>(
    `/api/v1/usage?${new URLSearchParams({
      range: "30d",
      by: "tool,model",
      device: id,
      utcOffsetMinutes: String(utcOffsetMinutes()),
    })}`,
  );
  const [transitionTarget, setTransitionTarget] = useState<
    "retired" | "revoked" | null
  >(null);
  const [transitioning, setTransitioning] = useState(false);
  const [transitionError, setTransitionError] = useState<string | null>(null);
  const [moreAudit, setMoreAudit] = useState<AuditEvent[]>([]);
  const [auditCursor, setAuditCursor] = useState<string | null>(null);
  const [auditLoadingMore, setAuditLoadingMore] = useState(false);
  const [auditLoadError, setAuditLoadError] = useState<string | null>(null);

  useEffect(() => {
    setMoreAudit([]);
    setAuditCursor(audit.data?.nextCursor ?? null);
    setAuditLoadError(null);
  }, [audit.data]);

  async function transitionDevice() {
    if (!transitionTarget) return;
    setTransitioning(true);
    setTransitionError(null);
    try {
      const action = transitionTarget === "retired" ? "retire" : "revoke";
      const response = await adminWrite(
        `/api/v1/devices/${encodeURIComponent(id)}/${action}`,
        { method: "POST" },
      );
      if (!response.ok) throw new Error(await response.text());
      setTransitionTarget(null);
      query.reload();
      audit.reload();
    } catch (error) {
      setTransitionError(
        error instanceof Error ? error.message : "Lifecycle change failed",
      );
    } finally {
      setTransitioning(false);
    }
  }

  async function loadMoreAudit() {
    if (!auditCursor) return;
    setAuditLoadingMore(true);
    setAuditLoadError(null);
    try {
      const response = await fetch(
        `/api/v1/devices/${encodeURIComponent(id)}/audit?limit=50&before=${encodeURIComponent(auditCursor)}`,
      );
      if (!response.ok) throw new Error(await response.text());
      const page = (await response.json()) as AuditPage;
      setMoreAudit((events) => [...events, ...page.events]);
      setAuditCursor(page.nextCursor);
    } catch (error) {
      setAuditLoadError(
        error instanceof Error ? error.message : "Audit history failed",
      );
    } finally {
      setAuditLoadingMore(false);
    }
  }

  if (query.loading) return <PageSkeleton />;
  if (query.error || !query.data) return <ErrorState message={query.error} />;
  return (
    <DeviceView
      audit={
        audit.data
          ? {
              events: [...audit.data.events, ...moreAudit],
              nextCursor: auditCursor,
            }
          : null
      }
      auditError={audit.error}
      auditLoadError={auditLoadError}
      auditLoading={audit.loading}
      auditLoadingMore={auditLoadingMore}
      device={query.data}
      onAuditLoadMore={auditCursor ? loadMoreAudit : undefined}
      onTransitionCancel={() => {
        if (!transitioning) {
          setTransitionTarget(null);
          setTransitionError(null);
        }
      }}
      onTransitionConfirm={transitionDevice}
      onTransitionRequest={(target) => {
        setTransitionError(null);
        setTransitionTarget(target);
      }}
      transitionError={transitionError}
      transitionTarget={transitionTarget}
      transitioning={transitioning}
      usage={usage.data}
    />
  );
}

function utcOffsetMinutes() {
  return -new Date().getTimezoneOffset();
}

const usageRanges: UsageRange[] = ["24h", "7d", "30d", "90d"];

type UsageSelection = {
  dimension: UsageDimension;
  filters: UsageFilters;
  range: UsageRange;
};

/** The page's selection lives in the query string so it can be linked. */
function readUsageSelection(search: string): UsageSelection {
  const params = new URLSearchParams(search);
  const range = params.get("range");
  const group = params.get("group");
  const filters: UsageFilters = {};
  for (const { value } of usageDimensions) {
    const filter = params.get(value);
    if (filter !== null) filters[value] = filter;
  }
  return {
    dimension:
      usageDimensions.find((option) => option.value === group)?.value ?? "user",
    filters,
    range: usageRanges.find((option) => option === range) ?? "7d",
  };
}

function withFilters(params: URLSearchParams, filters: UsageFilters) {
  for (const [dimension, value] of Object.entries(filters)) {
    params.set(dimension, value);
  }
  return params;
}

function UsagePage() {
  const [selection, setSelection] = useState(() =>
    readUsageSelection(window.location.search),
  );
  const { dimension, filters, range } = selection;
  const pageSearch = withFilters(
    new URLSearchParams({ range, group: dimension }),
    filters,
  ).toString();
  const query = useApi<UsageReport>(
    `/api/v1/usage?${withFilters(
      new URLSearchParams({
        range,
        by: usageDimensions.map(({ value }) => value).join(","),
        utcOffsetMinutes: String(utcOffsetMinutes()),
      }),
      filters,
    )}`,
  );

  useEffect(() => {
    window.history.replaceState(null, "", `/usage?${pageSearch}`);
  }, [pageSearch]);

  return (
    <UsageView
      dimension={dimension}
      error={query.error}
      filters={filters}
      loading={query.loading}
      onDimensionChange={(next) =>
        setSelection((current) => ({ ...current, dimension: next }))
      }
      onFiltersChange={(next) =>
        setSelection((current) => ({ ...current, filters: next }))
      }
      onRangeChange={(next) =>
        setSelection((current) => ({ ...current, range: next }))
      }
      range={range}
      report={query.data}
    />
  );
}

function ConfigurationPage() {
  const query = useApi<ActiveDaemonConfig>("/api/v1/daemon-config");
  return <ConfigurationView initialConfig={query.data?.config} />;
}

function SettingsPage() {
  const query = useApi<ControllerSettings>("/api/v1/settings");
  return (
    <SettingsView
      data={query.data}
      loading={query.loading}
      error={query.error}
    />
  );
}
