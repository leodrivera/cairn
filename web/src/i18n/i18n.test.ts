import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { errorText, pickLang, setLang, t, tn } from "./index";
import { ApiFailure } from "../lib/api";
import type { User } from "../lib/types";

afterEach(() => setLang("en"));

const user = (lang: User["lang"]): User => ({
  id: 1,
  email: "admin@example.com",
  name: "Admin",
  scope: "/",
  permissions: { read: true, write: true, share: true, admin: true },
  lang,
  theme: null,
  view: null,
  hasPassword: true,
});

test("pick_lang", () => {
  assert.equal(pickLang(null, ["pt-PT", "en"]), "pt-BR");
  assert.equal(pickLang(user("en"), ["pt-BR"]), "en");
  assert.equal(pickLang(user(null), ["de", "pt"]), "pt-BR");
  assert.equal(pickLang(null, ["de"]), "en");
  assert.equal(pickLang(null, []), "en");
});

test("plural in en and pt-BR", () => {
  assert.equal(tn("files.moved", 1, { dest: "Archive" }), "Moved 1 item to Archive.");
  assert.equal(tn("files.moved", 3, { dest: "Archive" }), "Moved 3 items to Archive.");
  setLang("pt-BR");
  assert.equal(tn("files.moved", 1, { dest: "Archive" }), "1 item movido para Archive.");
  assert.equal(tn("files.moved", 1200, { dest: "Archive" }), "1.200 itens movidos para Archive.");
});

test("placeholders", () => {
  assert.equal(t("login.continueWith", { name: "Google" }), "Continue with Google");
});

const fail = (status: number, code: string, message: string, retryAfter: number | null = null) =>
  new ApiFailure(status, code, message, retryAfter);

test("error by code", () => {
  setLang("pt-BR");
  assert.equal(
    errorText(fail(409, "exists", "Something with that name is already there.")),
    "Já existe algo com esse nome aqui. Escolha outro nome ou substitua.",
  );
});

test("unknown code uses the server message", () => {
  setLang("pt-BR");
  assert.equal(errorText(fail(400, "brand_new", "A sentence from a newer server.")), "A sentence from a newer server.");
});

test("invalid is translated by its sentence", () => {
  setLang("pt-BR");
  assert.equal(errorText(fail(400, "invalid", "Enter a valid e-mail address.")), "Digite um e-mail válido.");
  assert.equal(errorText(fail(400, "invalid", "Some other rule.")), "Some other rule.");
});

test("too_many uses minutes from Retry-After", () => {
  assert.equal(
    errorText(fail(429, "too_many", "x", 61)),
    "Too many tries in a row. Wait 2 minutes, then try again.",
  );
  assert.equal(errorText(fail(429, "too_many", "x", 30)), "Too many tries in a row. Wait 1 minute, then try again.");
});

test("offline", () => {
  assert.equal(
    errorText(fail(0, "offline", "")),
    "Can't reach the server. Check that Cairn is running, then try again.",
  );
});

test("pt-BR says 0 in the plural", () => {
  setLang("pt-BR");
  try {
    assert.equal(tn("links.times", 0), "Aberto 0 vezes");
    assert.equal(tn("links.times", 1), "Aberto 1 vez");
    assert.equal(tn("links.times", 3), "Aberto 3 vezes");
  } finally {
    setLang("en");
  }
  assert.equal(tn("links.times", 0), "Opened 0 times");
  assert.equal(tn("links.times", 1), "Opened 1 time");
});

test("new_menu_strings_in_pt_br", () => {
  setLang("pt-BR");
  assert.equal(t("explorer.new"), "Novo");
  assert.equal(t("explorer.newTextFile"), "Novo arquivo de texto");
});

test("trash_strings_in_pt_br", () => {
  setLang("pt-BR");
  assert.equal(t("nav.sharedWithMe"), "Compartilhados comigo");
  assert.equal(t("nav.trash"), "Lixeira");
  assert.equal(t("dialog.moveToTrash"), "Mover para a lixeira");
  assert.equal(t("dialog.deleteForever"), "Excluir permanentemente");
  assert.equal(t("trash.restore"), "Restaurar");
  assert.equal(t("trash.emptyTrash"), "Esvaziar lixeira");
  assert.equal(tn("files.trashed", 2), "2 itens foram para a lixeira.");
});

test("sharing_strings_in_pt_br", () => {
  setLang("pt-BR");
  assert.equal(t("people.canView"), "Pode ver");
  assert.equal(t("people.canEdit"), "Pode editar");
  assert.equal(t("shared.leave"), "Sair");
  assert.equal(t("error.grant_gone"), "Este item compartilhado não está mais disponível.");
});

test("holds_mount_in_pt_br", () => {
  setLang("pt-BR");
  const f = new ApiFailure(409, "holds_mount", "media has another disk mounted inside. Unmount it first.");
  assert.equal(errorText(f, { name: "media" }), "media tem outro disco montado dentro. Desmonte-o primeiro.");
});

test("grant_expiry_strings_in_pt_br", () => {
  setLang("pt-BR");
  assert.equal(t("grants.until", { date: "14 de mar. de 2027" }), "Até 14 de mar. de 2027");
  assert.equal(t("grants.changeTo"), "Mudar para");
});

test("trash_and_grant_sentences_in_pt_br", () => {
  setLang("pt-BR");
  const say = (m: string) => errorText(new ApiFailure(400, "invalid", m));
  assert.equal(say("Name between 1 and 20 e-mail addresses."), "Informe entre 1 e 20 endereços de e-mail.");
  assert.equal(say("Send between 1 and 10000 items."), "Envie entre 1 e 10000 itens.");
  assert.equal(say("Keep trash between 0 and 3650 days."), "Mantenha a lixeira entre 0 e 3650 dias.");
});

test("restore_other_disk_in_pt_br", () => {
  setLang("pt-BR");
  const f = new ApiFailure(409, "restore_other_disk", "a.txt cannot be restored: its folder is now on another disk.");
  assert.equal(errorText(f, { name: "a.txt" }), "a.txt não pode ser restaurado: a pasta dele agora está em outro disco.");
});

test("trash_denied_in_pt_br", () => {
  setLang("pt-BR");
  const f = new ApiFailure(409, "trash_denied", "The trash cannot be made here: Cairn may not write to /photos. Ask an admin, or delete permanently.");
  assert.equal(errorText(f), "A lixeira não pode ser criada aqui: o Cairn não tem permissão de escrita na pasta do topo. Fale com um admin ou exclua permanentemente.");
});
