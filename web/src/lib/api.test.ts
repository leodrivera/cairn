import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { api, ApiFailure, onSignedOut } from "./api";
import { json, stubFetch } from "../test/dom";

test("sends X-Cairn on POST, not on GET", async () => {
  const log = stubFetch({ "POST /api/folders/a": () => json({}, 201), "GET /api/files": () => json({}) });
  await api("POST", "/api/folders/a", { x: 1 });
  await api("GET", "/api/files");
  assert.equal(log[0].headers.get("x-cairn"), "1");
  assert.equal(log[0].headers.get("content-type"), "application/json");
  assert.equal(await log[0].text(), '{"x":1}');
  assert.equal(log[1].headers.get("x-cairn"), null);
});

test("parses the error body", async () => {
  stubFetch({ "POST /api/ops": () => json({ error: "exists", message: "Something with that name is already there." }, 409) });
  await assert.rejects(api("POST", "/api/ops", {}), (e: unknown) => {
    assert.ok(e instanceof ApiFailure);
    assert.equal(e.status, 409);
    assert.equal(e.code, "exists");
    assert.equal(e.serverMessage, "Something with that name is already there.");
    return true;
  });
});

test("reads Retry-After", async () => {
  stubFetch({ "POST /api/auth/login": () => json({ error: "too_many", message: "Wait." }, 429, { "retry-after": "120" }) });
  await assert.rejects(api("POST", "/api/auth/login", {}), (e: ApiFailure) => e.retryAfter === 120);
});

test("a network error is offline", async () => {
  globalThis.fetch = (async () => {
    throw new TypeError("Failed to fetch");
  }) as typeof fetch;
  await assert.rejects(api("GET", "/api/me"), (e: ApiFailure) => e.status === 0 && e.code === "offline");
});

test("a body that is not JSON is internal", async () => {
  stubFetch({ "GET /api/me": () => new Response("<html>Bad gateway</html>", { status: 502 }) });
  await assert.rejects(api("GET", "/api/me"), (e: ApiFailure) => e.status === 502 && e.code === "internal");
});

test("204 resolves undefined", async () => {
  stubFetch({ "DELETE /api/users/2": () => new Response(null, { status: 204 }) });
  assert.equal(await api("DELETE", "/api/users/2"), undefined);
});

test("signed_out calls the handler before rejecting", async () => {
  let called = 0;
  onSignedOut(() => called++);
  stubFetch({ "GET /api/files": () => json({ error: "signed_out", message: "Sign in first." }, 403) });
  await assert.rejects(api("GET", "/api/files"), (e: ApiFailure) => e.code === "signed_out" && called === 1);
});
