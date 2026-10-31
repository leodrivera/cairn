import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { flush, json, phoneWidth, render, waitFor } from "../test/dom";
import { admin, file, folder, listing, reader, startAt } from "../test/fixtures";
import { resetUploads } from "../state/uploads";
import type { Transport } from "../lib/uploads";

function rows(el: HTMLElement): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>('.list-body [role="row"]')];
}

/** Waits until the listing (or a state panel) is on screen. */
async function shown(el: HTMLElement) {
  await waitFor(() => assert.ok(el.querySelector('.list-body [role="row"], .state')));
}

function button(el: HTMLElement, text: string): HTMLButtonElement {
  const b = [...el.querySelectorAll("button")].find((x) => x.textContent?.trim() === text);
  assert.ok(b, `no button "${text}"`);
  return b as HTMLButtonElement;
}

function click(target: Element, init: MouseEventInit = {}) {
  target.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));
}

const sample = [
  folder("2026"),
  folder("raw"),
  file("trail-ridge.jpg", { mime: "image/jpeg" }),
  file("notes.txt", { mime: "text/plain", size: 812 }),
  file("Report #1 (50%) ✓ café.txt", { mime: "text/plain" }),
  file("site-backup.tar.gz", { mime: "application/gzip" }),
];

test("fifty thousand rows are windowed", async () => {
  const many = Array.from({ length: 50_000 }, (_, i) => file(`file-${String(i).padStart(5, "0")}`));
  startAt("/files/many", admin, { "GET /api/files/many": listing("many", many) });
  const el = render(<App />);
  await shown(el);
  await waitFor(() => assert.ok(rows(el).length > 0));
  assert.ok(rows(el).length < 60, `${rows(el).length} rows in the DOM`);
  const scroller = el.querySelector<HTMLElement>(".list-body")!;
  scroller.scrollTop = 26_000;
  scroller.dispatchEvent(new Event("scroll"));
  await waitFor(() => {
    const first = rows(el)[0].querySelector(".name")!.textContent!;
    assert.ok(first >= "file-00480", first);
  });
  assert.ok(rows(el).length < 60);
});

test("columns: size, folders without one, modified", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const r = rows(el);
  assert.equal(r.length, 6);
  assert.match(r[0].textContent!, /2026—Oct 24/);
  assert.match(r[3].textContent!, /notes\.txt812 BOct 24/);
});

test("folders navigate", async () => {
  startAt("/files/photos", admin, {
    "GET /api/files/photos": listing("photos", sample),
    "GET /api/files/photos/2026": listing("photos/2026", []),
  });
  const el = render(<App />);
  await shown(el);
  click(rows(el)[0].querySelector("a.name")!);
  await waitFor(() => assert.equal(location.pathname, "/files/photos/2026"));
  await waitFor(() => assert.match(el.textContent!, /This folder is empty/));
});

test("a text file opens the viewer", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  click(rows(el)[4].querySelector("a.name")!);
  await waitFor(() => assert.equal(location.pathname, "/view/Report%20%231%20(50%25)%20%E2%9C%93%20caf%C3%A9.txt"));
});

test("other files download", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const a = rows(el)[5].querySelector<HTMLAnchorElement>("a.name")!;
  assert.equal(a.getAttribute("href"), "/api/raw/site-backup.tar.gz?download=1");
  assert.equal(a.hasAttribute("download"), true);
});

test("breadcrumbs show raw names readably", async () => {
  startAt("/files/docs/%00Y2Fm6Q", admin, { "GET /api/files/docs/\u0000Y2Fm6Q": listing("docs/\u0000Y2Fm6Q", []) });
  const el = render(<App />);
  await shown(el);
  const crumbs = el.querySelector(".breadcrumbs")!;
  assert.equal(crumbs.textContent, "My files/docs/caf�");
  const links = [...crumbs.querySelectorAll("a")].map((a) => a.getAttribute("href"));
  assert.deepEqual(links, ["/files/", "/files/docs"]);
  assert.equal(crumbs.querySelector('[aria-current="page"]')!.textContent, "caf�");
});

test("empty state offers Upload only with write", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", []) });
  let el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /This folder is empty/));
  assert.ok(el.querySelector(".state button"));
  startAt("/files/", reader, { "GET /api/files": listing("", []) });
  el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /This folder is empty/));
  assert.ok(!el.querySelector(".state button"));
  assert.ok(!el.querySelector('button[aria-label="New folder"]'));
  assert.ok(!el.querySelector('.toolbar button[aria-haspopup="menu"]'));
});

