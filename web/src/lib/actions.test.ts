import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { downloadUrl, zipForm } from "./actions";
import { file, folder } from "../test/fixtures";

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
