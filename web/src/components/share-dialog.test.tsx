import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { ShareDialog } from "./share-dialog";
import { Toast } from "./toast";
import { buttonNamed, byLabel, flush, json, render, setValue, stubFetch, waitFor } from "../test/dom";
import { errorText, setLang } from "../i18n/index";
import { ApiFailure } from "../lib/api";
import type { Share } from "../lib/types";

const share: Share = {
  id: "k7Qw3nTzAbCdEfGhIjKl-_",
  url: "https://files.example.com/s/k7Qw3nTzAbCdEfGhIjKl-_",
  path: "/photos/2026/trail-ridge.jpg",
  name: "trail-ridge.jpg",
  expiresAt: 1_900_000_000,
  hasPassword: false,
  download: true,
  hits: 0,
  createdAt: 1_800_000_000,
  owner: { id: 1, name: "Admin" },
};

function open(kind: "file" | "folder" = "file", onClose = () => {}) {
  return render(
    <>
      <ShareDialog entryPath="photos/2026/trail-ridge.jpg" kind={kind} onClose={onClose} />
      <Toast />
    </>,
  );
}

function linkInput(el: HTMLElement): HTMLInputElement | null {
  return el.querySelector<HTMLInputElement>('[role="dialog"] input[readonly]');
}

function clipboard(write: (text: string) => Promise<void>): () => void {
  const had = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
  return () => {
    if (had) Object.defineProperty(navigator, "clipboard", had);
    else delete (navigator as { clipboard?: unknown }).clipboard;
  };
}

async function created(el: HTMLElement) {
  buttonNamed(el, "Create link").click();
  await waitFor(() => assert.ok(linkInput(el)));
}

test("creates with defaults", async () => {
  const log = stubFetch({ "POST /api/shares": () => json({ share }, 201) });
  const el = open();
  assert.match(el.textContent!, /Anything a browser shows can still be saved\./);
  assert.equal(byLabel<HTMLSelectElement>(el, "Expires").value, "7");
  await created(el);
  assert.deepEqual(await log[0].json(), {
    path: "photos/2026/trail-ridge.jpg",
    expiresDays: 7,
    password: null,
    download: true,
  });
  assert.equal(log[0].headers.get("x-cairn"), "1");
  assert.equal(linkInput(el)!.value, "https://files.example.com/s/k7Qw3nTzAbCdEfGhIjKl-_");
  assert.match(el.textContent!, /Anyone with this link can open the file/);
});

test("expiry choices are sent as days or null", async () => {
  const log = stubFetch({ "POST /api/shares": () => json({ share }, 201) });
  const el = open();
  const select = byLabel<HTMLSelectElement>(el, "Expires");
  assert.deepEqual(
    [...select.options].map((o) => o.textContent),
    ["In 24 hours", "In 7 days", "In 30 days", "Never"],
  );
  setValue(select, "never");
  await flush();
  await created(el);
  assert.equal((await log[0].json()).expiresDays, null);
});

test("password switch shows the field and sends it", async () => {
  const log = stubFetch({ "POST /api/shares": () => json({ share: { ...share, hasPassword: true } }, 201) });
  const el = open();
  assert.throws(() => byLabel(el, "Link password"));
  byLabel(el, "Require a password").click();
  await flush();
  setValue(byLabel(el, "Link password"), "s3cret-pass");
  await flush();
  await created(el);
  assert.equal((await log[0].json()).password, "s3cret-pass");
});

test("a password under 8 characters blocks Create", async () => {
  stubFetch({});
  const el = open();
  byLabel(el, "Require a password").click();
  await flush();
  assert.ok(buttonNamed(el, "Create link").disabled);
  setValue(byLabel(el, "Link password"), "1234567");
  await flush();
  assert.ok(buttonNamed(el, "Create link").disabled);
  assert.match(el.textContent!, /At least 8 characters\./);
  setValue(byLabel(el, "Link password"), "12345678");
  await flush();
  assert.ok(!buttonNamed(el, "Create link").disabled);
});

test("url fallback is this origin", async () => {
  stubFetch({ "POST /api/shares": () => json({ share: { ...share, url: null } }, 201) });
  const el = open();
  await created(el);
  assert.equal(linkInput(el)!.value, `${location.origin}/s/k7Qw3nTzAbCdEfGhIjKl-_`);
});

