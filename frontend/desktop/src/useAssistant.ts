import { useCallback, useEffect, useRef, useState } from "react";

import {
  cancelChat,
  executeChatAction,
  getChatContext,
  getChatProviders,
  startChat,
} from "./backend";
import { redactPaths } from "./redact";
import type {
  ChatAction,
  ChatContext,
  ChatMessage,
  ChatProviderInfo,
} from "./types";

type ActionNotice = { tone: "success" | "error"; message: string } | null;

function message(error: unknown): string {
  return redactPaths(error instanceof Error ? error.message : String(error));
}

function newRequestId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `chat-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function useAssistant(
  enabled: boolean,
  jobId: string | null,
  onOpenSettings: () => void,
) {
  const [providers, setProviders] = useState<ChatProviderInfo[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);
  const [selectedProvider, setSelectedProvider] = useState("rule-based");
  const [context, setContext] = useState<ChatContext | null>(null);
  const [contextLoading, setContextLoading] = useState(false);
  const [contextError, setContextError] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastMessage, setLastMessage] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<ChatAction | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [actionNotice, setActionNotice] = useState<ActionNotice>(null);
  const loadGeneration = useRef(0);
  const activeRequest = useRef<string | null>(null);
  const userProvider = useRef<string | null>(null);
  const settingsRef = useRef(onOpenSettings);
  settingsRef.current = onOpenSettings;
  const jobIdRef = useRef(jobId);
  jobIdRef.current = jobId;

  const load = useCallback(
    async (requestedJobId: string | null = jobIdRef.current) => {
      const current = ++loadGeneration.current;
      setContextLoading(true);
      setContextError(null);
      setProviderError(null);
      setError(null);
      setMessages([]);
      setPendingAction(null);
      setActionNotice(null);
      setSending(false);
      const running = activeRequest.current;
      activeRequest.current = null;
      if (running) void cancelChat(running).catch(() => {});
      const [providerResult, contextResult] = await Promise.allSettled([
        getChatProviders(),
        getChatContext(requestedJobId),
      ]);
      if (current !== loadGeneration.current) return;
      if (providerResult.status === "fulfilled") {
        setProviders(providerResult.value);
        const explicit = userProvider.current;
        setSelectedProvider(
          explicit &&
            (explicit === "rule-based" ||
              providerResult.value.some((provider) => provider.id === explicit))
            ? explicit
            : (providerResult.value[0]?.id ?? "rule-based"),
        );
      } else {
        setProviders([]);
        setSelectedProvider("rule-based");
        setProviderError(message(providerResult.reason));
      }
      if (contextResult.status === "fulfilled") {
        setContext(contextResult.value);
      } else {
        setContextError(message(contextResult.reason));
      }
      setContextLoading(false);
    },
    [],
  );

  useEffect(() => {
    if (enabled) void load(jobId);
  }, [enabled, jobId, load]);

  const send = useCallback(
    async (text: string, appendUser = true) => {
      const trimmed = text.trim();
      if (!trimmed || sending) return;
      const history = messages
        .filter((entry) => entry.tone !== "error")
        .map((entry) => ({ role: entry.role, content: entry.content }));
      if (appendUser) {
        setMessages((current) => [
          ...current,
          { id: newRequestId(), role: "user", content: trimmed },
        ]);
      }
      setSending(true);
      setError(null);
      setActionNotice(null);
      setLastMessage(trimmed);
      const requestId = newRequestId();
      activeRequest.current = requestId;
      try {
        const reply = await startChat(
          requestId,
          selectedProvider,
          jobIdRef.current,
          trimmed,
          history,
        );
        if (activeRequest.current !== requestId) return;
        setMessages((current) => [
          ...current,
          {
            id: `${requestId}-reply`,
            role: "assistant",
            content: reply.message,
            actions: reply.proposedActions,
          },
        ]);
      } catch (reason) {
        if (activeRequest.current !== requestId) return;
        setError(message(reason));
      } finally {
        if (activeRequest.current === requestId) {
          activeRequest.current = null;
          setSending(false);
        }
      }
    },
    [messages, selectedProvider, sending],
  );

  const retry = useCallback(() => {
    if (lastMessage) void send(lastMessage, false);
  }, [lastMessage, send]);

  const cancel = useCallback(async () => {
    const requestId = activeRequest.current;
    if (!requestId) return;
    activeRequest.current = null;
    setSending(false);
    setActionNotice({ tone: "success", message: "Request cancelled." });
    await cancelChat(requestId).catch(() => {});
  }, []);

  const clear = useCallback(() => {
    const requestId = activeRequest.current;
    activeRequest.current = null;
    setSending(false);
    if (requestId) void cancelChat(requestId).catch(() => {});
    setMessages([]);
    setError(null);
    setLastMessage(null);
    setPendingAction(null);
    setActionNotice(null);
  }, []);

  const confirmAction = useCallback(async () => {
    const action = pendingAction;
    const job = context?.job;
    if (!action || !job || actionBusy) return;
    setActionBusy(true);
    setActionNotice(null);
    try {
      const result = await executeChatAction(
        action.id,
        job.jobId,
        selectedProvider,
      );
      setMessages((current) => [
        ...current,
        {
          id: newRequestId(),
          role: "assistant",
          content: result.detail,
        },
      ]);
      setActionNotice({ tone: "success", message: result.detail });
      setPendingAction(null);
      if (result.navigate === "settings") settingsRef.current();
    } catch (reason) {
      setActionNotice({ tone: "error", message: message(reason) });
    } finally {
      setActionBusy(false);
    }
  }, [actionBusy, context?.job, pendingAction, selectedProvider]);

  const chooseProvider = useCallback((providerId: string) => {
    userProvider.current = providerId;
    setSelectedProvider(providerId);
    setError(null);
  }, []);

  const cancelAction = useCallback(() => setPendingAction(null), []);

  return {
    providers,
    providerError,
    selectedProvider,
    chooseProvider,
    context,
    contextLoading,
    contextError,
    messages,
    sending,
    error,
    pendingAction,
    actionBusy,
    actionNotice,
    load,
    send,
    retry,
    cancel,
    clear,
    requestAction: setPendingAction,
    cancelAction,
    confirmAction,
  };
}
