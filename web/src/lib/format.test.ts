import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { formatSize, formatWhen } from "./format";
import { setLang } from "../i18n/index";

afterEach(() => setLang("en"));

test("sizes", () => {
  assert.equal(formatSize(0), "0 B");
  assert.equal(formatSize(812), "812 B");
  assert.equal(formatSize(1024), "1 KB");
  assert.equal(formatSize(186_368), "182 KB");
  assert.equal(formatSize(4_404_019), "4.2 MB");
  assert.equal(formatSize(1_932_735_283), "1.8 GB");
  assert.equal(formatSize(1024 ** 4), "1 TB");
  setLang("pt-BR");
  assert.equal(formatSize(4_404_019), "4,2 MB");
});

// Local times, so the test does not depend on the machine's time zone.
const now = new Date(2026, 9, 29, 15, 0);
const sec = (d: Date) => Math.floor(d.getTime() / 1000);

test("when", () => {
  assert.equal(formatWhen(sec(new Date(2026, 9, 29, 9, 14)), now), "Today, 09:14");
  assert.equal(formatWhen(sec(new Date(2026, 9, 28, 22, 0)), now), "Yesterday");
  assert.equal(formatWhen(sec(new Date(2026, 9, 24, 8, 0)), now), "Oct 24");
  assert.equal(formatWhen(sec(new Date(2024, 9, 24, 8, 0)), now), "Oct 24, 2024");
  setLang("pt-BR");
  assert.equal(formatWhen(sec(new Date(2026, 9, 29, 9, 14)), now), "Hoje, 09:14");
  assert.equal(formatWhen(sec(new Date(2026, 9, 28, 22, 0)), now), "Ontem");
  assert.equal(formatWhen(sec(new Date(2026, 9, 24, 8, 0)), now), "24 de out.");
});
