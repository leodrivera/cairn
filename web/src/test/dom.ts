// Test helpers: a DOM from happy-dom, a fake fetch, and rendering into it.
// Never pass a DOM node to assert.equal or deepEqual: a failing message inspects the whole document and hangs.
// Compare with assert.ok(a === b) or assert.ok(!node) instead.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { render as preactRender, type ComponentChild } from "preact";
import { act } from "preact/test-utils";

if (!(globalThis as { happyDOM?: unknown }).happyDOM) {
  GlobalRegistrator.register({
    url: "https://files.example.com/",
    width: 1280,
    height: 800,
    // Never load third-party scripts in tests; a script tag just fires "load".
    settings: { disableJavaScriptFileLoading: true, handleDisabledFileLoadingAsSuccess: true },
  });
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

/** The control a <label> with exactly this text points at. */
export function byLabel<T extends HTMLElement = HTMLInputElement>(root: HTMLElement, text: string): T {
  const label = [...root.querySelectorAll("label")].find((l) => l.textContent?.trim() === text);
  if (!label) throw new Error(`no label "${text}"`);
  const control = label.htmlFor ? root.querySelector(`#${CSS.escape(label.htmlFor)}`) : label.querySelector("input");
  if (!control) throw new Error(`label "${text}" points at nothing`);
  return control as T;
}

/** Sets a field's value and fires input and change, as typing or choosing would. */
export function setValue(control: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string): void {
  control.value = value;
  control.dispatchEvent(new Event("input", { bubbles: true }));
  control.dispatchEvent(new Event("change", { bubbles: true }));
}

/** The button whose text is exactly `text`. */
export function buttonNamed(root: HTMLElement, text: string): HTMLButtonElement {
  const b = [...root.querySelectorAll("button")].find((x) => x.textContent?.trim() === text);
  if (!b) throw new Error(`no button "${text}"`);
  return b as HTMLButtonElement;
}

/** Makes `(max-width: 720px)` match (a phone) or not. Returns a function that puts the real matchMedia back. */
export function phoneWidth(on = true): () => void {
  const real = window.matchMedia;
  window.matchMedia = ((query: string) => ({
    matches: on && query.includes("max-width: 720px"),
    media: query,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  return () => {
    window.matchMedia = real;
  };
}
