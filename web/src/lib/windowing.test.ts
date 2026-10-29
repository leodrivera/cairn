import { test } from "node:test";
import assert from "node:assert/strict";
import { gridRange, visibleRange } from "./windowing";

test("range math", () => {
  assert.deepEqual(visibleRange(0, 520, 52, 50_000), { start: 0, end: 18 });
  assert.deepEqual(visibleRange(52_000, 520, 52, 50_000), { start: 992, end: 1018 });
  assert.deepEqual(visibleRange(52 * 49_995, 520, 52, 50_000), { start: 49_987, end: 50_000 });
  assert.deepEqual(visibleRange(0, 520, 52, 0), { start: 0, end: 0 });
  assert.deepEqual(visibleRange(0, 520, 52, 5), { start: 0, end: 5 });
  assert.deepEqual(visibleRange(-40, 520, 52, 50, 0), { start: 0, end: 10 });
});

test("grid range math", () => {
  assert.deepEqual(gridRange(0, 800, 216, 100, 4), { start: 0, end: 24 });
  assert.deepEqual(gridRange(2160, 800, 216, 100, 4), { start: 32, end: 64 });
  assert.deepEqual(gridRange(0, 800, 216, 10, 4), { start: 0, end: 10 });
  assert.deepEqual(gridRange(0, 800, 216, 0, 4), { start: 0, end: 0 });
});