test("filter and clear", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const search = el.querySelector<HTMLInputElement>('input[type="search"]')!;
  search.value = "CAFE";
  search.dispatchEvent(new Event("input", { bubbles: true }));
  await waitFor(() => assert.equal(rows(el).length, 1));
  search.value = "zzz";
  search.dispatchEvent(new Event("input", { bubbles: true }));
  await waitFor(() => assert.match(el.textContent!, /No results for “zzz”/));
  assert.match(el.textContent!, /The search covers this folder only\./);
  button(el, "Clear search").click();
  await waitFor(() => assert.equal(rows(el).length, 6));
  assert.equal(search.value, "");
});

test("an outside link is not clickable", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", [{ ...file("elsewhere"), kind: "link", size: 0, outside: true }]),
  });
  const el = render(<App />);
  await shown(el);
  const row = rows(el)[0];
  assert.ok(!row.querySelector("a.name"));
  assert.match(row.textContent!, /Link outside your folder/);
});

test("the selection toolbar counts and downloads several", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  click(el.querySelector('button[aria-label="Select notes.txt"]')!);
  await flush();
  assert.match(el.querySelector(".selection-bar")!.textContent!, /1 selected/);
  click(el.querySelector('button[aria-label="Select trail-ridge.jpg"]')!);
  await waitFor(() => assert.match(el.querySelector(".selection-bar")!.textContent!, /2 selected/));
  assert.equal(button(el, "Download").disabled, false);
  assert.equal(
    el.querySelector('button[aria-label="Select notes.txt"]')!.getAttribute("aria-pressed"),
    "true",
  );
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await waitFor(() => assert.ok(!el.querySelector(".selection-bar"), "selection bar still shown"));
});

test("shift-click selects a range", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  click(el.querySelector('button[aria-label="Select raw"]')!);
  await flush();
  click(el.querySelector('button[aria-label="Select notes.txt"]')!, { shiftKey: true });
  await waitFor(() => assert.match(el.querySelector(".selection-bar")!.textContent!, /3 selected/));
});

test("a folder the user cannot open says so", async () => {
  startAt("/files/private", admin, {
    "GET /api/files/private": () =>
      json({ error: "forbidden", message: "You do not have permission to do this here." }, 403),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /You can't open this folder/));
  assert.ok(el.querySelector('a[href="/files/"]'));
});

test("a folder that is gone says so", async () => {
  startAt("/files/gone", admin, {
    "GET /api/files/gone": () => json({ error: "not_found", message: "No such file or folder." }, 404),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /This folder is gone/));
});

function pick(input: HTMLInputElement, files: File[]) {
  Object.defineProperty(input, "files", { value: files, configurable: true });
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function okTransport(urls: string[]): Transport {
  return async (url) => {
    urls.push(url);
    return { status: 201, body: "{}" };
  };
}

test("Upload › Files sends into this folder and reloads it", async () => {
  let loads = 0;
  startAt("/files/photos", admin, {
    "GET /api/files/photos": () => {
      loads++;
      return json({ path: "photos", entries: sample });
    },
  });
  const urls: string[] = [];
  resetUploads(okTransport(urls));
  const el = render(<App />);
  await shown(el);
  click(button(el, "Upload"));
  await waitFor(() => assert.ok(button(el, "Files")));
  assert.ok(button(el, "Folder"));
  const input = el.querySelector<HTMLInputElement>("input[data-pick=files]")!;
  assert.ok(input.multiple);
  pick(input, [new File(["a"], "cover.png")]);
  await waitFor(() => assert.match(el.textContent!, /Uploaded 1 file to photos\./));
  assert.deepEqual(urls, ["/api/upload/photos/cover.png"]);
  await waitFor(() => assert.equal(loads, 2));
});

test("Upload › Folder keeps the picked folder's paths", async () => {
  const log = startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "POST /api/folders/2027": () => json({}, 201),
  });
  const urls: string[] = [];
  resetUploads(okTransport(urls));
  const el = render(<App />);
  await shown(el);
  const input = el.querySelector<HTMLInputElement>("input[data-pick=folder]")!;
  assert.ok(input.hasAttribute("webkitdirectory"));
  const f = new File(["a"], "cover.png");
  Object.defineProperty(f, "webkitRelativePath", { value: "2027/cover.png" });
  pick(input, [f]);
  await waitFor(() => assert.deepEqual(urls, ["/api/upload/2027/cover.png"]));
  assert.ok(log.some((r) => r.method === "POST" && r.url.endsWith("/api/folders/2027")));
  await waitFor(() => assert.match(el.textContent!, /2027\/cover\.png/));
});

test("dropping a folder on the list uploads it", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "POST /api/folders/trip": () => json({}, 201),
  });
  const urls: string[] = [];
  resetUploads(okTransport(urls));
  const el = render(<App />);
  await shown(el);
  const fileEntry = {
    isFile: true,
    isDirectory: false,
    name: "map.png",
    file: (ok: (f: File) => void) => ok(new File(["m"], "map.png")),
  };
  let reads = 0;
  const dirEntry = {
    isFile: false,
    isDirectory: true,
    name: "trip",
    createReader: () => ({ readEntries: (ok: (e: unknown[]) => void) => ok(reads++ === 0 ? [fileEntry] : []) }),
  };
  const drop = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(drop, "dataTransfer", {
    value: { items: [{ kind: "file", webkitGetAsEntry: () => dirEntry }], files: [], types: ["Files"] },
  });
  el.querySelector(".files")!.dispatchEvent(drop);
  assert.ok(drop.defaultPrevented);
  await waitFor(() => assert.deepEqual(urls, ["/api/upload/trip/map.png"]));
});

