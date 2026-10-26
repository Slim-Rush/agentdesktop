import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import { DesktopStoryFrame } from "../stories/DesktopStoryFrame";
import type {
  ChatAction,
  ChatContext,
  ChatMessage,
  ChatProviderInfo,
} from "../types";
import { AssistantView } from "./AssistantView";

const provider: ChatProviderInfo = {
  id: "claude-code",
  displayName: "Claude Code",
  available: true,
  readOnlyMode: "print mode with no tools and no MCP servers",
  unavailableReason: null,
};

const refreshCatalog: ChatAction = {
  id: "refresh_catalog",
  label: "Refresh catalog",
  description: "Sync the signed catalog before trying again.",
};

const retryInstall: ChatAction = {
  id: "retry_install",
  label: "Retry install",
  description: "Start a new install job for the same agent and version.",
};

const context: ChatContext = {
  job: {
    jobId: "local-1700000000-1",
    agentId: "claude-code",
    version: "1.2.0",
    action: "install",
    state: 7,
    errorCode: "checksum_mismatch",
    errorMessage: "artifact checksum does not match the signed catalog",
    diagnostics:
      '{"agentId":"claude-code","version":"1.2.0","action":1,"catalogRevision":7}',
    catalogRevision: 7,
  },
  recentJobs: [
    {
      jobId: "local-1700000000-1",
      agentId: "claude-code",
      version: "1.2.0",
      action: "install",
      state: 7,
      errorCode: "checksum_mismatch",
      errorMessage: "artifact checksum does not match the signed catalog",
      diagnostics: "{}",
      catalogRevision: 7,
    },
  ],
  agents: [{ id: "claude-code", displayName: "Claude Code", configured: true }],
  catalogRevision: 7,
  catalogStale: false,
  localConfigRevision: 3,
  warnings: [],
  preview:
    "Failed job\n- Job: local-1700000000-1\n- Agent: claude-code 1.2.0\n- Action: install\n- State: 7\n- Error code: checksum_mismatch\n- Error: artifact checksum does not match the signed catalog\n\nCatalog and sync\n- Revision: 7\n- Stale: false\n- Local config revision: 3",
  playbook: {
    errorCode: "checksum_mismatch",
    explanation:
      "The downloaded agent did not match the signed catalog. Refresh the catalog and retry the install.",
    actions: [refreshCatalog, retryInstall],
  },
};

const messages: ChatMessage[] = [
  { id: "message-1", role: "user", content: "Why did the install fail?" },
  {
    id: "message-2",
    role: "assistant",
    content:
      "The artifact checksum did not match the signed catalog. Refresh the catalog, then retry the install.",
    actions: [refreshCatalog, retryInstall],
  },
];

const meta = {
  title: "Desktop/Assistant",
  component: AssistantView,
  decorators: [
    (Story) => (
      <DesktopStoryFrame pageTitle="Assistant" view="assistant">
        <Story />
      </DesktopStoryFrame>
    ),
  ],
  parameters: { layout: "fullscreen" },
  tags: ["test"],
  args: {
    providers: [provider],
    providerError: null,
    selectedProvider: "claude-code",
    onProviderChange: fn(),
    context,
    contextLoading: false,
    contextError: null,
    messages: [],
    sending: false,
    error: null,
    pendingAction: null,
    actionBusy: false,
    actionNotice: null,
    onSend: fn(),
    onRetry: fn(),
    onCancel: fn(),
    onClear: fn(),
    onRefreshContext: fn(),
    onReviewAction: fn(),
    onConfirmAction: fn(),
    onCancelAction: fn(),
  },
} satisfies Meta<typeof AssistantView>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Ask about a failed agent job"),
    ).toBeVisible();
    await expect(canvas.getByText("Context preview")).toBeVisible();
    await expect(canvas.getByText(/Failed job/)).toBeVisible();
    await expect(canvas.getByText(/checksum_mismatch/)).toBeVisible();
    await expect(canvas.getByText("Refresh catalog")).toBeVisible();
    await expect(
      canvas.getAllByRole("button", { name: "Review" }),
    ).toHaveLength(2);
  },
};

export const Populated: Story = {
  args: { messages },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Why did the install fail?")).toBeVisible();
    await expect(canvas.getByRole("log")).toHaveAttribute(
      "aria-live",
      "polite",
    );
    await expect(
      canvas.getAllByRole("button", { name: "Review" }),
    ).toHaveLength(2);
  },
};

