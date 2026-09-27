import type { ApiErrorBody } from "./types";

const TOKEN_KEY = "limitwise-bearer";

export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly fields: Record<string, string> = {},
  ) {
    super(message);
  }
}

export async function bootstrapSession(): Promise<string> {
  const existing = sessionStorage.getItem(TOKEN_KEY);
  const hash = new URLSearchParams(window.location.hash.replace(/^#/, ""));
  const bootstrap = hash.get("bootstrap");
  window.history.replaceState(null, document.title, `${window.location.pathname}${window.location.search}`);
  if (!bootstrap) {
    if (existing) return existing;
    throw new ApiError("bootstrap_missing", "Launch this page from `limitwise ui` again.");
  }
  const response = await fetch("/api/v1/bootstrap", {
    method: "POST",
    credentials: "omit",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ bootstrap }),
  });
  const body = (await response.json()) as { token?: string } | ApiErrorBody;
  if (!response.ok || !("token" in body) || !body.token) {
    throwApiError(body, response.status);
  }
  sessionStorage.setItem(TOKEN_KEY, body.token);
  return body.token;
}

export async function api<T>(
  token: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const response = await fetch(`/api/v1${path}`, {
    ...init,
    credentials: "omit",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...init.headers,
    },
  });
  const body = (await response.json()) as T | ApiErrorBody;
  if (!response.ok) {
    if (response.status === 401) sessionStorage.removeItem(TOKEN_KEY);
    throwApiError(body, response.status);
  }
  return body as T;
}

function throwApiError(body: unknown, status: number): never {
  if (body && typeof body === "object" && "error" in body) {
    const error = (body as ApiErrorBody).error;
    throw new ApiError(error.code, error.message, error.fields);
  }
  throw new ApiError("http_error", `Local API request failed with HTTP ${status}`);
}
