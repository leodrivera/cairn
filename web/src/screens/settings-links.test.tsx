import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, json, phoneWidth, render, waitFor } from "../test/dom";
import { admin, reader, startAt } from "../test/fixtures";
import type { Share } from "../lib/types";

const now = () => Math.floor(Date.now() / 1000);

function link(extra: Partial<Share> = {}): Share {
  return {
    id: "k7Qw3nTzAbCdEfGhIjKl-_",
    url: "https://files.example.com/s/k7Qw3nTzAbCdEfGhIjKl-_",
    path: "/photos/2026/trail-ridge.jpg",
    name: "trail-ridge.jpg",
    expiresAt: now() + 7 * 86_400 - 60,
    hasPassword: false,
    download: true,
    hits: 3,
    createdAt: now() - 60,
    owner: { id: 1, name: "Admin" },
    ...extra,
  };
}

const backup = link({
  id: "Pm4xVb9aAbCdEfGhIjKl-_",
  url: null,
  path: "/backups/site-backup.tar.gz",
  name: "site-backup.tar.gz",
  expiresAt: null,
  hasPassword: true,
  hits: 1,
});

function clipboard(write: (text: string) => Promise<void>): () => void {
  Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
  return () => delete (navigator as { clipboard?: unknown }).clipboard;
}

async function screen(shares: Share[] | (() => Response), user = admin, extra = {}) {
  const log = startAt("/settings/links", user, {
    "GET /api/shares": typeof shares === "function" ? shares : () => json({ shares }),
    ...extra,
  });
  const el = render(<App />);
  return { el, log };
}

test("lists links", async () => {
  const { el } = await screen([link(), backup]);
  await waitFor(() => assert.match(el.textContent!, /site-backup\.tar\.gz/));
  const text = el.querySelector(".links-list")!.textContent!;
  for (const part of ["trail-ridge.jpg", "/photos/2026", "/s/k7Qw3nTz", "In 7 days", "Never", "Password", "Open", "3 times", "1 time"]) {
    assert.ok(text.includes(part), `${part} in ${text}`);
  }
  const a = el.querySelector<HTMLAnchorElement>('.links-list a[href="https://files.example.com/s/k7Qw3nTzAbCdEfGhIjKl-_"]');
  assert.ok(a);
  assert.equal(a!.target, "_blank");
  assert.equal(a!.rel, "noopener noreferrer");
  assert.ok(el.querySelector(`.links-list a[href="${location.origin}/s/Pm4xVb9aAbCdEfGhIjKl-_"]`));
});

test("a link expiring soon is in the danger color", async () => {
  const { el } = await screen([link({ expiresAt: now() + 30 * 3600 })]);
  await waitFor(() => assert.match(el.textContent!, /Tomorrow/));
  assert.ok(el.querySelector(".links-list .soon"));
  const { el: el2 } = await screen([link({ expiresAt: now() + 3600 })]);
  await waitFor(() => assert.match(el2.textContent!, /Today/));
});

test("copy button", async () => {
  const written: string[] = [];
  const restore = clipboard(async (t) => void written.push(t));
  const { el } = await screen([link()]);
  await waitFor(() => assert.ok(el.querySelector('[aria-label="Copy link for trail-ridge.jpg"]')));
  el.querySelector<HTMLButtonElement>('[aria-label="Copy link for trail-ridge.jpg"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Link copied\./));
  assert.deepEqual(written, [link().url]);
  restore();
});

test("delete asks first", async () => {
  let list = [link(), backup];
  const { el, log } = await screen(() => json({ shares: list }), admin, {
    "DELETE /api/shares/k7Qw3nTzAbCdEfGhIjKl-_": () => {
      list = [backup];
      return new Response(null, { status: 204 });
    },
  });
  await waitFor(() => assert.ok(el.querySelector('[aria-label="Delete link for trail-ridge.jpg"]')));
  const del = () => el.querySelector<HTMLButtonElement>('[aria-label="Delete link for trail-ridge.jpg"]')!.click();
  del();
  await waitFor(() => assert.match(el.querySelector('[role="dialog"]')!.textContent!, /Delete the link to trail-ridge\.jpg\?/));
  assert.match(el.querySelector('[role="dialog"]')!.textContent!, /People who have it can no longer open it\./);
  buttonNamed(el.querySelector('[role="dialog"]')!, "Cancel").click();
  await waitFor(() => assert.ok(!el.querySelector('[role="dialog"]')));
  assert.ok(!log.some((r) => r.method === "DELETE"));
  del();
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"]')));
  buttonNamed(el.querySelector('[role="dialog"]')!, "Delete link").click();
  await waitFor(() => assert.match(el.textContent!, /Deleted the link to trail-ridge\.jpg\./));
  await waitFor(() => assert.ok(!el.querySelector('[aria-label="Delete link for trail-ridge.jpg"]')));
  assert.ok(log.some((r) => r.method === "DELETE"));
});

test("an admin sees who shared someone else's link", async () => {
  const { el } = await screen([link(), link({ id: "Zt6yQn1wAbCdEfGhIjKl-_", owner: { id: 7, name: "Ana" } })]);
  await waitFor(() => assert.match(el.textContent!, /Shared by Ana/));
  assert.equal(el.textContent!.match(/Shared by/g)!.length, 1);
});

test("empty state", async () => {
  const { el } = await screen([], reader);
  await waitFor(() => assert.match(el.textContent!, /No share links yet\./));
  assert.match(el.textContent!, /Links you create appear here\./);
});

test("load error offers Try again", async () => {
  let fail = true;
  const { el } = await screen(() =>
    fail ? json({ error: "internal", message: "x" }, 500) : json({ shares: [link()] }),
  );
  await waitFor(() => assert.match(el.textContent!, /Something went wrong on the server/));
  fail = false;
  buttonNamed(el, "Try again").click();
  await waitFor(() => assert.ok(el.querySelector(".links-list")));
});

test("phone layout stacks rows", async () => {
  const restore = phoneWidth();
  try {
    const { el } = await screen([link()]);
    await waitFor(() => assert.ok(el.querySelector(".links-list li")));
    assert.ok(!el.querySelector(".links-list table"));
    assert.ok(el.querySelector('[aria-label="Copy link for trail-ridge.jpg"]'));
    assert.ok(el.querySelector('[aria-label="Delete link for trail-ridge.jpg"]'));
  } finally {
    restore();
  }
});

test("pt-BR rows", async () => {
  const pt = { ...admin, lang: "pt-BR" as const };
  const { el } = await screen([link()], pt);
  await waitFor(() => assert.match(el.textContent!, /Em 7 dias/));
  assert.match(el.textContent!, /3 vezes/);
});

test("a folder that is not UTF-8 shows readable", async () => {
  const { el } = await screen([link({ path: "/\u0000UmVsYXTzcmlv/notes.txt", name: "notes.txt" })]);
  await waitFor(() => assert.ok(el.querySelector(".links-list .link-file")));
  const text = el.querySelector(".links-list .link-file")!.textContent!;
  assert.ok(text.includes("/Relat\uFFFDrio"), text);
  assert.ok(!text.includes("\u0000"), text);
});
