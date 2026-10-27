import { Search } from "lucide-react";
import { useMemo, useState } from "react";

import { DeviceTable, EmptyDevices } from "../components/DeviceTable";
import { ErrorState, PageSkeleton } from "../components/ViewStates";
import type { Device, DeviceState } from "../types";

type StateFilter = DeviceState | "all";

export function DevicesView({
  devices,
  error = null,
  loading = false,
}: {
  devices: Device[];
  error?: string | null;
  loading?: boolean;
}) {
  const [search, setSearch] = useState("");
  const [stateFilter, setStateFilter] = useState<StateFilter>("all");
  const filtered = useMemo(
    () =>
      devices.filter(
        (device) =>
          (stateFilter === "all" || device.state === stateFilter) &&
          `${device.hostname} ${device.os} ${device.agent_version}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      ),
    [devices, search, stateFilter],
  );

  return (
    <div className="stack">
      <section className="section-intro">
        <div>
          <h2>Managed devices</h2>
          <p>
            Inventory, connectivity, and rollout state for every enrolled
            machine.
          </p>
        </div>
        <div className="device-filters">
          <label className="filter-select">
            <span className="sr-only">Filter by lifecycle state</span>
            <select
              aria-label="Filter by lifecycle state"
              value={stateFilter}
              onChange={(event) =>
                setStateFilter(event.target.value as StateFilter)
              }
            >
              <option value="all">All lifecycle states</option>
              <option value="active">Active</option>
              <option value="retired">Retired</option>
              <option value="revoked">Revoked</option>
            </select>
          </label>
          <div className="search-box">
            <Search size={16} />
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search devices…"
              aria-label="Search devices"
            />
          </div>
        </div>
      </section>
      <section className="card table-card">
        {loading ? (
          <PageSkeleton rows={5} />
        ) : error ? (
          <ErrorState message={error} />
        ) : filtered.length ? (
          <DeviceTable devices={filtered} />
        ) : (
          <EmptyDevices searching={Boolean(search) || stateFilter !== "all"} />
        )}
      </section>
    </div>
  );
}
