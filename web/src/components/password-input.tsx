// A password field with a button inside its right edge that shows or hides what was typed.
import type { Ref } from "preact";
import { useEffect, useState } from "preact/hooks";
import { t } from "../i18n/index";
import { Icon } from "./icons";

type Props = {
  id: string;
  value: string;
  onInput(v: string): void;
  autocomplete: string;
  class?: string;
  inputRef?: Ref<HTMLInputElement>;
  required?: boolean;
  maxLength?: number;
  describedBy?: string;
  /** A new value hides the text again; callers change it after a successful submit. */
  resetKey?: unknown;
};

export function PasswordInput(p: Props) {
  const [shown, setShown] = useState(false);
  useEffect(() => setShown(false), [p.resetKey]);
  return (
    <div class="password-field">
      <input
        id={p.id}
        ref={p.inputRef}
        class={p.class ?? "input"}
        // One element for both states keeps the caret; the JSX types split inputs by `type`, hence the cast.
        type={(shown ? "text" : "password") as "text"}
        autocomplete={p.autocomplete}
        autocapitalize="off"
        spellcheck={false}
        required={p.required}
        maxLength={p.maxLength}
        aria-describedby={p.describedBy}
        value={p.value}
        onInput={(e) => p.onInput(e.currentTarget.value)}
      />
      <button
        type="button"
        class="icon-button password-toggle"
        // One name for both states: the pressed state tells whether the text shows.
        aria-label={t("password.show")}
        aria-pressed={shown}
        aria-controls={p.id}
        // Keeps focus (and the caret) in the field when the button is pressed with a pointer.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => setShown(!shown)}
      >
        <Icon name={shown ? "eyeOff" : "eye"} />
      </button>
    </div>
  );
}
