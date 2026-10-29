// Shapes of what the server sends. Field names follow the JSON.

export type Lang = "en" | "pt-BR";

export type Perms = { read: boolean; write: boolean; share: boolean; admin: boolean };

export type User = {
  id: number;
  email: string;
  name: string;
  scope: string;
  permissions: Perms;
  lang: Lang | null;
  theme: "dark" | "light" | null;
  view: "list" | "gallery" | null;
  hasPassword: boolean;
};

export type Entry = {
  name: string;
  kind: "folder" | "file" | "link";
  size: number;
  /** Unix seconds. */
  modified: number;
  mime: string;
  outside?: true;
  /** Only for a name that is not UTF-8: the path segment to use for it. */
  rawName?: string;
};

export type Listing = { path: string; entries: Entry[]; space?: { used: number; total: number } };

export type Config = {
  authMethods: ("password" | "oauth" | "oidc")[];
  captcha: { provider: "recaptcha" | "turnstile" | "hcaptcha"; siteKey: string } | null;
  providers: { name: string; href: string }[];
  setupNeeded?: true;
  theme: "dark" | "light" | null;
  thumbnails: boolean;
  editMax: number;
};

export type Me = { user: User | null; method?: string; security?: { captchaDisabled: boolean } };
