import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { exportMentors } from "@/api/app";
import { failureOf } from "@/api/call";
import { FailureNote } from "@/components/failure";
import type { MentorSort, Scope, TableFormat } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Dialog, DialogBody, DialogClose, DialogContent, DialogFooter, DialogHeader } from "@/components/ui/dialog";

/** The table lists the mentors in the order of the list, `sort`. */
export function ExportDialog({
  scope,
  sort,
  count,
  onClose,
}: {
  scope: Scope;
  sort?: MentorSort;
  count: number;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<TableFormat>("xlsx");
  const [note, setNote] = useState(true);
  const save = useMutation({
    mutationFn: () => exportMentors({ scope, sort, format, note }, "导师表格"),
    onSuccess: (saved) => {
      if (saved) onClose();
    },
  });
  return (
    <Dialog open onOpenChange={(open) => !open && !save.isPending && onClose()}>
      <DialogContent>
        <DialogHeader>导出表格</DialogHeader>
        <DialogBody className="space-y-5">
          <p>导出 {count.toLocaleString()} 位导师</p>
          <fieldset className="text-sm">
            <legend className="mb-2 text-muted-foreground">格式</legend>
            <RadioGroup value={format} onValueChange={setFormat} className="flex gap-5">
              {(["xlsx", "csv"] as const).map((f) => (
                <label key={f} className="flex items-center gap-2">
                  <RadioGroupItem value={f} />
                  {f.toUpperCase()}
                </label>
              ))}
            </RadioGroup>
          </fieldset>
          <div>
            <label className="flex items-center gap-2.5 text-sm">
              <Checkbox checked={note} onCheckedChange={setNote} />
              包含备注
            </label>
            <p className="mt-1.5 text-sm text-muted-foreground">备注是写给自己的，分享表格前请检查备注</p>
          </div>
        </DialogBody>
        <DialogFooter>
          <div className="min-w-0 grow text-sm text-destructive" role="alert">
            {save.error && <FailureNote failure={failureOf(save.error)} />}
          </div>
          <DialogClose render={<Button variant="ghost" disabled={save.isPending} />}>取消</DialogClose>
          <Button disabled={save.isPending} onClick={() => save.mutate()}>
            {save.isPending ? "正在导出…" : "导出"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
