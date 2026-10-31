import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, byLabel, flush, json, render, setValue, waitFor, passwordToggles } from "../test/dom";
import { admin, startAt } from "../test/fixtures";

async function profile(el: HTMLElement) {
  await waitFor(() => assert.match(el.textContent!, /Signed in as/));
}

test("profile sends only the changed fields and applies the theme", async () => {
  const bodies: unknown[] = [];
  const log = startAt("/settings/profile", admin, {
    "PATCH /api/me": async (req) => {
      const body = await req.json();
      bodies.push(body);
      return json({ user: { ...admin, ...body } });
    },
  });
  const el = render(<App />);
  await profile(el);
  assert.match(el.textContent!, /Signed in as admin@example\.com, with a password\./);
  assert.equal(byLabel(el, "Display name").value, "Admin");
  setValue(byLabel<HTMLSelectElement>(el, "Theme"), "light");
  await flush();
  buttonNamed(el, "Save changes").click();
  await waitFor(() => assert.deepEqual(bodies, [{ theme: "light" }]));
  await waitFor(() => assert.equal(document.documentElement.dataset.theme, "light"));
  assert.equal(log.filter((r) => r.method === "PATCH").length, 1);
});

test("profile names the sign-in method", async () => {
  startAt("/settings/profile", { ...admin, hasPassword: false }, {
    "GET /api/me": () => json({ user: { ...admin, hasPassword: false }, method: "oidc" }),
  });
  const el = render(<App />);
  await profile(el);
  assert.match(el.textContent!, /Signed in as admin@example\.com, with OIDC\./);
  assert.ok(!el.textContent!.includes("Change password"));
});

test("profile checks the password rule before sending", async () => {
  const log = startAt("/settings/profile", admin);
  const el = render(<App />);
  await profile(el);
  setValue(byLabel(el, "Current password"), "Old-pass1!");
  setValue(byLabel(el, "New password"), "short");
  setValue(byLabel(el, "Repeat new password"), "short");
  await flush();
  buttonNamed(el, "Change password").click();
  await waitFor(() => assert.match(el.textContent!, /The password needs at least 8 characters/));
  setValue(byLabel(el, "New password"), "Longer-pass1!");
  setValue(byLabel(el, "Repeat new password"), "Longer-pass2!");
  await flush();
  buttonNamed(el, "Change password").click();
  await waitFor(() => assert.match(el.textContent!, /The new passwords do not match\./));
  assert.ok(!log.some((r) => r.method === "PUT"));
});

test("profile: a wrong current password says so; a good change clears the fields", async () => {
  let answer = json({ error: "wrong_password", message: "The current password is wrong." }, 400);
  const bodies: unknown[] = [];
  startAt("/settings/profile", admin, {
    "PUT /api/me/password": async (req) => {
      bodies.push(await req.json());
      return answer;
    },
  });
  const el = render(<App />);
  await profile(el);
  assert.match(el.textContent!, /Changing it signs you out on your other devices\./);
  setValue(byLabel(el, "Current password"), "Old-pass1!");
  setValue(byLabel(el, "New password"), "Longer-pass1!");
  setValue(byLabel(el, "Repeat new password"), "Longer-pass1!");
  await flush();
  buttonNamed(el, "Change password").click();
  await waitFor(() => assert.match(el.textContent!, /The current password is wrong\./));
  assert.deepEqual(bodies, [{ current: "Old-pass1!", next: "Longer-pass1!" }]);
  answer = json({ ok: true });
  buttonNamed(el, "Change password").click();
  await waitFor(() => assert.match(el.textContent!, /Password changed\./));
  assert.equal(byLabel(el, "Current password").value, "");
});

test("password_field_has_toggle", async () => {
  startAt("/settings/profile", admin);
  const el = render(<App />);
  await profile(el);
  assert.equal(passwordToggles(el), 3);
});
