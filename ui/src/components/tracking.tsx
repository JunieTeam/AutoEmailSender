import type { EmailTracking } from "@/api/types";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { day, time } from "@/format";

/** Said wherever opens are explained (docs/design.md#邮件追踪). */
export const TRACKING_CAVEAT =
  "结果只作参考：有的邮箱默认不显示图片，打开了也记录不到；有的会自动加载图片（比如苹果“邮件”的“保护邮件活动”），未打开也会记为打开；你在自己邮箱的“已发送”里打开这封也会记一次；追踪图片还可能略微增加进垃圾箱的概率。";

/** How a sent letter's tracking stands (docs/ui.md#往来); nothing until the server has it. */
export function TrackingNote({ tracking: t }: { tracking: EmailTracking }) {
  if (t.state === "pending") return null;
  if (t.state !== "tracked") return <p className="text-sm text-muted-foreground">未追踪：{UNTRACKED[t.state]}</p>;
  // The server keeps the times of the latest opens only, and none of those it counted before it
  // began to keep them.
  const untimed = t.opens > t.openTimes.length;
  return (
    <Popover>
      <PopoverTrigger
        openOnHover
        className="text-sm text-muted-foreground underline decoration-dotted underline-offset-4"
      >
        {t.lastOpenAt === null ? "尚无打开记录" : `已打开 ${t.opens} 次，最近 ${at(t.lastOpenAt)}`}
      </PopoverTrigger>
      <PopoverContent side="top" className="w-72 space-y-1.5 p-3 text-muted-foreground">
        {t.openTimes.length > 0 && (
          <div className="text-foreground">
            <p>{untimed ? `共打开 ${t.opens} 次，这里是最近 ${t.openTimes.length} 次：` : "每次打开的时间："}</p>
            <ol className="max-h-40 overflow-y-auto tabular-nums">
              {t.openTimes.map((ms, i) => (
                <li key={i}>{at(ms)}</li>
              ))}
            </ol>
          </div>
        )}
        {untimed && t.firstOpenAt !== null && <p className="text-foreground">第一次打开：{at(t.firstOpenAt)}</p>}
        <p>{TRACKING_CAVEAT}</p>
      </PopoverContent>
    </Popover>
  );
}

const UNTRACKED = {
  no_quota: "额度不足",
  no_plus: "Plus 已到期",
  expired: "交出后 30 天内未能登记到服务器",
} satisfies Record<Exclude<EmailTracking["state"], "pending" | "tracked">, string>;

function at(ms: number): string {
  return `${day(ms)} ${time(ms)}`;
}
