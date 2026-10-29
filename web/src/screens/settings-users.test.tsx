import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, json, render, waitFor } from "../test/dom";
import { admin, reader, startAt } from "../test/fixtures";

test("users are listed, and delete asks first", async () => {
  let lists = 0;
  const log = startAt("/settings/users", admin, {
    "GET /api/users": () => {
      lists++;
      return json({ users: lists === 1 ? [admin, { ...reader, scope: "/public" }] : [admin] });
    },
    "DELETE /api/users/2": () => new Response(null, { status: 204 }),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /guest@example\.com/));
  const rows = [...el.querySelectorAll(".users-table tbody tr")];
  assert.equal(rows.length, 2);
  assert.match(rows[1].textContent!, /GUGuestguest@example\.com\/publicReadWriteShareAdmin/);
  assert.equal(rows[1].querySelectorAll(".chip.on").length, 1);
  assert.equal(rows[0].querySelectorAll(".chip.on").length, 4);
  assert.ok(el.querySelector('button[aria-label="Edit Guest"]'));
  el.querySelector<HTMLButtonElement>('button[aria-label="Delete Guest"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Delete Guest\?/));
  assert.match(el.querySelector('[role="dialog"]')!.textContent!, /Their share links stop working\. Their files stay\./);
  assert.ok(!log.some((r) => r.method === "DELETE"));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.match(el.textContent!, /Deleted Guest\./));
  await waitFor(() => assert.ok(!el.textContent!.includes("guest@example.com")));
});

test("the last admin cannot be deleted; the dialog says why", async () => {
  startAt("/settings/users", admin, {
    "GET /api/users": () => json({ users: [admin] }),
    "DELETE /api/users/1": () =>
      json({ error: "last_admin", message: "Cairn needs at least one admin. Make another user an admin first." }, 400),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /admin@example\.com/));
  el.querySelector<HTMLButtonElement>('button[aria-label="Delete Admin"]')!.click();
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"]')));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.match(el.textContent!, /Cairn needs at least one admin/));
});

test("New user opens the form", async () => {
  startAt("/settings/users", admin, { "GET /api/users": () => json({ users: [admin] }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /admin@example\.com/));
  buttonNamed(el, "New user").click();
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"]')));
  assert.ok(buttonNamed(el, "Create user"));
});