test("the empty folder's Upload opens the file picker", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", []) });
  resetUploads(okTransport([]));
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /This folder is empty/));
  const input = el.querySelector<HTMLInputElement>("input[data-pick=files]")!;
  let opened = 0;
  input.click = () => {
    opened++;
  };
  click(el.querySelector(".state button")!);
  assert.equal(opened, 1);
});

test("the upload panel stays across folders", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "GET /api/files/2026": listing("2026", []),
  });
  resetUploads(() => new Promise(() => {}));
  const el = render(<App />);
  await shown(el);
  pick(el.querySelector<HTMLInputElement>("input[data-pick=files]")!, [new File(["a"], "a.txt")]);
  await waitFor(() => assert.match(el.textContent!, /Uploading 1 of 1/));
  click(el.querySelector('.list-body a[href="/files/2026"]')!);
  await waitFor(() => assert.match(el.textContent!, /This folder is empty/));
  assert.match(el.textContent!, /Uploading 1 of 1/);
});

function menuItem(el: HTMLElement, label: string): HTMLButtonElement {
  const b = [...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((x) =>
    x.textContent!.startsWith(label),
  );
  assert.ok(b, `no menu item "${label}"`);
  return b!;
}

function more(el: HTMLElement, name: string): HTMLButtonElement {
  const b = el.querySelector<HTMLButtonElement>(`button[aria-label="More actions for ${name}"]`);
  assert.ok(b, `no More button for ${name}`);
  return b!;
}

/** Opens the toolbar's New menu and picks `item`. */
async function openNew(el: HTMLElement, item: string) {
  const b = el.querySelector<HTMLButtonElement>('.toolbar button[aria-haspopup="menu"]');
  assert.ok(b, "no New button");
  assert.equal(b!.textContent!.trim(), "New");
  click(b!);
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  const it = [...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((x) => x.textContent!.trim() === item);
  assert.ok(it, `no menu item ${item}`);
  click(it!);
}

function submit(el: HTMLElement) {
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

test("rename from the More menu", async () => {
  let loads = 0;
  const log = startAt("/files/photos", admin, {
    "GET /api/files/photos": () => {
      loads++;
      return json({ path: "photos", entries: sample });
    },
    "POST /api/ops": () => json({ done: 1, failed: [] }),
  });
  const el = render(<App />);
  await shown(el);
  click(more(el, "notes.txt"));
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  click(menuItem(el, "Rename"));
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"] input')));
  const input = el.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  input.value = "todo.txt";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
  submit(el);
  await waitFor(() => assert.match(el.textContent!, /Renamed to todo\.txt\./));
  assert.ok(!el.querySelector('[role="dialog"]'));
  const ops = log.find((r) => r.method === "POST")!;
  assert.deepEqual(await ops.json(), { ops: [{ op: "rename", from: "photos/notes.txt", to: "photos/todo.txt" }] });
  await waitFor(() => assert.equal(loads, 2));
  assert.ok(el.querySelector('.toast-region[role="status"] .toast'));
});

test("right click opens the menu; a reader gets no write items", async () => {
  startAt("/files/", reader, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  rows(el)[3].dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }));
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  const labels = [...el.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent);
  assert.deepEqual(labels, ["OpenEnter", "Download"]);
});

test("the menu offers Share… to people who may share", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  rows(el)[3].dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }));
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  click(menuItem(el, "Share…"));
  await waitFor(() => assert.match(el.querySelector('[role="dialog"]')!.textContent!, /Share link/));
});

