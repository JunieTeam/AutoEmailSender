import { cn } from "cn";
import type { Failure } from "@/api/types";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/**
 * The outside words a reason did not recognize, folded under it: some users can act on them, and a
 * screenshot of them tells the developer what happened (docs/ui.md#交互 出错).
 */
export function FailureDetails({ failure, className }: { failure: Failure; className?: string }) {
  if (!failure.details) return null;
  return (
    <Collapsible className={cn("text-xs", className)}>
      <CollapsibleTrigger className="min-h-6">详细信息</CollapsibleTrigger>
      <CollapsibleContent className="pb-1">
        <p className="text-left whitespace-pre-wrap text-muted-foreground wrap-anywhere">{failure.details}</p>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Why something failed, after what failed if `lead` says it, with its details underneath. */
export function FailureNote({ failure, lead, className }: { failure: Failure; lead?: string; className?: string }) {
  return (
    <div className={className}>
      <p className="wrap-anywhere">
        {lead}
        {failure.message}
      </p>
      <FailureDetails failure={failure} />
    </div>
  );
}
