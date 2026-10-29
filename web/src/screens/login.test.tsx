import "../test/dom";
import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { flush, json, render, stubFetch, waitFor } from "../test/dom";
import type { Config, User } from "../lib/types";

const config: Config = {
  authMethods: ["password"],
  captcha: null,
  providers: [],
  theme: null,
  thumbnails: false,
  editMax: 2097152,
};

const admin: User = {
  id: 1,
  email: "admin@example.com",
  name: "Admin",
  scope: "/",
  permissions: { read: true, write: true, share: true, admin: true },
  lang: null,
  theme: null,
  view: null,
  hasPassword: true,
};

function open(cfg: Partial<Config>, login?: (req: Request) => Response | Promise<Response>, at = "/login") {
  history.replaceState(null, "", at);
  const log = stubFetch({
    "GET /api/config": () => json({ ...config, ...cfg }),
    "GET /api/me": () => json({ user: null }),
    "POST /api/auth/login": login ?? (() => json({ user: admin })),
  });
  return { log, el: render(<App />) };
}

function type(el: HTMLElement, selector: string, value: string) {
  const input = el.querySelector<HTMLInputElement>(selector)!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

async function ready(el: HTMLElement) {
  await waitFor(() => assert.ok(el.querySelector("h1")));
}

async function submit(el: HTMLElement) {
  await flush(); // let typed values reach state, as between real key presses
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

afterEach(() => {
  delete (window as { grecaptcha?: unknown }).grecaptcha;
  document.head.querySelectorAll("script").forEach((s) => s.remove());
});

test("posts credentials and goes to next", async () => {
  const { log, el } = open({}, undefined, "/login?next=%2Ffiles%2Fphotos");
  await ready(el);
  assert.match(el.textContent!, /files\.example\.com/);
  type(el, "#login-email", "admin@example.com");
  type(el, "#login-password", "Abcdef1!");
  el.querySelector<HTMLInputElement>("#login-keep")!.click();
  await submit(el);
  await waitFor(() => assert.equal(location.pathname, "/files/photos"));
  const req = log.find((r) => r.method === "POST")!;
  assert.equal(req.headers.get("x-cairn"), "1");
  assert.deepEqual(await req.json(), { email: "admin@example.com", password: "Abcdef1!", keep: true });
});

test("wrong credentials clears the password", async () => {
  const { el } = open({}, () => json({ error: "wrong_credentials", message: "Wrong e-mail or password." }, 401));
  await ready(el);
  type(el, "#login-email", "admin@example.com");
  type(el, "#login-password", "nope");
  await submit(el);
  await waitFor(() => assert.match(el.querySelector('[role="alert"]')!.textContent!, /Wrong e-mail or password\./));
  const pw = el.querySelector<HTMLInputElement>("#login-password")!;
  assert.equal(pw.value, "");
  assert.equal(document.activeElement, pw);
});

test("too many uses Retry-After", async () => {
  const { el } = open({}, () => json({ error: "too_many", message: "x" }, 429, { "retry-after": "840" }));
  await ready(el);
  type(el, "#login-email", "admin@example.com");
  type(el, "#login-password", "x");
  await submit(el);
  await waitFor(() => assert.match(el.textContent!, /Wait 14 minutes, then try again\./));
});

test("setup notice only when needed", async () => {
  let { el } = open({ setupNeeded: true });
  await ready(el);
  assert.match(el.textContent!, /No users yet\. Create the first admin with cairn user add/);
  ({ el } = open({}));
  await ready(el);
  assert.doesNotMatch(el.textContent!, /No users yet/);
});

test("providers render as links with a divider", async () => {
  const { el } = open({
    authMethods: ["password", "oidc"],
    providers: [{ name: "Google", href: "/auth/oidc/google" }],
  });
  await ready(el);
  const a = el.querySelector<HTMLAnchorElement>("a.provider")!;
  assert.equal(a.getAttribute("href"), "/auth/oidc/google");
  assert.equal(a.textContent, "Continue with Google");
  assert.match(el.textContent!, /or with a password/);
});

test("without the password method there is no form", async () => {
  const { el } = open({ authMethods: ["oidc"], providers: [{ name: "Google", href: "/auth/oidc/google" }] });
  await ready(el);
  assert.equal(el.querySelector("#login-password"), null);
  assert.doesNotMatch(el.textContent!, /or with a password/);
});

test("the CAPTCHA blocks sign-in until it has a token", async () => {
  let opts: Record<string, unknown> = {};
  (window as unknown as { grecaptcha: unknown }).grecaptcha = {
    render: (_el: HTMLElement, o: Record<string, unknown>) => {
      opts = o;
      return 0;
    },
    reset: () => {},
  };
  const { log, el } = open({ captcha: { provider: "recaptcha", siteKey: "site-key-placeholder" } });
  await ready(el);
  await waitFor(() => assert.equal(opts.sitekey, "site-key-placeholder"));
  const button = el.querySelector<HTMLButtonElement>('button[type="submit"]')!;
  assert.equal(button.disabled, true);
  (opts.callback as (t: string) => void)("token-123");
  await waitFor(() => assert.equal(button.disabled, false));
  type(el, "#login-email", "admin@example.com");
  type(el, "#login-password", "Abcdef1!");
  await submit(el);
  await waitFor(() => assert.ok(log.some((r) => r.method === "POST")));
  const body = await log.find((r) => r.method === "POST")!.json();
  assert.equal(body.captchaToken, "token-123");
});