test("copy writes the clipboard, or says it could not", async () => {
  stubFetch({ "POST /api/shares": () => json({ share }, 201) });
  const written: string[] = [];
  let restore = clipboard(async (text) => void written.push(text));
  const el = open();
  await created(el);
  buttonNamed(el, "Copy").click();
  await waitFor(() => assert.match(el.textContent!, /Link copied\./));
  assert.deepEqual(written, [share.url]);
  restore();
  restore = clipboard(async () => {
    throw new Error("denied");
  });
  buttonNamed(el, "Copy").click();
  await waitFor(() => assert.match(el.textContent!, /Could not copy\. Select the link and copy it\./));
  restore();
});

test("Done sends only what changed", async () => {
  const log = stubFetch({
    "POST /api/shares": () => json({ share }, 201),
    [`PATCH /api/shares/${share.id}`]: () => json({ share: { ...share, download: false } }),
  });
  let closed = 0;
  let el = open("file", () => closed++);
  await created(el);
  buttonNamed(el, "Done").click();
  await waitFor(() => assert.equal(closed, 1));
  assert.equal(log.filter((r) => r.method === "PATCH").length, 0);
  el = open("file", () => closed++);
  await created(el);
  byLabel(el, "Allow download").click();
  await flush();
  buttonNamed(el, "Done").click();
  await waitFor(() => assert.equal(closed, 2));
  const patches = log.filter((r) => r.method === "PATCH");
  assert.equal(patches.length, 1);
  assert.deepEqual(await patches[0].json(), { download: false });
});

test("delete link", async () => {
  const log = stubFetch({
    "POST /api/shares": () => json({ share }, 201),
    [`DELETE /api/shares/${share.id}`]: () => new Response(null, { status: 204 }),
  });
  let closed = 0;
  const el = open("file", () => closed++);
  await created(el);
  buttonNamed(el, "Delete link").click();
  await waitFor(() => assert.equal(closed, 1));
  assert.ok(log.some((r) => r.method === "DELETE"));
  await waitFor(() => assert.match(el.textContent!, /Deleted the link to trail-ridge\.jpg\./));
});

test("folder lead", async () => {
  stubFetch({ "POST /api/shares": () => json({ share }, 201) });
  const el = open("folder");
  await created(el);
  assert.match(el.textContent!, /Anyone with this link can open the folder/);
});

test("create error shows in the dialog", async () => {
  stubFetch({ "POST /api/shares": () => json({ error: "forbidden", message: "x" }, 403) });
  const el = open();
  buttonNamed(el, "Create link").click();
  await waitFor(() => assert.match(el.querySelector('[role="alert"]')!.textContent!, /You do not have permission/));
});

test("escape closes", async () => {
  stubFetch({});
  let closed = 0;
  const el = open("file", () => closed++);
  el.querySelector('[role="dialog"]')!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
  );
  await flush();
  assert.equal(closed, 1);
});

test("pt-BR strings", async () => {
  stubFetch({});
  setLang("pt-BR");
  try {
    const el = open();
    assert.ok(buttonNamed(el, "Criar link"));
    assert.match(el.textContent!, /Exigir senha/);
  } finally {
    setLang("en");
  }
});

test("share error codes are translated", () => {
  setLang("pt-BR");
  try {
    const text = (code: string) => errorText(new ApiFailure(403, code, "x"));
    assert.equal(text("share_gone"), "Este link não funciona mais. Peça um novo a quem enviou.");
    assert.equal(text("share_locked"), "Este link pede senha.");
    assert.equal(text("wrong_link_password"), "Essa senha não está certa.");
    assert.equal(text("wrong_password"), "A senha atual está errada.");
    assert.equal(text("download_off"), "O download está desligado neste link.");
    const invalid = (m: string) => errorText(new ApiFailure(400, "invalid", m));
    assert.equal(invalid("The link password needs 8 to 256 characters."), "A senha do link precisa ter de 8 a 256 caracteres.");
    assert.equal(invalid("Expiry must be 1, 7 or 30 days, or never."), "A validade deve ser 1, 7 ou 30 dias, ou nunca.");
  } finally {
    setLang("en");
  }
});

test("a name that is not UTF-8 shows readable in the dialog", async () => {
  stubFetch({});
  const el = render(<ShareDialog entryPath={"\u0000UmVsYXTzcmlv/notes.txt"} kind="file" onClose={() => {}} />);
  const shown = el.querySelector(".share-path")!.textContent!;
  assert.equal(shown, "Relat\uFFFDrio/notes.txt");
});
