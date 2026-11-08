import { useMutation, useMutationState, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSyncExternalStore } from "react";
import { exportLogs } from "@/api/app";
import { call, failureOf } from "@/api/call";
import { FEEDBACK_CHARS, type FeedbackInput, type FeedbackPreview } from "@/api/types";
import { useAccount } from "@/components/account";
import { FailureNote } from "@/components/failure";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader } from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { day, fileSize } from "@/format";
import { SUPPORT_ADDRESS } from "@/legal";

const SUBMIT = ["submitFeedback"];

// What the user wrote stays until it is sent or the app quits, whether the dialog stays open or not.
let kept: FeedbackInput = { description: "", withLog: true };
const listeners = new Set<() => void>();
function keep(input: FeedbackInput) {
  kept = input;
  for (const listener of listeners) listener();
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Sends a problem to the developer, or says how to by email (docs/ui.md#设置). */
export function FeedbackDialog({ onClose }: { onClose: () => void }) {
  const account = useAccount();
  const state = account.data;
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="w-[560px]">
        <DialogHeader>反馈问题</DialogHeader>
        {account.error && (
          <DialogBody className="pb-5 text-sm">
            <FailureNote className="text-destructive" lead="账号未能读取：" failure={failureOf(account.error)} />
          </DialogBody>
        )}
        {state?.kind === "loggedIn" && !state.termsOutdated ? (
          <Submit email={state.email} />
        ) : (
          state && (
            <DialogBody className="flex flex-col gap-3 pb-5 text-sm">
              <p>
                {state.kind === "loggedIn" ? "同意新版条款后可直接在此提交" : "登录后可直接在此提交"}
                ，也可导出日志，连同问题说明发到 {SUPPORT_ADDRESS}
              </p>
              <ByEmail />
            </DialogBody>
          )
        )}
      </DialogContent>
    </Dialog>
  );
}

function Submit({ email }: { email: string }) {
  const queryClient = useQueryClient();
  const input = useSyncExternalStore(subscribe, () => kept);
  // Taken afresh each time the dialog opens, unless the last try may not have got through.
  const preview = useQuery({
    queryKey: ["previewFeedback"],
    queryFn: () => call("previewFeedback", {}),
    staleTime: Infinity,
    refetchOnMount: "always",
  });
  const submit = useMutation({
    mutationKey: SUBMIT,
    mutationFn: (i: FeedbackInput) => call("submitFeedback", i),
    // Here rather than at the call: the dialog may be closed by the time it answers.
    onSuccess: () => {
      keep({ ...kept, description: "" });
      void queryClient.invalidateQueries({ queryKey: ["previewFeedback"] });
    },
  });
  // The dialog may have been closed and opened again since the try began.
  const last = useMutationState({ filters: { mutationKey: SUBMIT }, select: (m) => m.state }).at(-1);
  const pending = last?.status === "pending";
  return (
    <>
      <DialogBody className="flex flex-col gap-3 text-sm">
        <label className="flex flex-col gap-1.5">
          遇到了什么问题
          <Textarea
            rows={6}
            maxLength={FEEDBACK_CHARS}
            value={input.description}
            disabled={pending}
            onChange={(e) => keep({ ...input, description: e.target.value })}
          />
        </label>
        <div>
          <label className="flex items-center gap-2">
            <Checkbox
              checked={input.withLog}
              disabled={pending}
              onCheckedChange={(withLog) => keep({ ...input, withLog })}
            />
            附上日志
          </label>
          {preview.data && <LogLine preview={preview.data} />}
          {preview.error && <FailureNote className="mt-1 text-destructive" failure={failureOf(preview.error)} />}
        </div>
        <p className="text-muted-foreground">
          说明、版本和系统信息发给开发者，日志可选；日志里有出错时服务器和服务商的回复，偶尔可能引用邮件的主题或片段。服务器不保存，反馈以邮件形式保存在开发者的客服邮箱，处理完一年后删除。回复会发到你的账号邮箱{" "}
          {masked(email)}。
        </p>
        {last?.status === "success" && input.description === "" && (
          <p className="text-success">已发给开发者，回复会发到你的账号邮箱</p>
        )}
        {last?.status === "error" && (
          <div className="flex flex-col gap-2">
            <FailureNote className="text-destructive" failure={failureOf(last.error)} />
            <p>也可导出日志，连同问题说明发到 {SUPPORT_ADDRESS}</p>
            <ByEmail />
          </div>
        )}
      </DialogBody>
      <DialogFooter className="justify-end">
        <Button
          disabled={pending || !preview.data || input.description.trim() === ""}
          onClick={() => submit.mutate(input)}
        >
          {pending ? "正在提交…" : "提交"}
        </Button>
      </DialogFooter>
    </>
  );
}

/** What goes with the description, and the log to look through before it does. */
function LogLine({ preview }: { preview: FeedbackPreview }) {
  const { days } = preview;
  const span =
    days === null ? "" : day(days.first) === day(days.last) ? day(days.first) : `${day(days.first)}至${day(days.last)}`;
  return (
    <Collapsible className="pl-6">
      <p className="text-muted-foreground">
        软件版本、系统、{span}
        {span && "的"}运行日志（{fileSize(preview.size)}）
      </p>
      <CollapsibleTrigger>查看</CollapsibleTrigger>
      <CollapsibleContent>
        <LogText />
      </CollapsibleContent>
    </Collapsible>
  );
}

function LogText() {
  // Read when shown: it may be a couple of megabytes.
  const log = useQuery({ queryKey: ["getFeedbackLog"], queryFn: () => call("getFeedbackLog", {}), gcTime: 0 });
  if (log.error) return <FailureNote className="text-destructive" failure={failureOf(log.error)} />;
  return (
    <pre className="max-h-72 overflow-auto rounded-md bg-muted p-3 text-xs whitespace-pre-wrap wrap-anywhere">
      {log.data ?? "正在读取…"}
    </pre>
  );
}

/** The other way to reach the developer. */
function ByEmail() {
  const exported = useMutation({ mutationFn: exportLogs });
  const copy = useMutation({ mutationFn: () => navigator.clipboard.writeText(SUPPORT_ADDRESS) });
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="quiet" size="sm" disabled={exported.isPending} onClick={() => exported.mutate()}>
          导出日志
        </Button>
        <Button variant="quiet" size="sm" onClick={() => copy.mutate()}>
          {copy.isSuccess ? "已复制" : "复制邮箱地址"}
        </Button>
        {exported.data && <span className="text-muted-foreground">已导出</span>}
      </div>
      {exported.error && <FailureNote className="text-destructive" failure={failureOf(exported.error)} />}
      {copy.isError && <p className="text-destructive">未能复制，请手动输入邮箱地址</p>}
    </div>
  );
}

/** "a***@qq.com": enough to recognize, little in a screenshot. */
function masked(email: string) {
  return email.replace(/^(.)[^@]*/, "$1***");
}
