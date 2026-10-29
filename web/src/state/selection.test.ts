import { test } from "node:test";
import assert from "node:assert/strict";
import { Selection } from "./selection";

test("toggle, only, range and all", () => {
  const order = ["a", "b", "c", "d", "e"];
  const s = new Selection();
  s.toggle("b");
  s.range("d", order);
  assert.deepEqual(s.list(), ["b", "c", "d"]);
  s.range("a", order);
  assert.deepEqual(s.list(), ["a", "b"]);
  s.toggle("a");
  assert.deepEqual(s.list(), ["b"]);
  s.only("e");
  assert.deepEqual(s.list(), ["e"]);
  s.all(order);
  assert.equal(s.size, 5);
  s.clear();
  assert.equal(s.size, 0);
  assert.equal(s.has("a"), false);
});
