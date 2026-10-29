// One line at the bottom that says what just happened, then fades after 4 s.
import { useEffect, useState } from "preact/hooks";

let current: { id: number; text: string } | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let seq = 0;
const listeners = new Set<() => void>();

function changed(): void {
  for (const fn of listeners) fn();
}

export function showToast(text: string): void {
  current = { id: ++seq, text };
  clearTimeout(timer);
  timer = setTimeout(() => {
    current = null;
    changed();
  }, 4000);
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
  return (
    <div class="toast-region" role="status">
      {current && (
        <p key={current.id} class="toast">
          {current.text}
        </p>
      )}
    </div>
  );
}
