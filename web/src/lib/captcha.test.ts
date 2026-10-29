import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { mountCaptcha } from "./captcha";

test("the vendor script is added once", async () => {
  const calls: Record<string, unknown>[] = [];
  (window as unknown as { turnstile: unknown }).turnstile = {
    render: (_el: HTMLElement, o: Record<string, unknown>) => {
      calls.push(o);
      return "w";
    },
    reset: () => {},
  };
  const a = document.createElement("div");
  const b = document.createElement("div");
  await mountCaptcha(a, "turnstile", "k", "dark", () => {});
  await mountCaptcha(b, "turnstile", "k", "light", () => {});
  const scripts = document.head.querySelectorAll('script[src^="https://challenges.cloudflare.com/turnstile/v0/api.js"]');
  assert.equal(scripts.length, 1);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].theme, "dark");
  assert.equal(calls[1].theme, "light");
});
