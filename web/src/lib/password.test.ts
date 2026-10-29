import { test } from "node:test";
import assert from "node:assert/strict";
import { passwordOk } from "./password";

test("password rule", () => {
  assert.ok(passwordOk("Guest-pass1"));
  assert.ok(passwordOk("Café ✓ ok"));
  assert.ok(!passwordOk("short-A"));
  assert.ok(!passwordOk("lowercase-only"));
  assert.ok(!passwordOk("UPPERCASE-ONLY"));
  assert.ok(!passwordOk("NoSpecial1"));
  assert.ok(!passwordOk(`Aa-${"x".repeat(254)}`));
});