test("Shift+F10 on a row opens its menu; F2 renames the selected item", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const row = rows(el)[3];
  row.querySelector<HTMLElement>("a.name")!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true }),
  );
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await waitFor(() => assert.ok(!el.querySelector('[role="menu"]')));
  click(row.querySelector(".check-button")!);
  await flush();
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true }));
  await waitFor(() => assert.equal(el.querySelector<HTMLInputElement>('[role="dialog"] input')?.value, "notes.txt"));
});

test("move the selection reports a partial failure", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "GET /api/files/2026": listing("2026", []),
    "POST /api/ops": () =>
      json({
        done: 1,
        failed: [{ from: "site-backup.tar.gz", error: "denied", message: "x" }],
      }),
  });
  const el = render(<App />);
  await shown(el);
  click(rows(el)[3].querySelector(".check-button")!);
  click(rows(el)[5].querySelector(".check-button")!);
  await flush();
  click(button(el, "Move"));
  await waitFor(() => assert.match(el.textContent!, /Move 2 items to…/));
  await waitFor(() => assert.ok([...el.querySelectorAll('[role="dialog"] button')].some((b) => b.textContent === "2026")));
  click([...el.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((b) => b.textContent === "2026")!);
  await waitFor(() => assert.ok(!button(el, "Move here").disabled));
  click(button(el, "Move here"));
  await waitFor(() =>
    assert.match(
      el.textContent!,
      /Moved 1 of 2 items\. site-backup\.tar\.gz: The server's folder permissions do not allow this\./,
    ),
  );
  assert.ok(!el.querySelector(".selection-bar"));
});

test("delete_permanent_when_trash_off", async () => {
  const log = startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "DELETE /api/files": () => json({ deleted: 1, failed: [] }),
  });
  const el = render(<App />);
  await shown(el);
  click(more(el, "notes.txt"));
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  click(menuItem(el, "Delete"));
  await waitFor(() => assert.match(el.textContent!, /Delete 1 item\?/));
  assert.ok(!log.some((r) => r.method === "DELETE"));
  assert.match(el.querySelector('[role="dialog"]')!.textContent!, /The trash is off on this server\./);
  click(el.querySelector<HTMLButtonElement>('[role="dialog"] .button.danger')!);
  await waitFor(() => assert.match(el.textContent!, /Deleted 1 item\./));
  assert.deepEqual(await log.find((r) => r.method === "DELETE")!.json(), { paths: ["notes.txt"], permanent: true });
});

test("new folder from the toolbar", async () => {
  startAt("/files/photos", admin, {
    "GET /api/files/photos": listing("photos", sample),
    "POST /api/folders/photos/2027": () => json({ entry: folder("2027") }, 201),
  });
  const el = render(<App />);
  await shown(el);
  await openNew(el, "New folder");
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"] input')));
  const input = el.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  input.value = "2027";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
  submit(el);
  await waitFor(() => assert.match(el.textContent!, /Created folder 2027\./));
});

const yesterday = Math.floor((Date.now() - 86_400_000) / 1000);

test("phone: one column with a meta line", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, {
    "GET /api/files": listing("", [folder("2026", { modified: yesterday }), file("trail-ridge.jpg", { modified: yesterday })]),
  });
  const el = render(<App />);
  await shown(el);
  const r = rows(el);
  assert.equal(r[1].querySelector(".meta")!.textContent, "4.2 MB · Yesterday");
  assert.equal(r[0].querySelector(".meta")!.textContent, "Yesterday");
  assert.ok(!el.querySelector(".list-head"));
});

