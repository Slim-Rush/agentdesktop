import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";

import { DesktopStoryFrame } from "../stories/DesktopStoryFrame";
import type {
  AgentConfigResponse,
  AgentSummary,
  Discovery,
  DryRunPlan,
} from "../types";
import { AgentsView } from "./AgentsView";

const agents: AgentSummary[] = [
  {
    id: "claude-code",
    displayName: "Claude Code",
    programKey: "claudeCode",
    platforms: ["macos"],
    architectures: ["aarch64"],
    installKind: "user",
    versions: [
      {
        version: "1.2.0",
        artifactUrl: "https://example.com/claude-code.tar.gz",
        sha256: "a".repeat(64),
        signature: "sig",
      },
    ],
    detection: { paths: ["~/.local/bin/claude"] },
    configured: { theme: "dark" },
  },
  {
    id: "codex",
    displayName: "Codex",
    programKey: "codex",
    platforms: ["macos"],
    architectures: ["aarch64"],
    installKind: "user",
    versions: [
      {
        version: "0.9.0",
        artifactUrl: "https://example.com/codex.tar.gz",
        sha256: "b".repeat(64),
        signature: "sig",
      },
    ],
    detection: { paths: [] },
    configured: null,
  },
];

const discovery: Discovery = {
  agents: [
    {
      kind: "claude-code",
      executable: "/Users/test/.local/bin/claude",
      version: "1.1.0",
    },
  ],
};

const config: AgentConfigResponse = {
  id: "claude-code",
  programKey: "claudeCode",
  revision: 4,
  config: { theme: "dark" },
};

const plan: DryRunPlan = {
  dryRun: true,
  paths: [
    "~/.local/bin/claude-code",
    "~/Library/Application Support/agentdesktop/agents/claude-code/1.2.0",
  ],
};

const meta = {
  title: "Desktop/Agents",
  component: AgentsView,
  decorators: [
    (Story) => (
      <DesktopStoryFrame pageTitle="Agents" view="agents">
        <Story />
      </DesktopStoryFrame>
    ),
  ],
  parameters: { layout: "fullscreen" },
  tags: ["test"],
  args: {
    agents,
    discovery,
    jobs: [],
    sync: {
      catalog: {
        revision: 3,
        sha256: "c".repeat(64),
        lastSyncUnixSeconds: 1_700_000_000,
        stale: false,
      },
      localConfigRevision: 4,
    },
    platform: "macOS",
    architecture: "aarch64",
    loading: false,
    error: null,
    notice: null,
    jobPollError: null,
    busyAgent: null,
    onStartJob: fn(),
    onDiagnose: fn(),
    onDryRun: fn(async () => plan),
    getConfig: fn(async () => config),
    saveConfig: fn(async () => config),
    onRefresh: fn(),
    onRefreshCatalog: fn(),
    onRefreshInventory: fn(),
  },
} satisfies Meta<typeof AgentsView>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Claude Code")).toBeVisible();
    await expect(canvas.getByText(/Update available/)).toBeVisible();
    await expect(canvas.getByRole("button", { name: "Install" })).toBeVisible();
  },
};

export const Empty: Story = {
  args: { agents: [], discovery: { agents: [] } },
};

export const CatalogMissing: Story = {
  args: {
    agents: null,
    discovery: { agents: [] },
    error: "daemon returned 404 Not Found: agent catalog has not been synced",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("No signed catalog yet")).toBeVisible();
    await expect(
      canvas.getAllByRole("button", { name: "Sync catalog" }),
    ).toHaveLength(2);
    await expect(
      canvas.queryByText(/daemon returned 404/),
    ).not.toBeInTheDocument();
  },
};

export const Loading: Story = {
  args: { agents: null, loading: true },
};

export const ErrorState: Story = {
  args: {
    error:
      "Could not reach the daemon at /Users/test/Library/Application Support/agentdesktop/agentdesktop.sock.",
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/<local path>/)).toBeVisible();
    await expect(canvas.queryByText(/\/Users\/test/)).not.toBeInTheDocument();
  },
};

export const Unsupported: Story = {
  args: { platform: "Windows" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getAllByRole("button", { name: "Unsupported" }),
    ).toHaveLength(2);
  },
};

