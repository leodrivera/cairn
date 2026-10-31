import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, flush, json, render, waitFor } from "../test/dom";
import { admin, startAt } from "../test/fixtures";
import type { TrashEntry } from "../lib/types";

const at = Math.floor(new Date(2026, 9, 24, 8, 0).getTime() / 1000);

function entry(id: string, name: string, extra: Partial<TrashEntry> = {}): TrashEntry {
  return {
    id,
    name,
    kind: "file",
    size: 812,
    from: "/docs",
    deletedBy: { id: 1, name: "Admin" },
    deletedAt: at,
    expiresAt: at + 30 * 86_400,
    canDelete: true,
    ...extra,
  };
}

function open(entries: TrashEntry[], routes: Record<string, (r: Request) => Response | Promise<Response>> = {}) {
  let list = entries;
  const log = startAt("/trash", admin, {
    "GET /api/trash": () => json({ entries: list }),
    ...routes,
  });
  const el = render(<App />);
  return { el, log, set: (l: TrashEntry[]) => (list = l) };
}

async function ready(el: HTMLElement) {
  await waitFor(() => assert.ok(el.querySelector(".trash-list, .state")));
}

test("lists_entries", async () => {
  const { el } = open([entry("e1", "notes.txt"), entry("e2", "Archive", { kind: "folder", size: 2048, from: "/" })]);
  await ready(el);
  assert.match(el.querySelector("h1")!.textContent!, /Trash/);
  const text = el.querySelector(".trash-list")!.textContent!;
  assert.match(text, /notes\.txt/);
  assert.match(text, /\/docs/);
  assert.match(text, /Admin/);
  assert.match(text, /Oct 24/);
  assert.match(text, /812 B/);
  assert.match(text, /Archive/);
  assert.ok(el.querySelector('.sidebar a[href="/trash"][aria-current="page"]'));
});

test("restore_one", async () => {
  const { el, log, set } = open([entry("e1", "notes.txt")], {
    "POST /api/trash/restore": () => json({ restored: [{ id: "e1", path: "docs/notes.txt", name: "notes.txt" }], failed: [] }),
  });
  await ready(el);
  set([]);
  buttonNamed(el, "Restore").click();
  await waitFor(() => assert.match(el.textContent!, /Restored 1 item\./));
  assert.deepEqual(await log.find((r) => r.method === "POST")!.json(), { ids: ["e1"] });
  await waitFor(() => assert.match(el.textContent!, /The trash is empty/));
});

test("delete_one_forever", async () => {
  const { el, log } = open([entry("e1", "notes.txt")], {
    "DELETE /api/trash": () => json({ deleted: 1, failed: [] }),
  });
  await ready(el);
  el.querySelector<HTMLButtonElement>('button[aria-label="Delete notes.txt permanently"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Delete notes\.txt permanently\?/));
  assert.ok(!log.some((r) => r.method === "DELETE"));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.ok(log.some((r) => r.method === "DELETE")));
  assert.deepEqual(await log.find((r) => r.method === "DELETE")!.json(), { ids: ["e1"] });
  await waitFor(() => assert.match(el.textContent!, /Deleted 1 item permanently\./));
});

test("empty_trash", async () => {
  const { el, log } = open([entry("e1", "a.txt"), entry("e2", "b.txt")], {
    "POST /api/trash/empty": () => json({ deleted: 2 }),
  });
  await ready(el);
  buttonNamed(el, "Empty trash").click();
  await waitFor(() => assert.match(el.textContent!, /2 items will be deleted permanently\./));
  assert.ok(!log.some((r) => r.url.endsWith("/api/trash/empty")));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.ok(log.some((r) => r.url.endsWith("/api/trash/empty"))));
});

test("grant_entry_has_no_delete", async () => {
  const { el } = open([entry("e1", "a.txt", { canDelete: false })]);
  await ready(el);
  assert.ok(!el.querySelector('button[aria-label="Delete a.txt permanently"]'));
  assert.ok(![...el.querySelectorAll("button")].some((b) => b.textContent?.trim() === "Empty trash"));
  assert.ok(buttonNamed(el, "Restore"));
});

test("empty_state", async () => {
  const { el } = open([]);
  await ready(el);
  assert.match(el.textContent!, /The trash is empty/);
  await flush();
});