test("phone: a long press starts selection mode; Select all; close", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const row = rows(el)[3];
  row.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch", clientX: 5, clientY: 5 }));
  await new Promise((r) => setTimeout(r, 560));
  row.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "touch" }));
  const swallowed = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
  row.querySelector("a.name")!.dispatchEvent(swallowed);
  assert.ok(swallowed.defaultPrevented);
  assert.equal(location.pathname, "/files/");
  await waitFor(() => assert.match(el.textContent!, /1 selected/));
  const bar = el.querySelector(".bottom-bar")!;
  assert.ok(bar);
  assert.deepEqual(
    [...bar.querySelectorAll("button")].map((b) => b.textContent!.trim()),
    ["Download", "Move", "Delete"],
  );
  button(el, "Select all").click();
  await waitFor(() => assert.match(el.textContent!, /6 selected/));
  el.querySelector<HTMLButtonElement>('button[aria-label="Clear selection"]')!.click();
  await waitFor(() => assert.ok(!el.querySelector(".bottom-bar")));
});

test("phone: a short press does not select", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const row = rows(el)[3];
  row.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch" }));
  await new Promise((r) => setTimeout(r, 100));
  row.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "touch" }));
  await new Promise((r) => setTimeout(r, 500));
  await flush();
  assert.ok(!el.querySelector(".bottom-bar"));
});

test("phone: the toolbar folds New folder and the view into More", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  assert.ok(!el.querySelector('.toolbar [role="group"][aria-label="View"]'));
  assert.ok(button(el, "Upload"));
  el.querySelector<HTMLButtonElement>('.toolbar button[aria-label="More"]')!.click();
  await waitFor(() => assert.ok(el.querySelector('.toolbar .menu')));
  const items = [...el.querySelectorAll(".toolbar .menu button")].map((b) => b.textContent!.trim());
  assert.deepEqual(items, ["New folder", "New text file", "List view", "Gallery view"]);
});

test("final: a slow listing that lands after moving on is dropped", async () => {
  let releaseA: (r: Response) => void = () => {};
  startAt("/files/a", admin, {
    "GET /api/files/a": () => new Promise<Response>((r) => (releaseA = r)),
    "GET /api/files/b": listing("b", [file("only-in-b.txt")]),
  });
  const el = render(<App />);
  await flush();
  const { navigate } = await import("../lib/router");
  navigate("/files/b");
  await waitFor(() => assert.match(el.textContent!, /only-in-b\.txt/));
  releaseA(json({ path: "a", entries: [file("report.txt")] }));
  await flush(10);
  assert.match(el.textContent!, /only-in-b\.txt/);
  assert.ok(!el.textContent!.includes("report.txt"));
});

test("final: more than 1000 items are refused in the browser", async (t) => {
  t.after(phoneWidth());
  const many = Array.from({ length: 1001 }, (_, i) => file(`f-${i}.txt`));
  startAt("/files/", admin, { "GET /api/files": listing("", many) });
  let submitted = 0;
  const original = HTMLFormElement.prototype.submit;
  HTMLFormElement.prototype.submit = () => void submitted++;
  t.after(() => (HTMLFormElement.prototype.submit = original));
  const el = render(<App />);
  await shown(el);
  click(rows(el)[0].querySelector(".check-button")!);
  await waitFor(() => assert.ok(button(el, "Select all")));
  button(el, "Select all").click();
  await waitFor(() => assert.match(el.textContent!, /1,001 selected/));
  [...el.querySelectorAll<HTMLButtonElement>(".bottom-bar button")][0].click();
  await waitFor(() => assert.match(el.textContent!, /Choose between 1 and 1000 items in one folder\./));
  assert.equal(submitted, 0);
});

test("minor: Escape while typing in Search keeps the selection", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  click(rows(el)[3].querySelector(".check-button")!);
  await waitFor(() => assert.ok(el.querySelector(".selection-bar")));
  const search = el.querySelector<HTMLInputElement>("#explorer-search")!;
  search.focus();
  search.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await flush();
  assert.ok(el.querySelector(".selection-bar"));
});

test("minor: Ctrl or Cmd+click on a row toggles it", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const ev = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ctrlKey: true });
  rows(el)[3].querySelector("a.name")!.dispatchEvent(ev);
  assert.ok(ev.defaultPrevented);
  await waitFor(() => assert.match(el.textContent!, /1 selected/));
  assert.equal(location.pathname, "/files/");
  click(rows(el)[5].querySelector(".cell-when")!, { metaKey: true });
  await waitFor(() => assert.match(el.textContent!, /2 selected/));
  click(rows(el)[3].querySelector(".cell-size")!, { ctrlKey: true });
  await waitFor(() => assert.match(el.textContent!, /1 selected/));
});

