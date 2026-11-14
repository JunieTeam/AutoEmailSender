import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { call, failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type { Pack, Quota as TrackingQuota } from "@/api/types";
import { TermsNotice, useAccount, useTermsOutdated } from "@/components/account";
import { PayDialog, REFUNDED, usePlusItems, yuan } from "@/components/plus";
import { TRACKING_CAVEAT } from "@/components/tracking";
import { buttonVariants, Button } from "@/components/ui/button";
import { day, sentence } from "@/format";
import { useNow } from "@/now";
import { Help } from "@/components/ui/help";
import { Section, SwitchButton, SwitchRow } from "./settings";

/**
 * Settings › 邮件 › 邮件追踪 (docs/ui.md#交互). The quota lives on the server and packs are paid for
 * in the browser, so coming back reads it again.
 */
export function TrackingSection() {
  const account = useAccount();
  const email = account.data?.kind === "loggedIn" ? account.data.email : null;
  const queryClient = useQueryClient();
  const status = useQuery({ queryKey: ["getTracking", email], queryFn: () => call("getTracking", {}) });
  const save = useMutation({
    mutationFn: (enabled: boolean) => call("setTracking", { enabled }),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["getTracking"] }),
  });
  const now = useNow();
  const { refetch } = status;
  useEffect(() => {
    const refresh = () => void refetch({ cancelRefetch: false });
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refetch]);
  const outdated = useTermsOutdated();
  const data = status.data;
  const server = data?.account?.kind === "read" ? data.account : null;
  const plusUntil = server?.plusUntil != null && server.plusUntil > now ? server.plusUntil : null;
  const plus = plusUntil !== null;
  const enabled = data?.enabled ?? false;
  const quota = server?.tracking;
  return (
    <Section title="邮件追踪">
      <SwitchRow
        label="邮件追踪"
        summary="在首封和追问里放一张看不见的小图片，看导师是否打开了邮件"
        help={
          <>
            <p>
              导师打开邮件、加载图片时记一次，往来里能看到打开了几次、每次是什么时候。回复导师的邮件不放这张图片：导师已回过信，无需再追踪。
            </p>
            <p>
              官方服务器能看到每封邮件的随机编号、打开时间、打开者的 IP 和设备信息，只保存编号、打开次数、第一次和最近
              50 次打开的时间，看不到收件人和邮件内容。
            </p>
            <p>{TRACKING_CAVEAT}</p>
          </>
        }
        control={
          <SwitchButton
            label="邮件追踪"
            checked={enabled}
            disabled={!data || save.isPending || (!enabled && (!plus || outdated))}
            onClick={() => save.mutate(!enabled)}
          />
        }
      >
        {status.error && (
          <FailureNote className="text-sm text-destructive" lead="设置未能加载：" failure={failureOf(status.error)} />
        )}
        {save.error && <FailureNote className="text-sm text-destructive" failure={failureOf(save.error)} />}
      </SwitchRow>
      {data && !data.account && <Prompt text="登录并开通 Plus 后可使用" action="去登录" />}
      {data?.account?.kind === "unreadable" && (
        <p className="text-sm text-destructive">额度未能读取：{data.account.reason}</p>
      )}
      {server && outdated && <TermsNotice />}
      {server && !outdated && !plus && (
        <div className="flex flex-col gap-1.5">
          <Prompt text="开通 Plus 后可使用邮件追踪，每 30 天 500 封" action="开通 Plus" />
          {server.tracking.packLeft > 0 && (
            <p className="text-sm text-muted-foreground">
              额度包还剩 {server.tracking.packLeft} 封，Plus 有效时可继续使用
            </p>
          )}
        </div>
      )}
      {server && !outdated && plusUntil !== null && email && (
        <Quota key={email} email={email} plusUntil={plusUntil} quota={server.tracking} />
      )}
      {enabled && quota && (
        <>
          {!plus && <p className="text-sm text-warning">Plus 已到期，之后发出的邮件不带追踪图片，已追踪的照常更新</p>}
          {plus && quota.used >= quota.limit && quota.packLeft === 0 && (
            <p className="text-sm text-warning">
              最近 30 天的 {quota.limit} 封和额度包均已用完，之后发出的邮件不带追踪图片
            </p>
          )}
        </>
      )}
      {data?.failure && (
        <p className="text-sm text-destructive">同步打开记录失败：{sentence(data.failure)}之后会自动重试。</p>
      )}
    </Section>
  );
}

function Prompt({ text, action }: { text: string; action: string }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <p className="text-sm text-muted-foreground">{text}</p>
      <Link to="/settings/account" className={buttonVariants({ variant: "quiet", size: "sm" })}>
        {action}
      </Link>
    </div>
  );
}

function Quota({ email, plusUntil, quota }: { email: string; plusUntil: number; quota: TrackingQuota }) {
  const items = usePlusItems(email);
  const [buying, setBuying] = useState<Pack | null>(null);
  return (
    <div className="flex flex-col gap-1.5">
      <p className="flex items-center gap-1 font-medium">
        最近 30 天已用 {quota.used} / {quota.limit} 封
        <Help label="额度">
          <p>一封邮件算 1 封，打开多少次都不再计。</p>
          <p>每 30 天 {quota.limit} 封，按最近 30 天计算，用完后使用额度包，先用快到期的。</p>
          <p>额度包自开通起 365 天内有效，Plus 有效时方可使用，Plus 到期期间照常计时。</p>
        </Help>
      </p>
      {quota.packLeft > 0 && quota.packExpiresAt != null && (
        <p className="text-sm">
          额度包剩 {quota.packLeft} 封，最早{day(quota.packExpiresAt)}到期
        </p>
      )}
      {items.error ? (
        <FailureNote className="text-sm text-destructive" lead="额度包未能读取：" failure={failureOf(items.error)} />
      ) : (
        items.data &&
        (items.data.refunded ? (
          <p className="text-sm text-muted-foreground">{REFUNDED}</p>
        ) : (
          <div className="grid max-w-md grid-cols-3 gap-2">
            {items.data.packs.map((pack) => (
              <Button
                key={pack.item}
                variant="quiet"
                className="h-auto flex-col items-start gap-0.5 px-3 py-2.5 text-left font-normal whitespace-normal"
                onClick={() => setBuying(pack)}
              >
                <span className="text-sm">{pack.count} 封</span>
                <span className="text-lg font-semibold">{yuan(pack.price)}</span>
              </Button>
            ))}
          </div>
        ))
      )}
      {buying && <PayDialog email={email} purchase={{ pack: buying, plusUntil }} onClose={() => setBuying(null)} />}
    </div>
  );
}
