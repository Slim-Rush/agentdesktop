import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent } from "storybook/test";

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
    loading: false,
    error: null,
    notice: null,
    busyAgent: null,
    onStartJob: fn(),
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

export const Loading: Story = {
  args: { agents: null, loading: true },
};

export const ErrorState: Story = {
  args: { error: "Could not reach the daemon." },
};

export const Unsupported: Story = {
  args: { platform: "Windows" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getAllByRole("button", { name: "Unsupported" }),
    ).toHaveLength(2);
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

export const UninstallConfirmation: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Uninstall" }));
    await expect(await canvas.findByRole("dialog")).toBeVisible();
    await expect(canvas.getByText(/remove these managed paths/)).toBeVisible();
  },
};
