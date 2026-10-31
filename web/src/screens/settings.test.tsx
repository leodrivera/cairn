import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { json, render, waitFor } from "../test/dom";
import { admin, reader, startAt } from "../test/fixtures";

function navLabels(el: HTMLElement): string[] {
  return [...el.querySelectorAll(".settings-nav a")].map((a) => a.textContent!.trim());
}

test("admins see every settings page", async () => {
  startAt("/settings/server", admin, { "GET /api/server": () => json({}) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".settings-nav")));
  assert.deepEqual(navLabels(el), ["Back to files", "Profile", "Users", "Server"]);
  assert.equal(el.querySelector('.settings-nav [aria-current="page"]')!.textContent!.trim(), "Server");
});

test("shared links is a place beside My files, not a settings page", async () => {
  startAt("/links", admin, { "GET /api/shares": () => json({ shares: [] }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /Every link that is currently active/));
  assert.ok(!el.querySelector(".settings-nav"));
  const places = [...el.querySelectorAll(".places a")].map((a) => a.textContent!.trim());
  assert.deepEqual(places, ["My files", "Shared with me", "Shared links", "Trash"]);
  assert.equal(el.querySelector('.places [aria-current="page"]')!.textContent!.trim(), "Shared links");
});

test("the old address /settings/links opens the shared links place", async () => {
  startAt("/settings/links", reader, { "GET /api/shares": () => json({ shares: [] }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /No share links yet\./));
  assert.ok(!el.querySelector(".settings-nav"));
  assert.equal(el.querySelector('.places [aria-current="page"]')!.textContent!.trim(), "Shared links");
});

test("non-admins have no Users or Server, and get Profile instead", async () => {
  startAt("/settings/users", reader);
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /Signed in as guest@example\.com/));
  assert.deepEqual(navLabels(el), ["Back to files", "Profile"]);
});

test("share links: the list comes from the server", async () => {
  const log = startAt("/links", admin, { "GET /api/shares": () => json({ shares: [] }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /No share links yet\./));
  assert.ok(log.some((r) => new URL(r.url).pathname === "/api/shares"));
});
