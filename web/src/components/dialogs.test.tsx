import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { DeleteDialog, FolderPicker, NewFolderDialog, RenameDialog } from "./dialogs";
import { ContextMenu, entryMenu } from "./context-menu";
import { flush, render, waitFor } from "../test/dom";
import { file, folder, listing, startAt, admin } from "../test/fixtures";

function button(el: HTMLElement, text: string): HTMLButtonElement {
  const b = [...el.querySelectorAll("button")].find((x) => x.textContent?.trim() === text);
  assert.ok(b, `no button "${text}"`);
  return b as HTMLButtonElement;
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function key(target: EventTarget, k: string, init: KeyboardEventInit = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));
}

test("rename selects the stem", async () => {
  const el = render(<RenameDialog entry={file("site-backup.tar.gz")} onRename={async () => {}} onClose={() => {}} />);
  await flush();
  const input = el.querySelector<HTMLInputElement>("input")!;
  assert.equal(input.value, "site-backup.tar.gz");
  assert.ok(document.activeElement === input);
  assert.equal(input.selectionStart, 0);
  assert.equal(input.selectionEnd, "site-backup.tar".length);
});

test("rename refuses a slash or an empty name", async () => {
  const names: string[] = [];
  const el = render(<RenameDialog entry={file("a.txt")} onRename={async (n) => void names.push(n)} onClose={() => {}} />);
  const input = el.querySelector<HTMLInputElement>("input")!;
  type(input, "a/b.txt");
  await flush();
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await waitFor(() => assert.match(el.textContent!, /A name cannot be empty or contain \//));
  type(input, "  ");
  await flush();
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await flush();
  assert.deepEqual(names, []);
  type(input, "b.txt");
  await flush();
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await waitFor(() => assert.deepEqual(names, ["b.txt"]));
});

test("new folder shows the server's error", async () => {
  const { ApiFailure } = await import("../lib/api");
  const el = render(
    <NewFolderDialog
      onCreate={async () => {
        throw new ApiFailure(409, "exists", "");
      }}
      onClose={() => {}}
    />,
  );
  type(el.querySelector<HTMLInputElement>("input")!, "2026");
  await flush();
  el.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await waitFor(() => assert.match(el.textContent!, /Something with that name is already there/));
});

test("picker disables the current folder and folders inside the chosen ones", async () => {
  startAt("/files/photos", admin, {
    "GET /api/files": listing("", [folder("photos"), folder("docs"), file("x.txt")]),
    "GET /api/files/photos": listing("photos", [folder("2026"), folder("raw")]),
    "GET /api/files/photos/2026": listing("photos/2026", [folder("summer")]),
  });
  const picked: string[] = [];
  const el = render(
    <FolderPicker
      op="move"
      dir="photos"
      items={[folder("2026"), file("a.txt")]}
      onPick={async (d) => void picked.push(d)}
      onClose={() => {}}
    />,
  );
  await waitFor(() => assert.ok(button(el, "docs")));
  assert.match(el.textContent!, /Move 2 items to…/);
  assert.ok(!el.textContent!.includes("x.txt"));
  assert.ok(!button(el, "Move here").disabled);
  button(el, "photos").click();
  await waitFor(() => assert.ok(button(el, "raw")));
  assert.ok(button(el, "Move here").disabled);
  button(el, "2026").click();
  await waitFor(() => assert.ok(button(el, "summer")));
  assert.ok(button(el, "Move here").disabled);
  button(el, "summer").click();
  await waitFor(() => assert.match(el.textContent!, /photos\/2026\/summer/));
  assert.ok(button(el, "Move here").disabled);
  el.querySelector<HTMLButtonElement>('button[aria-label="Up one folder"]')!.click();
  await waitFor(() => assert.ok(button(el, "summer")));
  el.querySelector<HTMLButtonElement>('button[aria-label="Up one folder"]')!.click();
  await waitFor(() => assert.ok(button(el, "raw")));
  button(el, "raw").click();
  await waitFor(() => assert.ok(!button(el, "Move here").disabled));
  button(el, "Move here").click();
  await waitFor(() => assert.deepEqual(picked, ["photos/raw"]));
});

test("copy picker says copy", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", [folder("docs")]) });
  const el = render(<FolderPicker op="copy" dir="" items={[file("a.txt")]} onPick={async () => {}} onClose={() => {}} />);
  await waitFor(() => assert.ok(button(el, "docs")));
  assert.match(el.textContent!, /Copy 1 item to…/);
  assert.ok(button(el, "Copy here").disabled);
});

test("delete focuses Cancel and lists three names", async () => {
  const items = ["cover.png", "trail-ridge.jpg", "notes.txt", "a.txt", "b.txt"].map((n) => file(n));
  let confirmed = 0;
  const el = render(
    <DeleteDialog dir="photos/2026" items={items} onConfirm={async () => void confirmed++} onClose={() => {}} />,
  );
  await flush();
  assert.ok(document.activeElement === button(el, "Cancel"));
  assert.match(el.textContent!, /Delete 5 items\?/);
  assert.match(el.textContent!, /cover\.png, trail-ridge\.jpg, notes\.txt,? and 2 more will be removed from photos\/2026\. This can't be undone\./);
  assert.ok(!el.textContent!.includes("a.txt"));
  button(el, "Delete").click();
  await waitFor(() => assert.equal(confirmed, 1));
});

test("delete of two names them both", async () => {
  const el = render(
    <DeleteDialog dir="" items={[file("cover.png"), file("trail-ridge.jpg")]} onConfirm={async () => {}} onClose={() => {}} />,
  );
  await flush();
  assert.match(el.textContent!, /cover\.png and trail-ridge\.jpg will be removed from My files\./);
});

test("menu keyboard: arrows move, Enter runs, Escape closes and gives focus back", async () => {
  const opener = document.createElement("button");
  document.body.append(opener);
  opener.focus();
  const ran: string[] = [];
  let closed = 0;
  const items = [
    { label: "One", run: () => ran.push("one") },
    "separator" as const,
    { label: "Two", run: () => ran.push("two") },
  ];
  let el = render(<ContextMenu at={{ x: 10, y: 10 }} items={items} onClose={() => closed++} />);
  await flush();
  const menu = el.querySelector('[role="menu"]')!;
  assert.ok(document.activeElement === menu.querySelectorAll('[role="menuitem"]')[0]);
  key(document.activeElement!, "ArrowDown");
  await flush();
  assert.ok(document.activeElement === menu.querySelectorAll('[role="menuitem"]')[1]);
  key(document.activeElement!, "Enter");
  await flush();
  assert.deepEqual(ran, ["two"]);
  assert.equal(closed, 1);
  assert.ok(document.activeElement === opener);

  opener.focus();
  el = render(<ContextMenu at={{ x: 10, y: 10 }} items={items} onClose={() => closed++} />);
  await flush();
  key(document.activeElement!, "ArrowUp");
  await flush();
  assert.ok(document.activeElement === el.querySelectorAll('[role="menuitem"]')[1]);
  key(document.activeElement!, "Escape");
  await flush();
  assert.equal(closed, 2);
  assert.deepEqual(ran, ["two"]);
  assert.ok(document.activeElement === opener);
  opener.remove();
});

test("menu hides write items for a reader", () => {
  const noop = () => {};
  const h = { open: noop, download: noop, rename: noop, move: noop, copy: noop, remove: noop };
  const labels = (canWrite: boolean) =>
    entryMenu(canWrite, h).map((i) => (i === "separator" ? "-" : `${i.label}${i.key ? ` ${i.key}` : ""}`));
  assert.deepEqual(labels(true), ["Open Enter", "Download", "-", "Rename F2", "Move to…", "Copy to…", "-", "Delete Del"]);
  assert.deepEqual(labels(false), ["Open Enter", "Download"]);
  assert.equal((entryMenu(true, h).at(-1) as { danger?: boolean }).danger, true);
});
