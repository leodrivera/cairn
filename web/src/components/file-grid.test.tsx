import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { render, waitFor } from "../test/dom";
import { admin, file, folder, listing, startAt } from "../test/fixtures";
import { json } from "../test/dom";

const gallery = { ...admin, view: "gallery" as const };
const photos = [folder("2026"), file("cover.png", { mime: "image/png" }), file("notes.txt", { mime: "text/plain" })];

async function tiles(el: HTMLElement) {
  await waitFor(() => assert.ok(el.querySelector(".grid .tile")));
  return [...el.querySelectorAll<HTMLElement>(".grid .tile")];
}

test("no img when thumbnails are off", async () => {
  startAt("/files/photos", gallery, { "GET /api/files/photos": listing("photos", photos) }, { thumbnails: false });
  const el = render(<App />);
  assert.equal((await tiles(el)).length, 3);
  assert.equal(el.querySelectorAll(".grid img").length, 0);
});

test("lazy img when thumbnails are on, only for images", async () => {
  const many = [file("Report #1 (50%) ✓ café.png", { mime: "image/png" }), ...photos];
  startAt("/files/photos", gallery, { "GET /api/files/photos": listing("photos", many) }, { thumbnails: true });
  const el = render(<App />);
  await tiles(el);
  const imgs = [...el.querySelectorAll<HTMLImageElement>(".grid img")];
  assert.equal(imgs.length, 2);
  assert.equal(imgs[0].getAttribute("src"), "/api/raw/photos/Report%20%231%20(50%25)%20%E2%9C%93%20caf%C3%A9.png");
  assert.equal(imgs[0].getAttribute("loading"), "lazy");
  assert.equal(imgs[0].getAttribute("decoding"), "async");
  assert.equal(imgs[0].getAttribute("alt"), "");
});

test("only tiles in the window render", async () => {
  const many = Array.from({ length: 10_000 }, (_, i) => file(`img-${i}.jpg`, { mime: "image/jpeg" }));
  startAt("/files/", gallery, { "GET /api/files": listing("", many) }, { thumbnails: true });
  const el = render(<App />);
  await tiles(el);
  const n = el.querySelectorAll(".grid img").length;
  assert.ok(n > 0 && n < 100, `${n} images in the DOM`);
});

test("switching the view saves it", async () => {
  const log = startAt("/files/", admin, {
    "GET /api/files": listing("", photos),
    "PATCH /api/me": () => json({ user: gallery }),
  });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector(".list-body")));
  el.querySelector<HTMLButtonElement>('button[aria-label="Gallery view"]')!.click();
  await tiles(el);
  const patches = log.filter((r) => r.method === "PATCH");
  assert.equal(patches.length, 1);
  assert.deepEqual(await patches[0].json(), { view: "gallery" });
  assert.equal(el.querySelector('button[aria-label="Gallery view"]')!.getAttribute("aria-pressed"), "true");
});
