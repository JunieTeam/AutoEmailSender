import { useQuery } from "@tanstack/react-query";
import { changelog } from "@/api/app";
import type { Change, ChangeKind, Release } from "@/api/types";
import { Dialog, DialogBody, DialogContent, DialogHeader } from "@/components/ui/dialog";
import { day } from "@/format";

const KINDS: [ChangeKind, string][] = [
  ["feature", "新功能"],
  ["improvement", "改进"],
  ["fix", "修复"],
];

/** An update's changes by kind; spanning several versions, each is marked with its own (docs/ui.md#交互). */
export function UpdateChanges({ releases }: { releases: Release[] }) {
  const [newest, oldest] = [releases[0], releases.at(-1)];
  if (!newest || !oldest) return <p>这个版本无更新说明</p>;
  const several = releases.length > 1;
  return (
    <div className="flex flex-col gap-4">
      {several && (
        <p className="text-muted-foreground">
          {releases.length} 个版本 · {oldest.version} – {newest.version}
        </p>
      )}
      {KINDS.map(([kind, label]) => {
        const changes = releases.flatMap((r) =>
          r.changes.filter((c) => c.kind === kind).map((c) => ({ text: c.text, version: r.version })),
        );
        return (
          changes.length > 0 && (
            <section key={kind} className="flex flex-col gap-1.5">
              <h3 className="font-medium">
                {label} <span className="font-normal text-muted-foreground tabular-nums">{changes.length}</span>
              </h3>
              <ul className="flex list-disc flex-col gap-1 pl-5 leading-relaxed">
                {changes.map((c, i) => (
                  <li key={i} className="wrap-anywhere">
                    {c.text}
                    {several && <span className="ml-2 text-muted-foreground tabular-nums">{c.version}</span>}
                  </li>
                ))}
              </ul>
            </section>
          )
        );
      })}
    </div>
  );
}

/** Each version up to the running one, newest first. */
export function ChangelogDialog({ onClose }: { onClose: () => void }) {
  const releases = useQuery({ queryKey: ["changelog"], queryFn: changelog, staleTime: Infinity });
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="w-[520px]">
        <DialogHeader>更新日志</DialogHeader>
        <DialogBody className="flex flex-col gap-6 pb-6 text-sm">
          {releases.data?.map((r) => (
            <section key={r.version} className="flex flex-col gap-3">
              <h3 className="text-base font-semibold">
                {r.version}
                {r.date && (
                  <span className="ml-2 text-sm font-normal text-muted-foreground">
                    {day(new Date(`${r.date}T00:00:00`).getTime())}
                  </span>
                )}
              </h3>
              <Kinds changes={r.changes} />
            </section>
          ))}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

function Kinds({ changes }: { changes: Change[] }) {
  return KINDS.map(([kind, label]) => {
    const of = changes.filter((c) => c.kind === kind);
    return (
      of.length > 0 && (
        <div key={kind} className="flex flex-col gap-1">
          <h4 className="text-muted-foreground">{label}</h4>
          <ul className="flex list-disc flex-col gap-1 pl-5 leading-relaxed">
            {of.map((c, i) => (
              <li key={i} className="wrap-anywhere">
                {c.text}
              </li>
            ))}
          </ul>
        </div>
      )
    );
  });
}
