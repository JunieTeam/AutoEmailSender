import type { ErrorComponentProps } from "@tanstack/react-router";
import { useEffect } from "react";
import { failureOf } from "@/api/call";
import { FailureDetails } from "@/components/failure";
import { Notice } from "@/components/notice";
import { Button } from "@/components/ui/button";
import { logUnexplained } from "@/ui-errors";

/** Takes the place of a page that could not be shown (docs/ui.md#交互 出错). */
export function RouteError({ error }: ErrorComponentProps) {
  useEffect(() => logUnexplained(error), [error]);
  const failure = failureOf(error);
  return (
    <Notice title={failure.message} actions={<Button onClick={() => location.reload()}>重新加载</Button>}>
      <FailureDetails className="mt-1" failure={failure} />
    </Notice>
  );
}
