import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import { ControllerStoryFrame } from "../stories/ControllerStoryFrame";
import {
  deviceAudit,
  deviceDetail,
  deviceUsageReport,
  emptyDeviceDetail,
  emptyUsageReport,
  failedDeviceDetail,
  oldAgentDeviceDetail,
  optedOutDeviceDetail,
  retiredDeviceDetail,
} from "../stories/fixtures";
import { DeviceView } from "./DeviceView";

const meta = {
  title: "Controller/Device details",
  component: DeviceView,
  decorators: [
    (Story) => (
      <ControllerStoryFrame path="/devices/device-mac-12345678">
        <Story />
      </ControllerStoryFrame>
    ),
  ],
  parameters: { layout: "fullscreen" },
  tags: ["test"],
  args: {
    audit: deviceAudit,
    auditError: null,
    auditLoadError: null,
    auditLoading: false,
    auditLoadingMore: false,
    device: deviceDetail,
    onAuditLoadMore: fn(),
    onTransitionCancel: fn(),
    onTransitionConfirm: fn(),
    onTransitionRequest: fn(),
    transitionError: null,
    transitionTarget: null,
    transitioning: false,
  },
} satisfies Meta<typeof DeviceView>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Healthy: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Ollama")).toBeVisible();
    await expect(canvas.getByText("qwen3:8b")).toBeVisible();
  },
};

export const BrowsesDiscoveredCapabilities: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByText("VS Code", { selector: ".tool-inventory-item strong" }),
    );
    await expect(
      canvas.getByRole("tab", { name: "MCP servers 7" }),
    ).toHaveAttribute("aria-selected", "true");
    await userEvent.click(canvas.getByRole("tab", { name: "Skills 11" }));
    const skillsPanel = within(
      await canvas.findByRole("tabpanel", { name: "Skills 11" }),
    );
    await expect(skillsPanel.getByText("Release workflow")).toBeVisible();
    await userEvent.click(
      skillsPanel.getByRole("button", { name: "Next Skills page" }),
    );
    await expect(skillsPanel.getByText("Performance diagnosis")).toBeVisible();
  },
};

export const ConfigurationFailed: Story = {
  args: { device: failedDeviceDetail },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Managed programs")).toBeVisible();
    await expect(canvas.getByText("Conflict")).toBeVisible();
    await expect(canvas.getByText("Blocked")).toBeVisible();
    await expect(
      canvas.getByText("not applied: vscode conflicted"),
    ).toBeVisible();
  },
};

export const ProgramOptedOut: Story = {
  args: { device: optedOutDeviceDetail },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Opted out")).toHaveAttribute(
      "title",
      "The device user opted out of this program",
    );
    await expect(canvas.getByText("Applied")).not.toHaveAttribute("title");
  },
};

export const OlderAgentWithoutProgramStatus: Story = {
  args: { device: oldAgentDeviceDetail },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(
        "Per-program status is not reported by this agent version",
      ),
    ).toBeVisible();
  },
};

export const NoActivityOrTools: Story = {
  args: { device: emptyDeviceDetail },
};

export const AuditLoadMoreFailed: Story = {
  args: { auditLoadError: "Audit history could not be loaded." },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Audit history could not be loaded."),
    ).toBeVisible();
  },
};

export const RetiredDeviceWithHistory: Story = {
  args: { device: retiredDeviceDetail },
  play: async ({ canvas }) => {
    await expect(canvas.getAllByText("Retired")[0]).toBeVisible();
    await expect(canvas.getByText("device-mac-old-00000001")).toBeVisible();
    await expect(canvas.getByText("device.re_enrolled")).toBeVisible();
  },
};

export const RequestsRetirement: Story = {
  play: async ({ args, canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Retire device" }),
    );
    await expect(args.onTransitionRequest).toHaveBeenCalledWith("retired");
  },
};

export const RetireConfirmation: Story = {
  args: { transitionTarget: "retired" },
  play: async ({ args, canvas }) => {
    const dialog = within(canvas.getByRole("dialog"));
    await expect(
      dialog.getByRole("heading", { name: "Retire dev-mac?" }),
    ).toBeVisible();
    await userEvent.click(
      dialog.getByRole("button", { name: "Retire device" }),
    );
    await expect(args.onTransitionConfirm).toHaveBeenCalledOnce();
  },
};

export const RevokeFailed: Story = {
  args: {
    transitionError: "The controller could not revoke this device.",
    transitionTarget: "revoked",
  },
};

export const Revoking: Story = {
  args: { transitionTarget: "revoked", transitioning: true },
};

export const WithUsage: Story = {
  args: { usage: deviceUsageReport },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("heading", { name: "Usage" })).toBeVisible();
    const tools = within(canvas.getByRole("table", { name: "Usage by tool" }));
    await expect(tools.getByText("Claude Code")).toBeVisible();
    await expect(
      canvas.getByRole("link", { name: "View in Usage" }),
    ).toHaveAttribute("href", "/usage?range=30d&device=device-mac-12345678");
  },
};

export const WithoutRecentUsage: Story = {
  args: { usage: emptyUsageReport },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("No LLM gateway usage in the last 30 days."),
    ).toBeVisible();
  },
};

export const Dark: Story = {
  ...BrowsesDiscoveredCapabilities,
  globals: { colorMode: "dark" },
};

export const DarkRevokeFailed: Story = {
  ...RevokeFailed,
  globals: { colorMode: "dark" },
};
