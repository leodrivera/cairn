// One line at the bottom that says what just happened, then fades: after 4 s, or 8 s when it offers an action.
import { useEffect, useState } from "preact/hooks";

export type ToastAction = { label: string; run(): void };

let current: { id: number; text: string; action?: ToastAction } | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let seq = 0;
const listeners = new Set<() => void>();

function changed(): void {
  for (const fn of listeners) fn();
}

function hide(): void {
  clearTimeout(timer);
  current = null;
  changed();
}

export function showToast(text: string, action?: ToastAction): void {
  current = { id: ++seq, text, action };
  clearTimeout(timer);
  timer = setTimeout(hide, action ? 8000 : 4000);
  changed();
}

export function Toast() {
  const [, setTick] = useState(0);
  useEffect(() => {
    const fn = () => setTick((n) => n + 1);
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  }, []);
  const action = current?.action;
  return (
    <div class="toast-region" role="status">
      {current && (
        <p key={current.id} class={action ? "toast with-action" : "toast"}>
          <span>{current.text}</span>
          {action && (
            <button
              type="button"
              class="button quiet toast-action"
              onClick={() => {
                hide();
                action.run();
              }}
            >
              {action.label}
            </button>
          )}
        </p>
      )}
    </div>
  );
}
