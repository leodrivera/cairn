import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { Icon } from "./icons";
import { render } from "../test/dom";

test("more_icon_is_filled_dots", () => {
  const el = render(<Icon name="more" />);
  const dots = [...el.querySelectorAll("circle")];
  assert.equal(dots.length, 3);
  for (const d of dots) {
    assert.ok(Number(d.getAttribute("r")) >= 1.75, `r=${d.getAttribute("r")}`);
    assert.equal(d.getAttribute("fill"), "currentColor");
  }
});