test("minor: the Upload menu closes on Escape and on a press outside", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  click(button(el, "Upload"));
  await waitFor(() => assert.ok(el.querySelector(".toolbar .menu")));
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await waitFor(() => assert.ok(!el.querySelector(".toolbar .menu")));
  click(button(el, "Upload"));
  await waitFor(() => assert.ok(el.querySelector(".toolbar .menu")));
  el.querySelector(".list-body")!.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  await waitFor(() => assert.ok(!el.querySelector(".toolbar .menu")));
});

test("minor: closing the tab during uploads asks; Sign out asks too", async () => {
  const log = startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "POST /api/auth/logout": () => json({ next: null }),
  });
  resetUploads(() => new Promise(() => {}));
  const asked: string[] = [];
  globalThis.confirm = (m?: string) => {
    asked.push(String(m));
    return false;
  };
  const el = render(<App />);
  await shown(el);
  pick(el.querySelector<HTMLInputElement>("input[data-pick=files]")!, [new File(["a"], "a.txt")]);
  await waitFor(() => assert.match(el.textContent!, /Uploading 1 of 1/));
  const ev = new Event("beforeunload", { cancelable: true });
  dispatchEvent(ev);
  assert.ok(ev.defaultPrevented);
  el.querySelector<HTMLButtonElement>('button[aria-label="Sign out"]')!.click();
  await flush(5);
  assert.deepEqual(asked, ["Uploads are still running. Sign out and stop them?"]);
  assert.ok(!log.some((r) => r.url.endsWith("/api/auth/logout")));
  resetUploads(() => new Promise(() => {}));
  const after = new Event("beforeunload", { cancelable: true });
  dispatchEvent(after);
  assert.ok(!after.defaultPrevented);
});

test("minor: Sign out that cannot reach the server stays signed in and says so", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "POST /api/auth/logout": () => {
      throw new TypeError("network down");
    },
  });
  resetUploads(okTransport([]));
  const el = render(<App />);
  await shown(el);
  el.querySelector<HTMLButtonElement>('button[aria-label="Sign out"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /Can't reach the server\. Check that Cairn is running, then try again\./));
  assert.equal(location.pathname, "/files/");
  assert.ok(el.querySelector(".list-body"));
});

test("minor: the page behind a dialog is inert while it is open", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  await openNew(el, "New folder");
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"]')));
  // The dialog marks the page in an effect, which runs after the next frame.
  await waitFor(() => assert.ok(el.querySelector(".sidebar")!.hasAttribute("inert"), "sidebar"));
  assert.ok(el.querySelector(".toolbar")!.hasAttribute("inert"), "toolbar");
  assert.ok(!el.querySelector('[role="dialog"]')!.closest("[inert]"));
  button(el, "Cancel").click();
  await waitFor(() => assert.ok(!el.querySelector('[role="dialog"]')));
  assert.ok(!el.querySelector("[inert]"), "left inert: " + el.querySelector("[inert]")?.className);
});

test("new_menu_offers_folder_and_text_file", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  const b = el.querySelector<HTMLButtonElement>('.toolbar button[aria-haspopup="menu"]')!;
  assert.equal(b.textContent!.trim(), "New");
  click(b);
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  const items = [...el.querySelectorAll('[role="menuitem"]')].map((x) => x.textContent!.trim());
  assert.deepEqual(items, ["New folder", "New text file"]);
  assert.equal(document.activeElement, el.querySelector('[role="menuitem"]'));
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
  assert.equal(document.activeElement!.textContent!.trim(), "New text file");
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  await waitFor(() => assert.ok(!el.querySelector('[role="menu"]')));
});

