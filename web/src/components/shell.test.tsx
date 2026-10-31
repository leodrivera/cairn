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
  el.querySelector<HTMLAnchorElement>('.sidebar a[href="/links"]')!.dispatchEvent(
    new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }),
  );
  await waitFor(() => assert.equal(location.pathname, "/links"));
  await flush();
  assert.ok(!el.querySelector(".sidebar"));
});

test("on a wide screen the sidebar stays and there is no menu button", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".sidebar")));
  assert.ok(!el.querySelector('button[aria-label="Open menu"]'));
});

test("the account row has a Settings button beside Sign out, and the name opens the profile", async () => {
  startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".account")));
  const gear = el.querySelector<HTMLAnchorElement>('.account a[aria-label="Settings"]');
  assert.ok(gear, "no Settings button in the account row");
  assert.equal(gear!.getAttribute("href"), "/settings/profile");
  assert.ok(gear!.querySelector("svg"), "the Settings button shows an icon");
  assert.equal(el.querySelectorAll('[aria-label="Settings"]').length, 1);
  assert.ok(el.querySelector('.account button[aria-label="Sign out"]'));
  const name = el.querySelector<HTMLAnchorElement>(".account-link")!;
  assert.equal(name.getAttribute("href"), "/settings/profile");
  assert.equal(name.getAttribute("aria-label"), null);
  gear!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  await waitFor(() => assert.ok(el.querySelector(".settings-nav")));
  assert.equal(location.pathname, "/settings/profile");
});

test("the Settings button in Portuguese", async () => {
  const before = Object.getOwnPropertyDescriptor(navigator, "languages");
  Object.defineProperty(navigator, "languages", { value: ["pt-BR"], configurable: true });
  try {
    startAt("/files/", admin, { "GET /api/files": listing("", [file("a.txt")]) });
    const el = render(<App />);
    await waitFor(() => assert.ok(el.querySelector('.account a[aria-label="Configurações"]')));
  } finally {
    if (before) Object.defineProperty(navigator, "languages", before);
    else delete (navigator as { languages?: unknown }).languages;
  }
});
