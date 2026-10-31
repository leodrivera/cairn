import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, byLabel, flush, json, render, setValue, waitFor } from "../test/dom";
import { admin, startAt } from "../test/fixtures";

const server = {
  root: "/srv",
  publicUrl: null,
  addr: "0.0.0.0:8080",
  authMethods: ["password", "oidc"],
  captcha: "none",
  uploadMax: 0,
  editMax: 2097152,
  threads: 2,
  trustedProxies: [],
  log: "info",
};

test("server rows and the CAPTCHA warning", async () => {
  startAt("/settings/server", admin, {
    "GET /api/me": () => json({ user: admin, method: "password", security: { captchaDisabled: true } }),
    "GET /api/server": () => json(server),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  assert.match(el.textContent!, /Values marked “set by the environment” come from the container\./);
  assert.match(el.textContent!, /Sign-in has no CAPTCHA\. Turn one on before you open Cairn to the internet: set CAIRN_CAPTCHA\./);
  const rows = [...el.querySelectorAll(".env-row")].map((r) => r.textContent);
  assert.deepEqual(rows, [
    "Server root/srvCAIRN_ROOT",
    "Public addressNot setCAIRN_PUBLIC_URL",
    "Listen address0.0.0.0:8080CAIRN_ADDR",
    "Sign-in methodsPassword, OIDCCAIRN_AUTH_METHODS",
    "CAPTCHAOffCAIRN_CAPTCHA",
    "Largest uploadNo limitCAIRN_UPLOAD_MAX",
    "Largest text file to edit2 MBCAIRN_EDIT_MAX",
    "Worker threads2CAIRN_THREADS",
    "Trusted proxiesNoneCAIRN_TRUSTED_PROXIES",
    "Log levelinfoCAIRN_LOG",
  ]);
});

test("no warning with a CAPTCHA; values are named", async () => {
  startAt("/settings/server", admin, {
    "GET /api/server": () =>
      json({ ...server, captcha: "turnstile", uploadMax: 1073741824, trustedProxies: ["10.0.0.0/8"], publicUrl: "https://files.example.com/" }),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  assert.ok(!el.textContent!.includes("Sign-in has no CAPTCHA"));
  assert.match(el.textContent!, /CAPTCHATurnstile/);
  assert.match(el.textContent!, /Largest upload1 GB/);
  assert.match(el.textContent!, /Trusted proxies10\.0\.0\.0\/8/);
  assert.match(el.textContent!, /https:\/\/files\.example\.com\//);
});

test("appearance saves with PUT /api/settings; Discard resets", async () => {
  const bodies: unknown[] = [];
  startAt(
    "/settings/server",
    admin,
    {
      "GET /api/server": () => json(server),
      "PUT /api/settings": async (req) => {
        const body = await req.json();
        bodies.push(body);
        return json({ settings: body });
      },
    },
    { theme: null, thumbnails: false },
  );
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  assert.match(el.textContent!, /Off by default to keep Cairn light\./);
  const theme = byLabel<HTMLSelectElement>(el, "Default theme");
  const thumbs = byLabel(el, "Image thumbnails");
  assert.equal(thumbs.getAttribute("role"), "switch");
  setValue(theme, "dark");
  thumbs.click();
  await flush();
  buttonNamed(el, "Discard").click();
  await flush();
  assert.equal(byLabel<HTMLSelectElement>(el, "Default theme").value, "");
  assert.equal(byLabel(el, "Image thumbnails").checked, false);
  setValue(byLabel<HTMLSelectElement>(el, "Default theme"), "dark");
  byLabel(el, "Image thumbnails").click();
  await flush();
  buttonNamed(el, "Save changes").click();
  await waitFor(() => assert.deepEqual(bodies, [{ theme: "dark", thumbnails: true }]));
  await waitFor(() => assert.match(el.textContent!, /Saved the server settings\./));
});

const trash = { days: 30, fromEnv: false, size: 5_242_880, taken: [] as string[] };

test("trash_days_field", async () => {
  const bodies: unknown[] = [];
  startAt("/settings/server", admin, {
    "GET /api/server": () => json({ ...server, trash }),
    "PUT /api/settings": async (req) => {
      const body = await req.json();
      bodies.push(body);
      return json({ settings: body });
    },
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  const days = byLabel(el, "Keep deleted items for (days)");
  assert.equal(days.value, "30");
  setValue(days, "7");
  await flush();
  buttonNamed(el, "Save changes").click();
  await waitFor(() => assert.equal(bodies.length, 1));
  assert.deepEqual(bodies[0], { theme: null, thumbnails: false, trashDays: 7 });
});

test("trash_days_from_env_read_only", async () => {
  startAt("/settings/server", admin, { "GET /api/server": () => json({ ...server, trash: { ...trash, days: 7, fromEnv: true } }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  const rows = [...el.querySelectorAll(".env-row")].map((r) => r.textContent);
  assert.ok(rows.includes("Trash7 daysCAIRN_TRASH_DAYS"), JSON.stringify(rows));
  assert.ok(!el.querySelector("#server-trash-days"));
});

test("trash_folder_taken_notice", async () => {
  startAt("/settings/server", admin, { "GET /api/server": () => json({ ...server, trash: { ...trash, taken: ["/photos"] } }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  assert.match(el.textContent!, /\/photos\/\.cairn-trash was not made by Cairn, so the trash is off there\. Rename that folder\./);
});

test("trash_size_shown", async () => {
  startAt("/settings/server", admin, { "GET /api/server": () => json({ ...server, trash }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /CAIRN_ROOT/));
  assert.match(el.textContent!, /In the trash now: 5 MB/);
});
