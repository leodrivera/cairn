import "../test/dom";
import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { useState } from "preact/hooks";
import { PasswordInput } from "./password-input";
import { flush, render, setValue } from "../test/dom";
import { setLang } from "../i18n/index";

afterEach(() => setLang("en"));

function Field({ reset = 0 }: { reset?: number }) {
  const [v, setV] = useState("");
  return (
    <form>
      <label for="pw">Password</label>
      <PasswordInput id="pw" value={v} onInput={setV} autocomplete="current-password" resetKey={reset} />
    </form>
  );
}

const input = (el: HTMLElement) => el.querySelector<HTMLInputElement>("#pw")!;
const toggle = (el: HTMLElement) => el.querySelector<HTMLButtonElement>('button[aria-controls="pw"]')!;

test("toggle_shows_and_hides", async () => {
  const el = render(<Field />);
  assert.equal(input(el).type, "password");
  assert.equal(toggle(el).getAttribute("aria-label"), "Show password");
  assert.equal(toggle(el).getAttribute("aria-pressed"), "false");
  assert.equal(toggle(el).type, "button");
  toggle(el).click();
  await flush();
  assert.equal(input(el).type, "text");
  assert.equal(toggle(el).getAttribute("aria-label"), "Hide password");
  assert.equal(toggle(el).getAttribute("aria-pressed"), "true");
  toggle(el).click();
  await flush();
  assert.equal(input(el).type, "password");
});

test("toggle_keeps_value_and_caret", async () => {
  const el = render(<Field />);
  setValue(input(el), "Abcdef1!");
  await flush();
  input(el).setSelectionRange(3, 3);
  toggle(el).click();
  await flush();
  assert.equal(input(el).value, "Abcdef1!");
  assert.equal(input(el).selectionStart, 3);
  assert.equal(input(el).selectionEnd, 3);
});

test("reset_key_hides_again", async () => {
  let el = render(<Field reset={0} />);
  toggle(el).click();
  await flush();
  assert.equal(input(el).type, "text");
  el = render(<Field reset={1} />);
  await flush();
  assert.equal(input(el).type, "password");
});

test("toggle_strings_in_pt_br", async () => {
  setLang("pt-BR");
  const el = render(<Field />);
  assert.equal(toggle(el).getAttribute("aria-label"), "Mostrar senha");
  toggle(el).click();
  await flush();
  assert.equal(toggle(el).getAttribute("aria-label"), "Ocultar senha");
});
