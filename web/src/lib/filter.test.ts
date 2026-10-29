import { test } from "node:test";
import assert from "node:assert/strict";
import { filterEntries, fold } from "./filter";
import { file, folder } from "../test/fixtures";

test("accent and case insensitive, server order kept", () => {
  const entries = [folder("Café"), file("Report #1 (50%) ✓ café.txt"), file("notes.txt")];
  assert.deepEqual(
    filterEntries(entries, "CAFE").map((e) => e.name),
    ["Café", "Report #1 (50%) ✓ café.txt"],
  );
  assert.equal(filterEntries(entries, "  ").length, 3);
  assert.equal(fold("Ação"), "acao");
});
