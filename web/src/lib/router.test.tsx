import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { href, navigate, parseRoute, Link, useRoute } from "./router";
import { flush, render } from "../test/dom";

test("parses each route", () => {
  assert.deepEqual(parseRoute("/", ""), { name: "files", path: "" });
  assert.deepEqual(parseRoute("/files/", ""), { name: "files", path: "" });
  assert.deepEqual(parseRoute("/files", ""), { name: "files", path: "" });
  assert.deepEqual(parseRoute("/files/photos/2026/", ""), { name: "files", path: "photos/2026" });
  assert.deepEqual(parseRoute("/view/notes/a.txt", ""), { name: "view", path: "notes/a.txt" });
  assert.deepEqual(parseRoute("/settings", ""), { name: "settings", page: "profile" });
  assert.deepEqual(parseRoute("/settings/users", ""), { name: "settings", page: "users" });
  assert.deepEqual(parseRoute("/settings/nope", ""), { name: "missing" });
  assert.deepEqual(parseRoute("/links", ""), { name: "links" });
  // The old address of the page, from before it left the settings, still opens it.
  assert.deepEqual(parseRoute("/settings/links", ""), { name: "links" });
  assert.deepEqual(parseRoute("/login", ""), { name: "login", next: null });
  assert.deepEqual(parseRoute("/nowhere", ""), { name: "missing" });
});

test("share routes", () => {
  const id = "k7Qw3nTzAbCdEfGhIjKl-_";
  assert.deepEqual(parseRoute(`/s/${id}`, ""), { name: "share", id, path: "", file: false });
  assert.deepEqual(parseRoute(`/s/${id}`, "?path=sub%2Fa"), { name: "share", id, path: "sub/a", file: false });
  assert.deepEqual(parseRoute(`/s/${id}`, "?file=sub%2Fa.txt"), { name: "share", id, path: "sub/a.txt", file: true });
  assert.deepEqual(parseRoute("/s/short", ""), { name: "missing" });
  assert.deepEqual(parseRoute(`/s/${id}/`, ""), { name: "missing" });
  assert.equal(href({ name: "share", id, path: "a b/c", file: true }), `/s/${id}?file=a%20b%2Fc`);
  assert.equal(href({ name: "share", id, path: "", file: false }), `/s/${id}`);
});

test("hrefs", () => {
  assert.equal(href({ name: "files", path: "" }), "/files/");
  assert.equal(href({ name: "files", path: "a b" }), "/files/a%20b");
  assert.equal(href({ name: "settings", page: "server" }), "/settings/server");
  assert.equal(href({ name: "links" }), "/links");
  assert.equal(href({ name: "login", next: "/files/a" }), "/login?next=%2Ffiles%2Fa");
  assert.equal(href({ name: "login", next: null }), "/login");
});

test("malformed escape is missing", () => {
  assert.deepEqual(parseRoute("/files/%E0%A4%A", ""), { name: "missing" });
  assert.deepEqual(parseRoute("/files/a%2Fb", ""), { name: "missing" });
  assert.deepEqual(parseRoute("/view/", ""), { name: "missing" });
});

test("reserved prefixes", () => {
  assert.deepEqual(parseRoute("/s/abc", ""), { name: "missing" });
  assert.deepEqual(parseRoute("/auth/sso", ""), { name: "missing" });
});

test("login next", () => {
  assert.deepEqual(parseRoute("/login", "?next=%2Ffiles%2Fphotos"), { name: "login", next: "/files/photos" });
  // Only a path on this site is a place to go back to.
  assert.deepEqual(parseRoute("/login", "?next=https%3A%2F%2Fevil.example.net"), { name: "login", next: null });
  assert.deepEqual(parseRoute("/login", "?next=%2F%2Fevil.example.net"), { name: "login", next: null });
});

test("navigate updates useRoute and Link navigates", async () => {
  function Where() {
    const r = useRoute();
    return (
      <p>
        <span id="where">{r.name === "files" ? `files:${r.path}` : r.name}</span>
        <Link href="/settings/profile">Profile</Link>
      </p>
    );
  }
  navigate("/files/a");
  const el = render(<Where />);
  assert.equal(el.querySelector("#where")!.textContent, "files:a");
  navigate("/files/b");
  await flush();
  assert.equal(el.querySelector("#where")!.textContent, "files:b");
  el.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  await flush();
  assert.equal(location.pathname, "/settings/profile");
  assert.equal(el.querySelector("#where")!.textContent, "settings");
  history.back();
  await flush(10);
  assert.equal(el.querySelector("#where")!.textContent, "files:b");
});

test("login error parsed", () => {
  assert.deepEqual(parseRoute("/login", "?error=sso"), { name: "login", next: null, error: "sso" });
  assert.deepEqual(parseRoute("/login", "?error=no-account"), { name: "login", next: null, error: "no-account" });
  assert.deepEqual(parseRoute("/login", "?error=other"), { name: "login", next: null });
  assert.equal(href({ name: "login", next: null, error: "sso" }), "/login");
});

test("trash route", () => {
  assert.deepEqual(parseRoute("/trash", ""), { name: "trash" });
  assert.deepEqual(parseRoute("/trash/", ""), { name: "trash" });
  assert.equal(href({ name: "trash" }), "/trash");
});

test("a share page only takes paths beneath its link", () => {
  const id = "a".repeat(22);
  for (const key of ["file", "path"]) {
    for (const bad of ["..", ".", "../..", "a/../b", "a/./b", "a//b", "/a", "a/.."]) {
      assert.deepEqual(parseRoute(`/s/${id}`, `?${key}=${encodeURIComponent(bad)}`), { name: "missing" }, bad);
    }
  }
  assert.deepEqual(parseRoute(`/s/${id}`, "?file=%2e%2e%2fb"), { name: "missing" });
  assert.deepEqual(parseRoute(`/s/${id}`, "?path=..a%2F.b"), { name: "share", id, path: "..a/.b", file: false });
  assert.deepEqual(parseRoute(`/s/${id}`, "?path=a%2F"), { name: "share", id, path: "a", file: false });
});

test("shared routes", () => {
  const id = "AbCdEfGhIjKlMnOpQrSt-_";
  assert.deepEqual(parseRoute("/shared", ""), { name: "shared" });
  assert.deepEqual(parseRoute(`/shared/${id}`, ""), { name: "grant", id, path: "", view: false });
  assert.deepEqual(parseRoute(`/shared/${id}/files/a/b`, ""), { name: "grant", id, path: "a/b", view: false });
  assert.deepEqual(parseRoute(`/shared/${id}/view/a.txt`, ""), { name: "grant", id, path: "a.txt", view: true });
  for (const r of [
    { name: "grant" as const, id, path: "", view: false },
    { name: "grant" as const, id, path: "a/b c", view: false },
    { name: "grant" as const, id, path: "a.txt", view: true },
  ])
    assert.deepEqual(parseRoute(href(r), ""), r);
  assert.equal(href({ name: "shared" }), "/shared");
  assert.deepEqual(parseRoute("/shared/short", ""), { name: "missing" });
});
