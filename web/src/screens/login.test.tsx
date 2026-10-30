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

function open(
  cfg: Partial<Config>,
  login?: (req: Request) => Response | Promise<Response>,
  at = "/login",
  me: Record<string, unknown> = { user: null },
) {
  history.replaceState(null, "", at);
  const log = stubFetch({
    "GET /api/config": () => json({ ...config, ...cfg }),
    "GET /api/me": () => json(me),
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
  assert.ok(document.activeElement === pw, "focus on the password");
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
  assert.ok(!el.querySelector("#login-password"));
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

function alertText(el: HTMLElement): string {
  return el.querySelector('[role="alert"]')?.textContent ?? "";
}

test("shows the sso error", async () => {
  const { el } = open({}, undefined, "/login?error=sso");
  await ready(el);
  await waitFor(() => assert.equal(alertText(el), "Signing in with the provider did not work. Try again."));
});

test("shows no account with the e-mail", async () => {
  const { el } = open({}, undefined, "/login?error=no-account", { user: null, deniedEmail: "nobody@example.com" });
  await ready(el);
  await waitFor(() =>
    assert.equal(alertText(el), "No account for nobody@example.com. Ask the person who runs this server to add you."),
  );
});

test("no account without the cookie", async () => {
  const { el } = open({}, undefined, "/login?error=no-account");
  await ready(el);
  await waitFor(() =>
    assert.equal(alertText(el), "No account for this e-mail. Ask the person who runs this server to add you."),
  );
});

test("the error leaves the URL", async () => {
  const { el } = open({}, undefined, "/login?error=sso&next=%2Ffiles%2Fa");
  await ready(el);
  await waitFor(() => assert.ok(alertText(el).length > 0));
  await waitFor(() => assert.ok(!location.search.includes("error")));
  assert.ok(location.search.includes("next=%2Ffiles%2Fa"));
});

test("provider buttons link to their href", async () => {
  const { el } = open({
    authMethods: ["password", "oauth", "oidc"],
    providers: [
      { name: "Google", href: "/auth/oauth/google" },
      { name: "Example ID", href: "/auth/oidc/fake" },
    ],
  });
  await ready(el);
  const links = [...el.querySelectorAll<HTMLAnchorElement>("a.provider")];
  assert.deepEqual(
    links.map((a) => [a.getAttribute("href"), a.textContent]),
    [
      ["/auth/oauth/google", "Continue with Google"],
      ["/auth/oidc/fake", "Continue with Example ID"],
    ],
  );
});

test("pt-BR sso error", async () => {
  const before = Object.getOwnPropertyDescriptor(navigator, "languages");
  Object.defineProperty(navigator, "languages", { value: ["pt-BR"], configurable: true });
  try {
    const { el } = open({}, undefined, "/login?error=sso");
    await waitFor(() => assert.equal(alertText(el), "Não foi possível entrar pelo provedor. Tente de novo."));
  } finally {
    if (before) Object.defineProperty(navigator, "languages", before);
    else delete (navigator as { languages?: unknown }).languages;
  }
});
