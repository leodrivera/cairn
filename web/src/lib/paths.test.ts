import { test } from "node:test";
import assert from "node:assert/strict";
import { apiUrl, displaySegment, encodePath, join, parent, segmentOf, segments } from "./paths";
import { href, parseRoute } from "./router";
import type { Entry } from "./types";

const entry = (name: string, rawName?: string): Entry => ({
  name,
  kind: "file",
  size: 1,
  modified: 0,
  mime: "text/plain",
  ...(rawName ? { rawName } : {}),
});

const report = "Report #1 (50%) ✓ café.txt";

test("unicode round trip", () => {
  const path = join("photos/2026", segmentOf(entry(report)));
  assert.equal(apiUrl("raw", path), "/api/raw/photos/2026/Report%20%231%20(50%25)%20%E2%9C%93%20caf%C3%A9.txt");
  const back = parseRoute(href({ name: "view", path }), "");
  assert.deepEqual(back, { name: "view", path });
  const folder = parseRoute(href({ name: "files", path }), "");
  assert.deepEqual(folder, { name: "files", path });
});

test("question mark and hash in a name", () => {
  assert.equal(encodePath("a?b/c#d"), "a%3Fb/c%23d");
  assert.deepEqual(parseRoute(href({ name: "files", path: "a?b" }), ""), { name: "files", path: "a?b" });
});

test("raw name segment", () => {
  const e = entry("caf�.txt", "\u0000Y2Fm6S50eHQ");
  assert.equal(segmentOf(e), "\u0000Y2Fm6S50eHQ");
  const path = join("docs", segmentOf(e));
  assert.equal(encodePath(path), "docs/%00Y2Fm6S50eHQ");
  assert.equal(apiUrl("upload", path), "/api/upload/docs/%00Y2Fm6S50eHQ");
  assert.equal(displaySegment(segmentOf(e)), "caf�.txt");
  assert.deepEqual(parseRoute(href({ name: "view", path }), ""), { name: "view", path });
});

test("root urls", () => {
  assert.equal(apiUrl("files", ""), "/api/files");
  assert.equal(apiUrl("zip", ""), "/api/zip");
  assert.equal(apiUrl("raw", "a b", { download: "1" }), "/api/raw/a%20b?download=1");
});

test("parent, join and segments", () => {
  assert.equal(join("", "a"), "a");
  assert.equal(join("a/b", "c"), "a/b/c");
  assert.equal(parent("a/b/c"), "a/b");
  assert.equal(parent("a"), "");
  assert.equal(parent(""), "");
  assert.deepEqual(segments(""), []);
  assert.deepEqual(segments("a/b"), ["a", "b"]);
});

test("api_url_with_base", () => {
  assert.equal(apiUrl("files", "a b", undefined, "/api/g/AbCd"), "/api/g/AbCd/files/a%20b");
  assert.equal(apiUrl("raw", "", { download: "1" }, "/api/g/AbCd"), "/api/g/AbCd/raw?download=1");
  assert.equal(apiUrl("files", ""), "/api/files");
});
