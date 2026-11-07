import { cn } from "cn";

/** The colours of docs/ui.md#控件 状态标记; "none" is an outlined dot for a state not started. */
export type Tone = "info" | "success" | "warning" | "destructive" | "faint" | "none";

const TONES: Record<Tone, string> = {
  info: "bg-info",
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
  faint: "bg-faint",
  none: "border border-input",
};

/** The dot written before a state's name. */
function StatusDot({ tone }: { tone: Tone }) {
  return <span aria-hidden className={cn("size-2 shrink-0 rounded-full", TONES[tone])} />;
}

/** A state's name after its dot, kept on one line beside however long a name is next to it. */
export function StatusLabel({ tone, label, className }: { tone: Tone; label: string; className?: string }) {
  return (
    <span className={cn("flex shrink-0 items-center gap-1.5 whitespace-nowrap", className)}>
      <StatusDot tone={tone} />
      {label}
    </span>
  );
}
