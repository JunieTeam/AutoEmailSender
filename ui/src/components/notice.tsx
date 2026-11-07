import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

/** Fills an empty or failed area with what happened and what to do next (docs/ui.md#控件 空状态). */
export function Notice({
  icon: Icon,
  title,
  detail,
  actions,
  children,
}: {
  icon?: LucideIcon;
  title: string;
  detail?: string;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="grid h-full place-items-center p-6 text-center">
      <div className="max-w-md animate-enter">
        {Icon && <Icon aria-hidden className="mx-auto mb-3 size-9 text-faint" strokeWidth={1.3} />}
        <p className="text-lg font-medium text-balance wrap-anywhere">{title}</p>
        {detail && <p className="mt-1 text-sm text-balance text-muted-foreground wrap-anywhere">{detail}</p>}
        {children}
        {actions && <div className="mt-5 flex flex-wrap justify-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}
