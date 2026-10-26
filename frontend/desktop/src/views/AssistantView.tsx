import {
  AlertCircle,
  Bot,
  LoaderCircle,
  RefreshCw,
  Send,
  ShieldCheck,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type {
  ChatAction,
  ChatContext,
  ChatMessage,
  ChatProviderInfo,
} from "../types";

type ActionNotice = { tone: "success" | "error"; message: string } | null;

export interface AssistantViewProps {
  providers: ChatProviderInfo[];
  providerError: string | null;
  selectedProvider: string;
  onProviderChange: (providerId: string) => void;
  context: ChatContext | null;
  contextLoading: boolean;
  contextError: string | null;
  messages: ChatMessage[];
  sending: boolean;
  error: string | null;
  pendingAction: ChatAction | null;
  actionBusy: boolean;
  actionNotice: ActionNotice;
  onSend: (text: string) => void;
  onRetry: () => void;
  onCancel: () => void;
  onClear: () => void;
  onRefreshContext: () => void;
  onReviewAction: (action: ChatAction) => void;
  onConfirmAction: () => void;
  onCancelAction: () => void;
}

function ActionCard({
  action,
  disabled,
  onReview,
}: {
  action: ChatAction;
  disabled: boolean;
  onReview: (action: ChatAction) => void;
}) {
  return (
    <div className="assistant-action">
      <div>
        <strong>{action.label}</strong>
        <p>{action.description}</p>
      </div>
      <button
        type="button"
        className="button button-secondary"
        disabled={disabled}
        onClick={() => onReview(action)}
      >
        Review
      </button>
    </div>
  );
}

export function AssistantView({
  providers,
  providerError,
  selectedProvider,
  onProviderChange,
  context,
  contextLoading,
  contextError,
  messages,
  sending,
  error,
  pendingAction,
  actionBusy,
  actionNotice,
  onSend,
  onRetry,
  onCancel,
  onClear,
  onRefreshContext,
  onReviewAction,
  onConfirmAction,
  onCancelAction,
}: AssistantViewProps) {
  const [draft, setDraft] = useState("");
  const dialogRef = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const cancelActionRef = useRef(onCancelAction);
  cancelActionRef.current = onCancelAction;
  const selected = providers.find(
    (provider) => provider.id === selectedProvider,
  );
  const ruleBased = selectedProvider === "rule-based" || providers.length === 0;

  useEffect(() => {
    if (!pendingAction) return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const trigger =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    confirmRef.current?.focus();

    const focusable = () =>
      Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelActionRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusable();
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !dialog.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (active === last || !dialog.contains(active))
      ) {
        event.preventDefault();
        first.focus();
      }
    };
    const onFocusIn = (event: FocusEvent) => {
      if (!dialog.contains(event.target as Node)) {
        event.stopPropagation();
        confirmRef.current?.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusin", onFocusIn, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("focusin", onFocusIn, true);
      const usable =
        trigger?.isConnected &&
        !(trigger instanceof HTMLButtonElement && trigger.disabled);
      const target = usable
        ? trigger
        : document.querySelector<HTMLElement>(".page-heading button");
      target?.focus();
    };
  }, [pendingAction]);

  function submit() {
    const text = draft.trim();
    if (!text || sending || contextLoading || contextError || !context) return;
    setDraft("");
    onSend(text);
  }

  return (
    <div className="page-stack assistant-page">
      <div className="page-heading">
        <div>
          <h2>Failure assistant</h2>
          <p>
            Ask a local coding agent that is already installed and configured on
            this device.
          </p>
        </div>
        <div className="agent-actions">
          <button
            type="button"
            onClick={() => onRefreshContext()}
            disabled={contextLoading}
          >
            <RefreshCw size={16} /> Refresh context
          </button>
          <button
            type="button"
            onClick={onClear}
            disabled={!messages.length && !error}
          >
            Clear conversation
          </button>
        </div>
      </div>

      <section className="card assistant-provider">
        <label htmlFor="assistant-provider">Provider</label>
        <select
          id="assistant-provider"
          value={selectedProvider}
          onChange={(event) => onProviderChange(event.target.value)}
          disabled={sending}
        >
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.displayName}
            </option>
          ))}
          <option value="rule-based">Rule-based diagnosis</option>
        </select>
        <p className="agent-muted">
          {ruleBased
            ? "The built-in playbooks explain the error without calling a model."
            : `Uses ${selected?.displayName ?? "the selected agent"}'s existing configuration — ${selected?.readOnlyMode ?? "read-only"}. No new API key or model setting is added.`}
        </p>
        {providerError ? (
          <p className="notice notice-error" role="status">
            <AlertCircle size={16} /> Could not discover local providers:{" "}
            {providerError}. Falling back to rule-based diagnosis.
          </p>
        ) : null}
        {!providerError && providers.length === 0 ? (
          <p className="notice notice-error" role="status">
            No installed coding agent with a verified read-only mode was found.
            Using rule-based diagnosis.
          </p>
        ) : null}
      </section>

      {contextLoading ? (
        <section className="card">
          <div className="empty-inline">
            <LoaderCircle size={20} className="spin" />
            <div>
              <strong>Loading context</strong>
              <span>Reading the failed job and catalog metadata.</span>
            </div>
          </div>
        </section>
      ) : contextError ? (
        <div className="notice notice-error" role="alert">
          <AlertCircle size={16} /> {contextError}
          <button
            type="button"
            className="button button-secondary"
            onClick={() => onRefreshContext()}
          >
            Retry
          </button>
        </div>
      ) : context ? (
        <>
          <section className="card assistant-context">
            <details open>
              <summary>
                <span>
                  <strong>Context preview</strong>
                  <small>Redacted before sending</small>
                </span>
                <span>
                  {context.job
                    ? `Job ${context.job.jobId}`
                    : "No failed job selected"}
                </span>
              </summary>
              {context.warnings.length ? (
                <p className="notice notice-error" role="status">
                  <AlertCircle size={16} /> Some context is unavailable:{" "}
                  {context.warnings.join(" ")}
                </p>
              ) : null}
              <p className="agent-muted">
                Only this redacted context is sent with your message. Home
                paths, environment values, and token-like strings are removed.
              </p>
              <pre className="assistant-context-preview">{context.preview}</pre>
            </details>
          </section>

          {context.playbook.actions.length && !messages.length ? (
            <section className="card assistant-playbook">
              <h3>Suggested next steps</h3>
              <p>{context.playbook.explanation}</p>
              <div className="assistant-actions">
                {context.playbook.actions.map((action) => (
                  <ActionCard
                    key={action.id}
                    action={action}
                    disabled={sending || actionBusy}
                    onReview={onReviewAction}
                  />
                ))}
              </div>
            </section>
          ) : null}
        </>
      ) : null}

      <section className="card assistant-chat">
        <div
          className="assistant-messages"
          role="log"
          aria-label="Conversation"
          aria-live="polite"
          aria-relevant="additions text"
        >
          {messages.length === 0 ? (
            <div className="assistant-welcome">
              <Bot size={22} />
              <div>
                <strong>Ask about a failed agent job</strong>
                <p>
                  Describe what you expected, or ask what an error code means.
                  The assistant can suggest only the allowlisted actions shown
                  in the context card.
                </p>
              </div>
            </div>
          ) : (
            messages.map((entry) => (
              <article
                key={entry.id}
                className={`assistant-message assistant-message-${entry.role}${entry.tone === "error" ? " assistant-message-error" : ""}`}
              >
                <span className="assistant-message-role">
                  {entry.role === "user" ? "You" : "Assistant"}
                </span>
                <p>{entry.content}</p>
                {entry.actions?.length ? (
                  <div className="assistant-actions">
                    {entry.actions.map((action) => (
                      <ActionCard
                        key={action.id}
                        action={action}
                        disabled={sending || actionBusy}
                        onReview={onReviewAction}
                      />
                    ))}
                  </div>
                ) : null}
              </article>
            ))
          )}
          {sending ? (
            <div className="assistant-thinking" role="status">
              <LoaderCircle size={16} className="spin" />
              <span>Thinking…</span>
              <button
                type="button"
                className="button button-secondary"
                onClick={onCancel}
              >
                Cancel
              </button>
            </div>
          ) : null}
        </div>

        {error ? (
          <div className="notice notice-error assistant-error" role="alert">
            <AlertCircle size={16} />
            <span>{error}</span>
            <button
              type="button"
              className="button button-secondary"
              onClick={onRetry}
              disabled={sending}
            >
              Retry
            </button>
          </div>
        ) : null}
        {actionNotice && (!pendingAction || actionNotice.tone === "success") ? (
          <div
            className={`notice notice-${actionNotice.tone}`}
            role={actionNotice.tone === "error" ? "alert" : "status"}
          >
            {actionNotice.message}
          </div>
        ) : null}

        <form
          className="assistant-composer"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <label htmlFor="assistant-message">Message</label>
          <textarea
            id="assistant-message"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
            placeholder="Ask why the job failed…"
            rows={3}
            disabled={
              sending || contextLoading || contextError !== null || !context
            }
          />
          <div className="assistant-composer-actions">
            <span className="agent-muted">
              Enter sends · Shift+Enter adds a line
            </span>
            <button
              type="submit"
              className="button button-primary"
              disabled={
                sending ||
                contextLoading ||
                contextError !== null ||
                !context ||
                !draft.trim()
              }
            >
              <Send size={14} /> Send
            </button>
          </div>
        </form>
      </section>

      {pendingAction ? (
        <div className="agent-modal-backdrop" role="presentation">
          <div
            ref={dialogRef}
            className="agent-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="assistant-action-title"
            aria-describedby="assistant-action-description"
          >
            <h3 id="assistant-action-title">Confirm {pendingAction.label}?</h3>
            <p id="assistant-action-description">{pendingAction.description}</p>
            {actionNotice?.tone === "error" ? (
              <p className="notice notice-error" role="alert">
                {actionNotice.message}
              </p>
            ) : null}
            <p className="agent-muted">
              <ShieldCheck size={14} /> This changes local state and is recorded
              in the assistant audit log without message content.
            </p>
            <div className="agent-modal-actions">
              <button
                type="button"
                onClick={onCancelAction}
                disabled={actionBusy}
              >
                Cancel
              </button>
              <button
                ref={confirmRef}
                type="button"
                className="button button-primary"
                onClick={() => void onConfirmAction()}
                disabled={actionBusy}
              >
                {actionBusy ? "Working…" : "Confirm action"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
