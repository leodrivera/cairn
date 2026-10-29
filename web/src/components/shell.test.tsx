import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { flush, phoneWidth, render, waitFor } from "../test/dom";
import { admin, file, listing, startAt } from "../test/fixtures";

test("on a phone the menu button opens the sidebar as a sheet; Escape closes it", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
  const el = render(<App />);
  const menu = await (async () => {
    await waitFor(() => assert.ok(el.querySelector('button[aria-label="Open menu"]')));
    return el.querySelector<HTMLButtonElement>('button[aria-label="Open menu"]')!;
  })();
  assert.equal(menu.getAttribute("aria-expanded"), "false");
  assert.ok(!el.querySelector(".sidebar"));
  menu.focus();
  menu.click();
  await waitFor(() => assert.ok(el.querySelector(".sidebar.sheet")));
  assert.equal(menu.getAttribute("aria-expanded"), "true");
  assert.match(el.querySelector(".sidebar")!.textContent!, /My files/);
  assert.match(el.querySelector(".sidebar")!.textContent!, /Shared links/);
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await waitFor(() => assert.ok(!el.querySelector(".sidebar")));
  assert.ok(document.activeElement === el.querySelector('button[aria-label="Open menu"]'));
});

test("the sheet closes on navigation", async (t) => {
  t.after(phoneWidth());
  startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector('button[aria-label="Open menu"]')));
  el.querySelector<HTMLButtonElement>('button[aria-label="Open menu"]')!.click();
  await waitFor(() => assert.ok(el.querySelector(".sidebar.sheet")));
  el.querySelector<HTMLAnchorElement>('.sidebar a[href="/settings/links"]')!.dispatchEvent(
    new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }),
  );
  await waitFor(() => assert.equal(location.pathname, "/settings/links"));
  await flush();
  assert.ok(!el.querySelector(".sidebar"));
});

test("on a wide screen the sidebar stays and there is no menu button", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".sidebar")));
  assert.ok(!el.querySelector('button[aria-label="Open menu"]'));
});
