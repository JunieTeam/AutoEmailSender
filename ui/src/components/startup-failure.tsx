import { useMutation } from "@tanstack/react-query";
import appIcon from "../../../docs/icon.svg";
import { exportLogs } from "@/api/app";
import { failureOf } from "@/api/call";
import type { StartupFailure as Failed } from "@/api/types";
import { FailureNote } from "@/components/failure";
import { Button } from "@/components/ui/button";
import { SUPPORT_ADDRESS } from "@/legal";

/**
 * Shown in place of the app when the core could not start (docs/ui.md#页面). Signed-in state cannot
 * be read, so the developer is reached by email.
 */
export function StartupFailure({ failure }: { failure: Failed }) {
  const exported = useMutation({ mutationFn: exportLogs });
  const copy = useMutation({ mutationFn: () => navigator.clipboard.writeText(failure.diagnostic) });
  return (
    <main className="grid h-dvh place-items-center p-6">
      <div className="w-full max-w-[520px] rounded-xl bg-background p-8">
        <img src={appIcon} alt="" className="size-10" />
        <h1 className="mt-5 text-lg font-semibold">Auto Email Sender 未能启动</h1>
        <FailureNote className="mt-2 text-sm whitespace-pre-wrap text-muted-foreground" failure={failure.reason} />
        <div className="mt-6 flex items-center gap-3">
          <Button disabled={exported.isPending} onClick={() => exported.mutate()}>
            导出日志
          </Button>
          {exported.data && <span className="text-sm text-muted-foreground">已导出</span>}
          {exported.isError && <FailureNote className="text-sm text-destructive" failure={failureOf(exported.error)} />}
        </div>
        <p className="mt-4 text-sm">
          仍未解决的，可将日志和问题说明发到 {SUPPORT_ADDRESS}
          <Button variant="quiet" size="sm" onClick={() => copy.mutate()}>
            {copy.isSuccess ? "已复制" : "复制诊断信息"}
          </Button>
        </p>
        {copy.isError && <p className="text-sm text-destructive">未能复制，请导出日志</p>}
      </div>
    </main>
  );
}