test("new_text_file_creates_and_opens_editor", async () => {
  const log = startAt("/files/notes", admin, {
    "GET /api/files/notes": listing("notes", sample),
    "PUT /api/text/notes/todo.txt": () => json({ etag: '"0-1"' }, 200, { etag: '"0-1"' }),
    "GET *": () => new Response("", { status: 200 }),
    "HEAD *": () => new Response("", { status: 200 }),
  });
  const el = render(<App />);
  await shown(el);
  await openNew(el, "New text file");
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"] input')));
  const input = el.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  assert.equal(input.value, "untitled.txt");
  input.value = "todo.txt";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
  submit(el);
  await waitFor(() => assert.equal(location.pathname, "/view/notes/todo.txt"));
  const put = log.find((r) => r.method === "PUT")!;
  assert.equal(put.headers.get("if-none-match"), "*");
  assert.equal(await put.text(), "");
});

test("new_text_file_taken_name", async () => {
  startAt("/files/", admin, {
    "GET /api/files": listing("", sample),
    "PUT /api/text/notes.txt": () =>
      json({ error: "exists", message: "Something with that name is already there. Choose another name, or replace it." }, 409),
  });
  const el = render(<App />);
  await shown(el);
  await openNew(el, "New text file");
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"] input')));
  const input = el.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  input.value = "notes.txt";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
  submit(el);
  await waitFor(() => assert.match(el.querySelector('[role="dialog"]')!.textContent!, /already there/));
  assert.equal(location.pathname, "/files/");
});

test("new_menu_hidden_without_write", async () => {
  startAt("/files/", reader, { "GET /api/files": listing("", sample) });
  const el = render(<App />);
  await shown(el);
  assert.ok(!el.querySelector('.toolbar button[aria-haspopup="menu"]'));
});

const withTrash = { "GET /api/me": () => json({ user: admin, trash: { on: true } }) };

test("delete_sends_to_trash_and_undo_restores", async () => {
  const log = startAt("/files/", admin, {
    ...withTrash,
    "GET /api/files": listing("", sample),
    "DELETE /api/files": () => json({ deleted: 2, failed: [], entries: ["e1", "e2"] }),
    "POST /api/trash/restore": () => json({ restored: [{ id: "e1" }, { id: "e2" }], failed: [] }),
  });
  const el = render(<App />);
  await shown(el);
  click(rows(el)[2].querySelector(".check-button")!);
  click(rows(el)[3].querySelector(".check-button")!);
  await waitFor(() => assert.ok(el.querySelector(".selection-bar")));
  click(button(el, "Delete"));
  await waitFor(() => assert.match(el.textContent!, /Move 2 items to the trash\?/));
  click(button(el, "Move to trash"));
  await waitFor(() => assert.match(el.textContent!, /Moved 2 items to the trash\./));
  assert.deepEqual(await log.find((r) => r.method === "DELETE")!.json(), { paths: ["trail-ridge.jpg", "notes.txt"] });
  click(button(el, "Undo"));
  await waitFor(() => assert.ok(log.some((r) => r.method === "POST" && r.url.endsWith("/api/trash/restore"))));
  const post = log.find((r) => r.url.endsWith("/api/trash/restore"))!;
  assert.deepEqual(await post.json(), { ids: ["e1", "e2"] });
  await waitFor(() => assert.match(el.textContent!, /Restored 2 items\./));
});

test("other_disk_offers_permanent", async () => {
  const bodies: unknown[] = [];
  startAt("/files/", admin, {
    ...withTrash,
    "GET /api/files": listing("", sample),
    "DELETE /api/files": async (req) => {
      const body = await req.json();
      bodies.push(body);
      return body.permanent
        ? json({ deleted: 1, failed: [], entries: [] })
        : json({
            deleted: 0,
            failed: [{ path: "notes.txt", error: "other_disk", message: "notes.txt is on another disk, so it cannot go to the trash." }],
            entries: [],
          });
    },
  });
  const el = render(<App />);
  await shown(el);
  click(more(el, "notes.txt"));
  await waitFor(() => assert.ok(el.querySelector('[role="menu"]')));
  click(menuItem(el, "Delete"));
  await waitFor(() => assert.ok(el.querySelector('[role="dialog"]')));
  click(button(el, "Move to trash"));
  await waitFor(() => assert.match(el.querySelector('[role="dialog"]')?.textContent ?? "", /notes\.txt is on another disk, so it cannot go to the trash\./));
  click(button(el, "Delete permanently"));
  await waitFor(() => assert.equal(bodies.length, 2));
  assert.deepEqual(bodies[1], { paths: ["notes.txt"], permanent: true });
  await waitFor(() => assert.match(el.textContent!, /Deleted 1 item\./));
});
