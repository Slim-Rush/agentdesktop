import { ThemeProvider } from "@agentdesktop/ui";

import { DesktopShell } from "./components/DesktopShell";
import { PageBoundary } from "./components/PageBoundary";
import { StatusLoading } from "./components/StatusLoading";
import { useAgents } from "./useAgents";
import { useDesktopModel } from "./useDesktopModel";
import { useDeviceUsage } from "./useDeviceUsage";
import { AgentsView } from "./views/AgentsView";
import { EnrollmentView } from "./views/EnrollmentView";
import { SettingsView } from "./views/SettingsView";
import { StatusView } from "./views/StatusView";
import { ToolsView } from "./views/ToolsView";
import { UsageView } from "./views/UsageView";

export function Desktop() {
  const model = useDesktopModel();
  const agents = useAgents(model.view === "agents");
  const showingUsage =
    model.view === "usage" && model.showUsage && !model.needsEnrollment;
  const deviceUsage = useDeviceUsage(showingUsage);

  return (
    <ThemeProvider
      mode={model.settings.colorMode}
      onModeChange={model.setColorMode}
    >
      <DesktopShell
        fullWidth={model.needsEnrollment && model.view !== "settings"}
        isRefreshing={model.isRefreshing}
        notice={model.notice}
        onNavigate={model.navigate}
        onRefresh={() => {
          model.refresh();
          if (showingUsage) deviceUsage.reload();
        }}
        pageTitle={model.pageTitle}
        refreshError={model.refreshError}
        showUsage={model.showUsage}
        view={model.view}
      >
        <PageBoundary
          key={`${model.view}-${model.needsEnrollment}-${model.hasLoadedStatus}`}
        >
          {model.view === "settings" ? (
            <SettingsView
              settings={model.settings}
              disabled={!model.bootstrap || model.isSaving}
              onStartupChange={model.setOpenOnStartup}
              programManagement={model.programManagement}
              programsLoading={!model.hasLoadedStatus}
              savingPrograms={model.isChangingPrograms}
              onProgramChoiceChange={model.setProgramChoice}
            />
          ) : !model.hasLoadedStatus ? (
            <StatusLoading view={model.view} />
          ) : model.needsEnrollment && model.managedDevice ? (
            <EnrollmentView
              enrollment={model.managedDevice}
              busy={model.isManaging}
              daemon={model.connector?.runtime?.daemon ?? null}
              onCopy={model.copyDiagnostics}
              onEnroll={model.enroll}
            />
          ) : model.view === "home" ? (
            <StatusView
              bootstrap={model.bootstrap}
              connector={model.connector}
              managedDevice={model.managedDevice}
              discovery={model.discovery}
              isLoggingOut={model.isLoggingOut}
              onCopy={model.copyDiagnostics}
              onLogout={model.logout}
            />
          ) : model.view === "agents" ? (
            <AgentsView
              agents={agents.agents}
              discovery={model.discovery}
              jobs={agents.jobs}
              sync={agents.sync}
              platform={model.bootstrap?.platform ?? "macOS"}
              loading={agents.loading}
              error={agents.error}
              notice={agents.notice}
              busyAgent={agents.busyAgent}
              onStartJob={agents.startJob}
              onDryRun={agents.dryRunUninstall}
              getConfig={agents.getAgentConfig}
              saveConfig={agents.saveAgentConfig}
              onRefresh={agents.load}
              onRefreshCatalog={agents.refreshCatalog}
              onRefreshInventory={agents.refreshInventory}
            />
          ) : showingUsage ? (
            <UsageView
              error={deviceUsage.error}
              loading={deviceUsage.loading}
              onRangeChange={deviceUsage.setRange}
              range={deviceUsage.range}
              usage={deviceUsage.usage}
            />
          ) : (
            <ToolsView
              discovery={model.discovery}
              unavailable={!model.discovery}
            />
          )}
        </PageBoundary>
      </DesktopShell>
    </ThemeProvider>
  );
}