export const RuleBasedFallback: Story = {
  args: {
    providers: [],
    selectedProvider: "rule-based",
    messages: [
      {
        id: "message-rule-based",
        role: "assistant",
        content:
          "The downloaded agent did not match the signed catalog. Refresh the catalog and retry the install.",
        actions: [refreshCatalog, retryInstall],
      },
    ],
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/No installed coding agent with a verified/),
    ).toBeVisible();
  },
};

export const LoadingContext: Story = {
  args: { context: null, contextLoading: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Loading context")).toBeVisible();
  },
};

export const ProviderError: Story = {
  args: {
    providers: [],
    selectedProvider: "rule-based",
    providerError: "discovery failed",
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/Could not discover local providers/),
    ).toBeVisible();
  },
};

export const ErrorWithRetry: Story = {
  args: { error: "The local assistant provider timed out." },
  play: async ({ args, canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Retry" }));
    await expect(args.onRetry).toHaveBeenCalledOnce();
  },
};

export const ContextError: Story = {
  args: { context: null, contextError: "Could not reach the daemon." },
  play: async ({ args, canvas }) => {
    await expect(canvas.getByText("Could not reach the daemon.")).toBeVisible();
    await expect(canvas.getByLabelText("Message")).toBeDisabled();
    await userEvent.click(canvas.getByRole("button", { name: "Retry" }));
    await expect(args.onRefreshContext).toHaveBeenCalledOnce();
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh context" }),
    );
    await expect(args.onRefreshContext).toHaveBeenLastCalledWith();
  },
};

export const ContextWarnings: Story = {
  args: {
    context: {
      ...context,
      catalogRevision: null,
      warnings: ["Daemon sync status is unavailable."],
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Some context is unavailable/)).toBeVisible();
    await expect(
      canvas.getByText(/Daemon sync status is unavailable/),
    ).toBeVisible();
  },
};

export const Sending: Story = {
  args: { messages, sending: true },
  play: async ({ args, canvas }) => {
    await expect(canvas.getByText("Thinking…")).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }));
    await expect(args.onCancel).toHaveBeenCalledOnce();
  },
};

export const SendMessage: Story = {
  play: async ({ args, canvas }) => {
    await userEvent.type(
      canvas.getByLabelText("Message"),
      "What does checksum_mismatch mean?{Enter}",
    );
    await expect(args.onSend).toHaveBeenCalledWith(
      "What does checksum_mismatch mean?",
    );
  },
};

export const ClearConversation: Story = {
  args: { messages },
  play: async ({ args, canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Clear conversation" }),
    );
    await expect(args.onClear).toHaveBeenCalledOnce();
  },
};

export const ActionConfirmation: Story = {
  args: { messages, pendingAction: refreshCatalog },
  play: async ({ args, canvas }) => {
    const dialog = await canvas.findByRole("dialog");
    await expect(dialog).toBeVisible();
    const confirm = within(dialog).getByRole("button", {
      name: "Confirm action",
    });
    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    await expect(confirm).toHaveFocus();

    await userEvent.tab();
    await expect(cancel).toHaveFocus();
    await userEvent.tab({ shift: true });
    await expect(confirm).toHaveFocus();

    await userEvent.keyboard("{Escape}");
    await expect(args.onCancelAction).toHaveBeenCalledOnce();
  },
};

export const ActionFailure: Story = {
  args: {
    pendingAction: refreshCatalog,
    actionNotice: {
      tone: "error",
      message: "The daemon rejected the action.",
    },
  },
  play: async ({ canvas }) => {
    const dialog = await canvas.findByRole("dialog");
    await expect(
      within(dialog).getByText("The daemon rejected the action."),
    ).toBeVisible();
  },
};

export const ReflowAt320: Story = {
  args: { messages },
  globals: { viewport: { value: "reflow", isRotated: false } },
  play: async ({ canvas, canvasElement }) => {
    await expect(canvas.getByText("Why did the install fail?")).toBeVisible();
    const documentElement = canvasElement.ownerDocument.documentElement;
    const viewportWidth = canvasElement.ownerDocument.defaultView?.innerWidth;
    await expect(documentElement.scrollWidth).toBeLessThanOrEqual(
      viewportWidth ?? 0,
    );
  },
};
