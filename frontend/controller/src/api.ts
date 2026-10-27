import { useEffect, useState } from "react";

export const ADMIN_WRITE_HEADER = "x-agentdesktop-admin";

/**
 * Sends a mutating admin request. The controller accepts this CSRF marker on
 * its loopback listener when `adminAuth` is not configured; configured
 * deployments still require an injected bearer token.
 */
export function adminWrite(path: string, init: RequestInit = {}) {
  const headers = new Headers(init.headers);
  headers.set(ADMIN_WRITE_HEADER, "1");
  return fetch(path, { ...init, headers });
}

export function useApi<T>(path: string) {
  const [revision, setRevision] = useState(0);
  const [query, setQuery] = useState<{
    path: string;
    data: T | null;
    error: string | null;
    loading: boolean;
  }>({ path, data: null, error: null, loading: true });

  // biome-ignore lint/correctness/useExhaustiveDependencies: revision is the explicit reload trigger for this request.
  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setQuery({ path, data: null, error: null, loading: true });
    fetch(path, { signal: controller.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(await response.text());
        return response.json() as Promise<T>;
      })
      .then((response) => {
        if (active) {
          setQuery({ path, data: response, error: null, loading: false });
        }
      })
      .catch((reason: Error) => {
        if (active && reason.name !== "AbortError") {
          setQuery({
            path,
            data: null,
            error: reason.message || "Request failed",
            loading: false,
          });
        }
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [path, revision]);

  const current =
    query.path === path
      ? query
      : { path, data: null, error: null, loading: true };
  return {
    ...current,
    reload: () => setRevision((value) => value + 1),
  };
}
