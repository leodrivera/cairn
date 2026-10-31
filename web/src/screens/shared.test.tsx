import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, json, render, waitFor } from "../test/dom";
import { file, listing, reader, received, startAt } from "../test/fixtures";

const ID = "AbCdEfGhIjKlMnOpQrSt-_";

test("lists_received", async () => {
  startAt("/shared", reader, {
    "GET /api/grants/received": () =>
      json({ grants: [received(), received({ id: "ZbCdEfGhIjKlMnOpQrSt-_", name: "notes.txt", kind: "file", permission: "read", effective: "read" })] }),
  });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".shared-list")));
  const text = el.querySelector(".shared-list")!.textContent!;
  for (const part of ["team", "notes.txt", "Shared by Ana", "Can edit", "Can view", "Never"]) assert.ok(text.includes(part), part);
  assert.ok(el.querySelector('.sidebar a[href="/shared"][aria-current="page"]'));
});

test("leave_removes", async () => {
  let list = [received()];
  const log = startAt("/shared", reader, {
    "GET /api/grants/received": () => json({ grants: list }),
    [`DELETE /api/grants/${ID}`]: () => {
      list = [];
      return new Response(null, { status: 204 });
    },
  });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".shared-list")));
  el.querySelector<HTMLButtonElement>('button[aria-label="Leave team"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Leave team\?/));
  assert.ok(!log.some((r) => r.method === "DELETE"));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.ok(log.some((r) => r.method === "DELETE")));
  await waitFor(() => assert.match(el.textContent!, /Nothing is shared with you/));
});

test("opens_folder_and_file", async () => {
  startAt("/shared", reader, {
    "GET /api/grants/received": () => json({ grants: [received()] }),
    [`GET /api/g/${ID}`]: () => json({ grant: received() }),
    [`GET /api/g/${ID}/files`]: listing("", [file("a.txt", { mime: "text/plain" })]),
  });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".shared-list a")));
  const a = el.querySelector<HTMLAnchorElement>(".shared-list a")!;
  assert.equal(a.getAttribute("href"), `/shared/${ID}`);
  a.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  await waitFor(() => assert.match(el.textContent!, /a\.txt/));
  assert.equal(location.pathname, `/shared/${ID}`);
});

test("empty_state", async () => {
  startAt("/shared", reader, { "GET /api/grants/received": () => json({ grants: [] }) });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /Nothing is shared with you/));
  assert.ok(buttonNamed);
});
