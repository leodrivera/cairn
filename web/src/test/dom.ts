// Test helpers: a DOM from happy-dom, a fake fetch, and rendering into it.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { render as preactRender, type ComponentChild } from "preact";
import { act } from "preact/test-utils";

if (!(globalThis as { happyDOM?: unknown }).happyDOM) {
  GlobalRegistrator.register({ url: "https://files.example.com/", width: 1280, height: 800 });
}

export type Handler = (req: Request) => Response | Promise<Response>;

/** Replaces fetch. Keys are "METHOD /path" (no query); an unmatched request fails the test. Returns the log. */
export function stubFetch(routes: Record<string, Handler>): Request[] {
  const log: Request[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(new URL(String(input instanceof Request ? input.url : input), location.href), init);
    log.push(req.clone());
    const key = `${req.method} ${decodeURIComponent(new URL(req.url).pathname)}`;
    const handler = routes[key] ?? routes[`${req.method} *`];
    if (!handler) throw new Error(`unexpected request ${key}`);
    return handler(req);
  }) as typeof fetch;
  return log;
}

export function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

let container: HTMLElement | null = null;

/** Renders into a fresh container, unmounting the previous one. */
export function render(vnode: ComponentChild): HTMLElement {
  if (container) {
    preactRender(null, container);
    container.remove();
  }
  const el = document.createElement("div");
  container = el;
  document.body.append(el);
  act(() => preactRender(vnode, el));
  return el;
}

/** Lets pending promises, effects and zero-delay timers run. */
export async function flush(times = 5): Promise<void> {
  for (let i = 0; i < times; i++) {
    await act(() => new Promise<void>((r) => setTimeout(r, 0)));
  }
}

/** Flushes until `check` stops throwing, for at most `ms`; then throws its last error. */
export async function waitFor(check: () => void, ms = 2000): Promise<void> {
  const end = Date.now() + ms;
  for (;;) {
    try {
      check();
      return;
    } catch (e) {
      if (Date.now() > end) throw e;
      await flush(1);
    }
  }
}
