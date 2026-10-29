// Empty and error states: say what happened and what to do next. No illustrations.
import type { ComponentChildren } from "preact";
import { Icon, type IconName } from "./icons";

type Props = {
  icon: IconName;
  tone?: "accent" | "muted" | "danger";
  title: string;
  body: string;
  children?: ComponentChildren;
};

export function StatePanel({ icon, tone = "muted", title, body, children }: Props) {
  return (
    <section class="state" role="status">
      <span class={`state-icon tone-${tone}`}>
        <Icon name={icon} size={28} />
      </span>
      <h2>{title}</h2>
      <p>{body}</p>
      {children}
    </section>
  );
}
