import "./test/dom";
import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { App } from "./app";
import { Shell } from "./components/shell";
import { SessionContext } from "./state/session";
import { api } from "./lib/api";
import { setLang } from "./i18n/index";
import { flush, json, render, stubFetch, waitFor } from "./test/dom";
import type { Config, User } from "./lib/types";

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

function start(at: string, me: User | null, cfg: Partial<Config> = {}, extra = {}) {
  history.replaceState(null, "", at);
  return stubFetch({
    "GET /api/config": () => json({ ...config, ...cfg }),
    "GET /api/me": () => json({ user: me }),
    ...extra,
  });
}

afterEach(() => {
  setLang("en");
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.lang = "en";
});

test("signed out goes to the sign-in page with next", async () => {
  start("/files/photos", null);
  render(<App />);
  await waitFor(() => assert.equal(location.pathname + location.search, "/login?next=%2Ffiles%2Fphotos"));
});

test("signed in on the sign-in page goes to next", async () => {
  start("/login?next=%2Ffiles%2Fphotos", admin);
  render(<App />);
  await waitFor(() => assert.equal(location.pathname, "/files/photos"));
});

test("signed in on the sign-in page without next goes to my files", async () => {
  start("/login", admin);
  render(<App />);
  await waitFor(() => assert.equal(location.pathname, "/files/"));
});

test("a session ending later keeps the place", async () => {
  start("/files/photos", admin, {}, {
    "GET /api/files/photos": () => json({ error: "signed_out", message: "Sign in first." }, 403),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /My files/));
  assert.equal(location.pathname, "/files/photos");
  await api("GET", "/api/files/photos").catch(() => {});
  await waitFor(() => assert.equal(location.pathname + location.search, "/login?next=%2Ffiles%2Fphotos"));
});

test("theme order: user, then server default, then the device", async () => {
  start("/files/", { ...admin, theme: "light" }, { theme: "dark" });
  render(<App />);
  await waitFor(() => assert.equal(document.documentElement.dataset.theme, "light"));

  start("/files/", admin, { theme: "dark" });
  render(<App />);
  await waitFor(() => assert.equal(document.documentElement.dataset.theme, "dark"));

  start("/files/", admin, { theme: null });
  render(<App />);
  await waitFor(() => assert.equal(document.documentElement.hasAttribute("data-theme"), false));
});

test("language is applied", async () => {
  start("/files/", { ...admin, lang: "pt-BR" });
  const el = render(<App />);
  await waitFor(() => assert.equal(document.documentElement.lang, "pt-BR"));
  assert.match(el.textContent!, /Meus arquivos/);
});

test("offline shows a panel and Try again reloads", async () => {
  history.replaceState(null, "", "/files/");
  let up = false;
  stubFetch({
    "GET /api/config": () => {
      if (!up) throw new TypeError("Failed to fetch");
      return json(config);
    },
    "GET /api/me": () => json({ user: admin }),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /Can't reach the server/));
  up = true;
  const retry = [...el.querySelectorAll("button")].find((b) => b.textContent === "Try again")!;
  retry.click();
  await waitFor(() => assert.match(el.textContent!, /My files/));
});

function shellWith(user: User, space?: { used: number; total: number }) {
  return render(
    <SessionContext.Provider
      value={{ session: { config, me: { user } }, error: null, reload: async () => {}, setUser: () => {} }}
    >
      <Shell space={space}>
        <p>content</p>
      </Shell>
    </SessionContext.Provider>,
  );
}

test("admins see disk use", () => {
  const space = { used: 412 * 1024 ** 3, total: 1024 ** 4 };
  assert.match(shellWith(admin, space).textContent!, /412 GB of 1 TB used/);
  const reader = { ...admin, permissions: { read: true, write: false, share: false, admin: false } };
  assert.doesNotMatch(shellWith(reader, space).textContent!, /used/);
});

test("sign out posts and goes to the sign-in page", async () => {
  const log = start("/files/a", admin, {}, { "POST /api/auth/logout": () => json({ next: null }) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector('button[aria-label="Sign out"]')));
  const out = el.querySelector<HTMLButtonElement>('button[aria-label="Sign out"]')!;
  out.click();
  await waitFor(() => assert.equal(location.pathname, "/login"));
  assert.ok(log.some((r) => r.method === "POST" && r.url.endsWith("/api/auth/logout")));
});
