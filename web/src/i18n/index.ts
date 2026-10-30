// Strings for people: the current language, lookups, plurals, and server errors by code.
import { en } from "./en";
import { ptBR } from "./pt-BR";
import type { ApiFailure } from "../lib/api";
import type { Lang, User } from "../lib/types";

type Table = typeof en;
type Plural = { one: string; other: string };
export type Key = { [K in keyof Table]: Table[K] extends string ? K : never }[keyof Table];
export type PluralKey = Exclude<keyof Table, Key>;
type Vars = Record<string, string | number>;

const tables: Record<Lang, Record<string, string | Plural>> = { en, "pt-BR": ptBR };
let current: Lang = "en";

export function setLang(l: Lang): void {
  current = l;
}

export function lang(): Lang {
  return current;
}

/** The user's choice, else the browser's first Portuguese tag, else English. */
export function pickLang(user: User | null, browser: readonly string[]): Lang {
  if (user?.lang) return user.lang;
  return browser.some((l) => l.toLowerCase().startsWith("pt")) ? "pt-BR" : "en";
}

function fill(text: string, vars: Vars = {}): string {
  return text.replace(/\{(\w+)\}/g, (all, name: string) => {
    const v = vars[name];
    if (v === undefined) return all;
    return typeof v === "number" ? new Intl.NumberFormat(current).format(v) : v;
  });
}

export function t(key: Key, vars?: Vars): string {
  return fill(tables[current][key] as string, vars);
}

export function tn(key: PluralKey, n: number, vars: Vars = {}): string {
  const forms = tables[current][key] as Plural;
  const form = new Intl.PluralRules(current).select(n) === "one" ? forms.one : forms.other;
  return fill(form, { ...vars, n });
}

/** Server sentences behind codes that carry more than one, mapped to their keys. */
const sentences: Record<string, Key> = {
  "Enter a valid e-mail address.": "invalid.email",
  "Name is required.": "invalid.name",
  "The scope must be a folder path like /photos.": "invalid.scope",
  "Theme must be dark or light.": "invalid.theme",
  "Send between 1 and 10000 paths.": "invalid.paths",
  "Choose between 1 and 1000 items in one folder.": "invalid.zipItems",
  "Expiry must be 1, 7 or 30 days, or never.": "invalid.shareExpiry",
  "The link password needs 8 to 256 characters.": "invalid.sharePassword",
};

export function errorText(f: ApiFailure): string {
  if (f.code === "invalid" || f.code === "bad_request") {
    const key = sentences[f.serverMessage];
    if (key) return t(key);
    if (f.serverMessage) return f.serverMessage;
  }
  if (f.code === "too_many") {
    const minutes = Math.max(1, Math.ceil((f.retryAfter ?? 60) / 60));
    return t("error.too_many", { minutes: tn("time.minutes", minutes) });
  }
  const key = `error.${f.code}`;
  if (key in en) return t(key as Key);
  return f.serverMessage || t("error.internal");
}
