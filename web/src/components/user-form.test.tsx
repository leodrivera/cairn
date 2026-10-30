import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { UserForm } from "./user-form";
import { buttonNamed, byLabel, flush, json, render, setValue, stubFetch, waitFor } from "../test/dom";
import { reader } from "../test/fixtures";

function check(el: HTMLElement, label: string): HTMLInputElement {
  return byLabel(el, label);
}

function tick(box: HTMLInputElement) {
  box.click();
}

test("write and share tick read, which stays on while they are", async () => {
  const el = render(<UserForm sso={false} onSaved={() => {}} onClose={() => {}} />);
  const read = check(el, "Read files");
  const write = check(el, "Upload, rename, move and delete");
  const share = check(el, "Create share links");
  read.click();
  await flush();
  assert.equal(read.checked, false);
  tick(write);
  await flush();
  assert.equal(read.checked, true);
  assert.equal(read.disabled, true);
  tick(write);
  await flush();
  assert.equal(read.disabled, false);
  tick(share);
  await flush();
  assert.equal(read.checked, true);
  assert.equal(read.disabled, true);
});

test("a new user posts the form", async () => {
  const bodies: unknown[] = [];
  stubFetch({
    "POST /api/users": async (req) => {
      bodies.push(await req.json());
      return json({ user: { ...reader, id: 7 } }, 201);
    },
  });
  let saved = 0;
  const el = render(<UserForm sso={false} onSaved={() => saved++} onClose={() => {}} />);
  assert.match(el.textContent!, /The account\. Used in the password form and matched against the address an OAuth or OIDC provider reports\./);
  assert.match(el.textContent!, /The folder this user sees as their root, inside the server root\./);
  setValue(byLabel(el, "Name"), "Guest");
  setValue(byLabel(el, "E-mail"), "guest@example.com");
  setValue(byLabel(el, "Password"), "Guest-pass1!");
  setValue(byLabel(el, "Scope"), "/public");
  await flush();
  buttonNamed(el, "Create user").click();
  await waitFor(() => assert.equal(saved, 1));
  assert.deepEqual(bodies, [
    {
      name: "Guest",
      email: "guest@example.com",
      password: "Guest-pass1!",
      scope: "/public",
      permissions: { read: true, write: false, share: false, admin: false },
    },
  ]);
});

test("errors come by code", async () => {
  stubFetch({
    "POST /api/users": () => json({ error: "email_taken", message: "Another user already has this e-mail." }, 409),
  });
  const el = render(<UserForm sso={true} onSaved={() => {}} onClose={() => {}} />);
  setValue(byLabel(el, "Name"), "Guest");
  setValue(byLabel(el, "E-mail"), "guest@example.com");
  await flush();
  buttonNamed(el, "Create user").click();
  await waitFor(() => assert.match(el.textContent!, /Another user already has this e-mail\./));
});

test("a weak password is refused before sending", async () => {
  const log = stubFetch({});
  const el = render(<UserForm sso={true} onSaved={() => {}} onClose={() => {}} />);
  setValue(byLabel(el, "Name"), "Guest");
  setValue(byLabel(el, "E-mail"), "guest@example.com");
  setValue(byLabel(el, "Password"), "weak");
  await flush();
  buttonNamed(el, "Create user").click();
  await waitFor(() => assert.match(el.textContent!, /The password needs at least 8 characters/));
  assert.equal(log.length, 0);
});

test("editing sends only what changed, then the new password", async () => {
  const calls: [string, string, unknown][] = [];
  stubFetch({
    "PATCH /api/users/2": async (req) => {
      calls.push([req.method, new URL(req.url).pathname, await req.json()]);
      return json({ user: { ...reader, scope: "/docs" } });
    },
    "PUT /api/users/2/password": async (req) => {
      calls.push([req.method, new URL(req.url).pathname, await req.json()]);
      return json({ ok: true });
    },
  });
  let saved = 0;
  const el = render(<UserForm user={reader} sso={true} onSaved={() => saved++} onClose={() => {}} />);
  assert.equal(byLabel(el, "E-mail").value, "guest@example.com");
  setValue(byLabel(el, "Scope"), "/docs");
  setValue(byLabel(el, "Set a new password"), "Guest-pass1!");
  await flush();
  buttonNamed(el, "Save changes").click();
  await waitFor(() => assert.equal(saved, 1));
  assert.deepEqual(calls, [
    ["PATCH", "/api/users/2", { scope: "/docs" }],
    ["PUT", "/api/users/2/password", { password: "Guest-pass1!" }],
  ]);
});

test("remove password shows only with SSO and a password", async () => {
  const log = stubFetch({ "DELETE /api/users/2/password": () => json({ ok: true }) });
  let el = render(<UserForm user={reader} sso={false} onSaved={() => {}} onClose={() => {}} />);
  assert.ok(![...el.querySelectorAll("button")].some((b) => b.textContent === "Remove password"));
  let changed = 0;
  let closed = 0;
  el = render(
    <UserForm user={reader} sso={true} onSaved={() => {}} onChanged={() => changed++} onClose={() => closed++} />,
  );
  setValue(byLabel(el, "Scope"), "/docs");
  await flush();
  buttonNamed(el, "Remove password").click();
  await waitFor(() => assert.equal(changed, 1));
  assert.ok(log.some((r) => r.method === "DELETE"));
  // The form stays open with the other edit kept, and the button is gone.
  assert.equal(closed, 0);
  assert.equal(byLabel(el, "Scope").value, "/docs");
  assert.ok(![...el.querySelectorAll("button")].some((b) => b.textContent === "Remove password"));
});

test("the form opens with Name focused", async () => {
  const el = render(<UserForm sso={false} onSaved={() => {}} onClose={() => {}} />);
  await flush();
  assert.ok(document.activeElement === byLabel(el, "Name"));
});
