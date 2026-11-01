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
    "GET /api/grants": () => json({ grants: [] }),
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

test("without a public address the list says so once", async () => {
  const { el } = await screen([link({ url: null }), backup]);
  await waitFor(() => assert.ok(el.querySelector(".links-list")));
  assert.equal(el.textContent!.match(/no public address/g)?.length, 1);
  const { el: el2 } = await screen([link()]);
  await waitFor(() => assert.ok(el2.querySelector(".links-list")));
  assert.doesNotMatch(el2.textContent!, /no public address/);
});

const given = {
  id: "AbCdEfGhIjKlMnOpQrSt-_",
  name: "team",
  kind: "folder",
  path: "/team",
  permission: "read",
  effective: "read",
  expiresAt: null,
  createdAt: now() - 60,
  owner: { id: 1, name: "Admin" },
  recipient: { id: 2, name: "Bob", email: "bob@example.com" },
};

test("grants_section_lists_and_revokes", async () => {
  let grants = [given];
  const { el, log } = await screen([], admin, {
    "GET /api/grants": () => json({ grants }),
    "DELETE /api/grants/AbCdEfGhIjKlMnOpQrSt-_": () => {
      grants = [];
      return new Response(null, { status: 204 });
    },
  });
  await waitFor(() => assert.ok(el.querySelector(".grants-list")));
  const text = el.querySelector(".grants-list")!.textContent!;
  for (const part of ["team", "/team", "Bob", "bob@example.com", "Never"]) assert.ok(text.includes(part), part);
  el.querySelector<HTMLButtonElement>('button[aria-label="Stop sharing team with Bob"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Stop sharing team with Bob\?/));
  el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!.click();
  await waitFor(() => assert.ok(log.some((r) => r.method === "DELETE")));
  await waitFor(() => assert.ok(!el.querySelector(".grants-list")));
});

test("grants_change_permission_and_expiry", async () => {
  const bodies: unknown[] = [];
  const { el } = await screen([], admin, {
    "GET /api/grants": () => json({ grants: [given] }),
    "PATCH /api/grants/AbCdEfGhIjKlMnOpQrSt-_": async (req: Request) => {
      const body = await req.json();
      bodies.push(body);
      return json({ grant: { ...given, ...body } });
    },
  });
  await waitFor(() => assert.ok(el.querySelector(".grants-list")));
  const perm = el.querySelector<HTMLSelectElement>('select[aria-label="Access for Bob to team"]')!;
  perm.value = "write";
  perm.dispatchEvent(new Event("change", { bubbles: true }));
  await waitFor(() => assert.deepEqual(bodies, [{ permission: "write" }]));
  const exp = el.querySelector<HTMLSelectElement>('select[aria-label="Access for Bob to team ends"]')!;
  exp.value = "7";
  exp.dispatchEvent(new Event("change", { bubbles: true }));
  await waitFor(() => assert.deepEqual(bodies[1], { expiresDays: 7 }));
});

test("admin_sees_all_grants", async () => {
  const { el } = await screen([], admin, {
    "GET /api/grants": () => json({ grants: [{ ...given, owner: { id: 9, name: "Ana" } }] }),
  });
  await waitFor(() => assert.ok(el.querySelector(".grants-list")));
  assert.match(el.querySelector(".grants-list")!.textContent!, /Shared by Ana/);
});

const options = (s: HTMLSelectElement) => [...s.options].map((o) => [o.parentElement!.tagName === "OPTGROUP" ? (o.parentElement as HTMLOptGroupElement).label : "", o.textContent]);

test("grant_expiry_shows_current_then_changes", async () => {
  const until = new Date(2027, 2, 14, 12).getTime() / 1000;
  const { el } = await screen([], admin, {
    "GET /api/grants": () =>
      json({ grants: [given, { ...given, id: "ZyXwVuTsRqPoNmLkJiHg-_", expiresAt: until, recipient: { id: 3, name: "Ana", email: "ana@example.com" } }] }),
  });
  await waitFor(() => assert.ok(el.querySelector(".grants-list")));
  const never = el.querySelector<HTMLSelectElement>('select[aria-label="Access for Bob to team ends"]')!;
  // No "Keep:" prefix, and Never is not offered twice.
  assert.deepEqual(options(never), [
    ["", "Never"],
    ["Change to", "In 24 hours"],
    ["Change to", "In 7 days"],
    ["Change to", "In 30 days"],
  ]);
  const dated = el.querySelector<HTMLSelectElement>('select[aria-label="Access for Ana to team ends"]')!;
  assert.deepEqual(options(dated), [
    ["", "Until Mar 14, 2027"],
    ["Change to", "In 24 hours"],
    ["Change to", "In 7 days"],
    ["Change to", "In 30 days"],
    ["Change to", "Never"],
  ]);
});
