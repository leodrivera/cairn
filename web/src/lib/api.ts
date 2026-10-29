// The one way the front-end talks to the server: JSON in and out, errors as ApiFailure.

export class ApiFailure extends Error {
  constructor(
    public status: number,
    public code: string,
    public serverMessage: string,
    public retryAfter: number | null = null,
  ) {
    super(`${status} ${code}: ${serverMessage}`);
  }
}

let signedOut: () => void = () => {};

/** Called whenever the server says the session is gone, before the request's promise rejects. */
export function onSignedOut(cb: () => void): void {
  signedOut = cb;
}

/** Turns a response that is not ok into an ApiFailure. */
export async function failureOf(res: Response): Promise<ApiFailure> {
  const retry = Number(res.headers.get("retry-after"));
  const retryAfter = Number.isFinite(retry) && retry > 0 ? retry : null;
  try {
    const body = await res.json();
    if (typeof body?.error === "string") {
      return new ApiFailure(res.status, body.error, String(body.message ?? ""), retryAfter);
    }
  } catch {
    // Not JSON: a proxy page or a crash. Fall through.
  }
  return new ApiFailure(res.status, "internal", res.statusText, retryAfter);
}

export function offline(): ApiFailure {
  return new ApiFailure(0, "offline", "");
}

/** A request whose answer is not JSON (file bodies). Resolves to an ok response; failures throw like `api`. */
export async function request(url: string, init: RequestInit = {}): Promise<Response> {
  const method = init.method ?? "GET";
  const headers = new Headers(init.headers);
  if (method !== "GET" && method !== "HEAD") headers.set("X-Cairn", "1");
  let res: Response;
  try {
    res = await fetch(url, { ...init, method, headers, credentials: "same-origin" });
  } catch {
    throw offline();
  }
  if (!res.ok) {
    const f = await failureOf(res);
    if (f.code === "signed_out") signedOut();
    throw f;
  }
  return res;
}

export async function api<T>(
  method: string,
  url: string,
  body?: unknown,
  headers: Record<string, string> = {},
): Promise<T> {
  const h: Record<string, string> = { ...headers };
  if (method !== "GET" && method !== "HEAD") h["X-Cairn"] = "1";
  if (body !== undefined) h["Content-Type"] = "application/json";
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers: h,
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
    });
  } catch {
    throw offline();
  }
  if (!res.ok) {
    const f = await failureOf(res);
    if (f.code === "signed_out") signedOut();
    throw f;
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}
