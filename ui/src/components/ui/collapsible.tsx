import { Collapsible as CollapsiblePrimitive } from "@base-ui/react/collapsible";
import { ChevronRight } from "lucide-react";
import { cn } from "cn";

function Collapsible({ ...props }: CollapsiblePrimitive.Root.Props) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

/** A line of text after a chevron that turns down while open (docs/ui.md#控件). */
function CollapsibleTrigger({ className, children, ...props }: CollapsiblePrimitive.Trigger.Props) {
  return (
    <CollapsiblePrimitive.Trigger
      data-slot="collapsible-trigger"
      className={cn(
        "flex min-h-8 items-center gap-1 text-left text-muted-foreground transition-colors hover:text-foreground",
        className,
      )}
      {...props}
    >
      <ExpandChevron />
      {children}
    </CollapsiblePrimitive.Trigger>
  );
}

/**
 * docs/ui.md#动效: opens and closes by smoothly changing its height. The spacing goes on an inner
 * element, since padding on the panel itself would stay visible at height 0.
 */
function CollapsibleContent({
  className,
  children,
  ...props
}: Omit<CollapsiblePrimitive.Panel.Props, "className"> & { className?: string }) {
  return (
    <CollapsiblePrimitive.Panel
      data-slot="collapsible-content"
      className="h-(--collapsible-panel-height) overflow-hidden transition-[height,opacity] duration-240 ease-out data-ending-style:h-0 data-ending-style:opacity-0 data-ending-style:duration-160 data-ending-style:ease-in data-starting-style:h-0 data-starting-style:opacity-0"
      {...props}
    >
      <div className={className}>{children}</div>
    </CollapsiblePrimitive.Panel>
  );
}

/** Points right, and down while the control it is in is expanded. */
function ExpandChevron({ className }: { className?: string }) {
  return (
    <ChevronRight
      aria-hidden
      strokeWidth={1.6}
      className={cn(
        "size-4 shrink-0 text-faint transition-transform duration-180 in-aria-expanded:rotate-90",
        className,
      )}
    />
  );
}

export { Collapsible, CollapsibleTrigger, CollapsibleContent, ExpandChevron };