export const UnsupportedArchitecture: Story = {
  args: { architecture: "x86_64" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getAllByRole("button", { name: "Unsupported" }),
    ).toHaveLength(2);
  },
};

export const ReflowAt320: Story = {
  globals: { viewport: { value: "reflow", isRotated: false } },
  play: async ({ canvas, canvasElement }) => {
    await expect(canvas.getByText("Claude Code")).toBeVisible();
    const documentElement = canvasElement.ownerDocument.documentElement;
    const viewportWidth = canvasElement.ownerDocument.defaultView?.innerWidth;
    await expect(documentElement.scrollWidth).toBeLessThanOrEqual(
      viewportWidth ?? 0,
    );
  },
};

export const JobPollFailure: Story = {
  args: { jobPollError: "Could not reach the daemon." },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Job updates paused/)).toBeVisible();
  },
};

export const RunningJob: Story = {
  args: {
    jobs: [
      {
        jobId: "job-1",
        attempt: 1,
        state: 4,
        progressPercent: 40,
        errorCode: "",
        errorMessage: "",
        diagnosticsJson: "",
      },
    ],
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Running · 40%")).toBeVisible();
    await expect(canvas.getByRole("button", { name: "Update" })).toBeDisabled();
    await expect(
      canvas.getByRole("button", { name: "Install" }),
    ).toBeDisabled();
  },
};

export const FailedJob: Story = {
  args: {
    jobs: [
      {
        jobId: "job-2",
        attempt: 1,
        state: 7,
        progressPercent: 0,
        errorCode: "checksum_mismatch",
        errorMessage: "artifact checksum mismatch",
        diagnosticsJson: "{}",
      },
    ],
  },
  play: async ({ args, canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Diagnose" }));
    await expect(args.onDiagnose).toHaveBeenCalledWith("job-2");
  },
};

export const MissingVersions: Story = {
  args: {
    agents: [{ ...agents[0], versions: [] }],
    discovery: { agents: [] },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Claude Code")).toBeVisible();
    await expect(
      canvas.getByRole("button", { name: "Install" }),
    ).toBeDisabled();
    await expect(
      canvas.getByRole("button", { name: "Uninstall" }),
    ).toBeDisabled();
  },
};

export const Configure: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getAllByRole("button", { name: "Configure" })[0],
    );
    await expect(
      await canvas.findByLabelText("JSON configuration"),
    ).toBeVisible();
  },
};

const conflictingSave = fn(async () => {
  throw new Error("configuration revision conflict (409)");
});

export const ConfigConflict: Story = {
  args: { saveConfig: conflictingSave },
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getAllByRole("button", { name: "Configure" })[0],
    );
    const editor = await canvas.findByLabelText("JSON configuration");
    const original = JSON.stringify(config.config, null, 2);
    await waitFor(() => expect(editor).toHaveValue(original));
    const save = canvas.getByRole("button", {
      name: "Save configuration",
    });
    await waitFor(() => expect(save).toBeEnabled());
    await userEvent.click(save);
    await expect(
      await canvas.findByText(/configuration revision conflict \(409\)/),
    ).toBeVisible();
    await expect(editor).toHaveValue(original);
  },
};

export const UninstallConfirmation: Story = {
  play: async ({ canvas }) => {
    const trigger = canvas.getAllByRole("button", { name: "Uninstall" })[0];
    await userEvent.click(trigger);
    const dialog = await canvas.findByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(canvas.getByText(/remove these managed paths/)).toBeVisible();

    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    const confirm = within(dialog).getByRole("button", { name: "Uninstall" });
    await expect(confirm).toHaveFocus();

    // Tab from the last control wraps to the first, keeping focus in the dialog.
    await userEvent.tab();
    await expect(cancel).toHaveFocus();
    await expect(canvas.queryByRole("dialog")).toBeInTheDocument();

    // Shift+Tab from the first control wraps back to the last.
    await userEvent.tab({ shift: true });
    await expect(confirm).toHaveFocus();

    await userEvent.keyboard("{Escape}");
    await expect(canvas.queryByRole("dialog")).not.toBeInTheDocument();
    await expect(trigger).toHaveFocus();
  },
};
