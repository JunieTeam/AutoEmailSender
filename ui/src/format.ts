import type { Currency, ModelRate } from "@/api/types";

/** "今天", "昨天", or the date, in the user's time zone (docs/ui.md#文案). */
export function day(ms: number): string {
  const date = new Date(ms);
  const today = new Date();
  const days = Math.round((startOfDay(today) - startOfDay(date)) / 86_400_000);
  if (days === 0) return "今天";
  if (days === 1) return "昨天";
  const md = `${date.getMonth() + 1}月${date.getDate()}日`;
  return date.getFullYear() === today.getFullYear() ? md : `${date.getFullYear()}年${md}`;
}

/** "今天发出", "昨天发出" or "已发出 3 天", for a letter still waiting on a reply. */
export function sentFor(sentAt: number, now: number): string {
  const days = Math.max(0, Math.floor((now - sentAt) / 86_400_000));
  return days === 0 ? `${day(sentAt)}发出` : `已发出 ${days} 天`;
}

/**
 * Ends a message with "。" so that another sentence can follow it. Single-sentence messages carry no
 * full stop (docs/ui.md#文案) but longer ones end with one, so add it only when missing.
 */
export function sentence(text: string): string {
  return /[。？]$/.test(text) ? text : `${text}。`;
}

/** "320 KB", "1.2 MB". */
export function fileSize(bytes: number): string {
  const kb = bytes / 1024;
  return kb < 1024 ? `${Math.max(1, Math.round(kb))} KB` : `${(kb / 1024).toFixed(1)} MB`;
}

/** The units of prices and costs, which are never converted (docs/design.md#联系, 统计). */
export const UNITS: Record<Currency, string> = { cny: "元", usd: "美元" };

/**
 * "3.20 元" or "0.45 美元"; less than the last place shown is "不到 0.01 元". An amount `about` is
 * "约 3.20 元".
 */
export function money(amount: number, currency: Currency, { digits = 2, about = false } = {}): string {
  const least = 10 ** -digits;
  if (amount > 0 && amount < least) return `不到 ${least.toFixed(digits)} ${UNITS[currency]}`;
  return `${about ? "约 " : ""}${amount.toFixed(digits)} ${UNITS[currency]}`;
}

/** What tokens cost at a model's rate, in its currency. */
export function costAt(rate: ModelRate, input: number, output: number): number {
  return (rate.input * input + rate.output * output) / 1_000_000;
}

/** Which hours' prices an estimate took, for a provider that charges by the hour: "（按空闲时段价格）". */
export function hoursNote(rate: ModelRate): string {
  return rate.hours === null ? "" : `（按${rate.hours === "peak" ? "高峰" : "空闲"}时段价格）`;
}

/** What tokens are estimated to cost at a model's rate: "约 3.20 元（按空闲时段价格）". */
export function costEstimate(rate: ModelRate, input: number, output: number, digits = 2): string {
  return money(costAt(rate, input, output), rate.currency, { digits, about: true }) + hoursNote(rate);
}

/** "09:30", in the user's time zone. */
export function time(ms: number): string {
  const date = new Date(ms);
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}
