import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { downloadUrl, newFolder, remove, rename, startDownload, transfer, zipForm } from "./actions";
import { ApiFailure } from "./api";
import { json, stubFetch } from "../test/dom";
import { file, folder } from "../test/fixtures";

const raw = file("caf\uFFFD.txt", { rawName: "\u0000Y2Fm6S50eHQ" });

test("rename body", async () => {
  const log = stubFetch({ "POST /api/ops": () => json({ done: 1, failed: [] }) });
  await rename("photos", raw, "café.txt");
  assert.equal(log[0].headers.get("x-cairn"), "1");
  assert.deepEqual(await log[0].json(), {
    ops: [{ op: "rename", from: "photos/\u0000Y2Fm6S50eHQ", to: "photos/café.txt" }],
  });
});

test("rename failure throws the item's error", async () => {
  stubFetch({
    "POST /api/ops": () =>
      json({ done: 0, failed: [{ from: "photos/a.txt", error: "exists", message: "Something with that name is already there." }] }),
  });
  await assert.rejects(rename("photos", file("a.txt"), "b.txt"), (e) => e instanceof ApiFailure && e.code === "exists");
});

test("move reports partial", async () => {
  const failed = [{ from: "photos/b.txt", error: "denied", message: "x" }];
  const log = stubFetch({ "POST /api/ops": () => json({ done: 1, failed }) });
  const r = await transfer("move", "photos", [file("a.txt"), file("b.txt")], "");
  assert.deepEqual(r, { done: 1, failed });
  assert.deepEqual(await log[0].json(), {
    ops: [
      { op: "move", from: "photos/a.txt", to: "a.txt" },
      { op: "move", from: "photos/b.txt", to: "b.txt" },
    ],
  });
});

test("delete body paths", async () => {
  const log = stubFetch({ "DELETE /api/files": () => json({ deleted: 2, failed: [] }) });
  const r = await remove("photos", [folder("2026"), raw]);
  assert.deepEqual(r, { deleted: 2, failed: [] });
  assert.deepEqual(await log[0].json(), { paths: ["photos/2026", "photos/\u0000Y2Fm6S50eHQ"] });
});

test("new folder", async () => {
  const entry = folder("Report #1 (50%) ✓");
  const log = stubFetch({ "POST /api/folders/photos/Report #1 (50%) ✓": () => json({ entry }, 201) });
  assert.deepEqual(await newFolder("photos", "Report #1 (50%) ✓"), entry);
  assert.equal(new URL(log[0].url).pathname, "/api/folders/photos/Report%20%231%20(50%25)%20%E2%9C%93");
});

test("download urls", () => {
  assert.equal(downloadUrl("photos", [file("a b.txt")]), "/api/raw/photos/a%20b.txt?download=1");
  assert.equal(downloadUrl("photos", [folder("album")]), "/api/zip/photos/album");
  assert.equal(downloadUrl("", [folder("album")]), "/api/zip/album");
  assert.equal(downloadUrl("photos", [file("a.txt"), file("b.txt")]), null);
});

test("zipForm posts the chosen names, raw names as they are", () => {
  let sent: HTMLFormElement | null = null;
  let inputs: [string, string][] = [];
  const original = HTMLFormElement.prototype.submit;
  HTMLFormElement.prototype.submit = function (this: HTMLFormElement) {
    sent = this;
    inputs = [...this.querySelectorAll("input")].map((i) => [i.name, i.value]);
  };
  try {
    zipForm("photos", [file("a.txt"), file("caf�.txt", { rawName: "\u0000Y2Fm6S50eHQ" })]);
  } finally {
    HTMLFormElement.prototype.submit = original;
  }
  assert.ok(sent);
  const form = sent as HTMLFormElement;
  assert.equal(form.method, "post");
  assert.equal(new URL(form.action).pathname, "/api/zip/photos");
  assert.deepEqual(inputs, [
    ["item", "a.txt"],
    ["item", "\u0000Y2Fm6S50eHQ"],
  ]);
  assert.equal(form.isConnected, false);
});

test("final: downloads go through a hidden frame, so an error page never replaces the app", () => {
  let sent: HTMLFormElement | null = null;
  const original = HTMLFormElement.prototype.submit;
  HTMLFormElement.prototype.submit = function (this: HTMLFormElement) {
    sent = this;
  };
  try {
    zipForm("photos", [file("a.txt"), file("b.txt")]);
  } finally {
    HTMLFormElement.prototype.submit = original;
  }
  const frame = document.querySelector<HTMLIFrameElement>("iframe[name=cairn-download]");
  assert.ok(frame);
  assert.ok(frame!.hidden);
  assert.equal((sent as unknown as HTMLFormElement).target, "cairn-download");
  startDownload("/api/raw/a.txt?download=1");
  assert.equal(new URL(frame!.src).pathname + new URL(frame!.src).search, "/api/raw/a.txt?download=1");
  assert.equal(document.querySelectorAll("iframe[name=cairn-download]").length, 1);
});
