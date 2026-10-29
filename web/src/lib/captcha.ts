// The CAPTCHA widget on the sign-in form. The vendor script loads only here, only when CAPTCHA is on; its hosts
// match the server's Content-Security-Policy.

export type Provider = "recaptcha" | "turnstile" | "hcaptcha";

type Widget = {
  render(el: HTMLElement, opts: Record<string, unknown>): unknown;
  reset(id?: unknown): void;
};

export const vendors: Record<Provider, { script: string; global: string }> = {
  recaptcha: { script: "https://www.google.com/recaptcha/api.js?render=explicit", global: "grecaptcha" },
  turnstile: { script: "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit", global: "turnstile" },
  hcaptcha: { script: "https://js.hcaptcha.com/1/api.js?render=explicit", global: "hcaptcha" },
};

function widget(provider: Provider): Widget | null {
  const w = (window as unknown as Record<string, Widget | undefined>)[vendors[provider].global];
  return w && typeof w.render === "function" ? w : null;
}

function loadScript(provider: Provider): Promise<void> {
  const { script } = vendors[provider];
  let tag = document.head.querySelector<HTMLScriptElement>(`script[src="${script}"]`);
  if (!tag) {
    tag = document.createElement("script");
    tag.src = script;
    tag.async = true;
    document.head.append(tag);
  }
  const el = tag;
  return new Promise((resolve, reject) => {
    if (widget(provider)) return resolve();
    el.addEventListener("error", () => reject(new Error("captcha script failed")), { once: true });
    // The global appears a moment after "load" for some vendors; poll for up to 15 s.
    const end = Date.now() + 15000;
    const poll = () => {
      if (widget(provider)) resolve();
      else if (Date.now() > end) reject(new Error("captcha script did not start"));
      else setTimeout(poll, 50);
    };
    poll();
  });
}

/** Renders the widget into `el`. `onToken` gets the answer, or null when it expires. */
export async function mountCaptcha(
  el: HTMLElement,
  provider: Provider,
  siteKey: string,
  theme: "dark" | "light",
  onToken: (token: string | null) => void,
): Promise<{ reset(): void }> {
  await loadScript(provider);
  const w = widget(provider)!;
  const id = w.render(el, {
    sitekey: siteKey,
    theme,
    callback: (token: string) => onToken(token),
    "expired-callback": () => onToken(null),
  });
  return {
    reset() {
      w.reset(id);
      onToken(null);
    },
  };
}

/** The theme the page shows now: the chosen one, or the device's. */
export function effectiveTheme(): "dark" | "light" {
  const set = document.documentElement.dataset.theme;
  if (set === "dark" || set === "light") return set;
  return matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
