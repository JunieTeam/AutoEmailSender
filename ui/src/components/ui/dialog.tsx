import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { cn } from "cn";
import { XIcon } from "lucide-react";
import { createContext, use, useRef, useState } from "react";
import type * as React from "react";
import { useLayerStep } from "@/back";
import { Button } from "@/components/ui/button";

// docs/ui.md#尺寸 and #动效: a 20px-cornered card that grows in from 96%; on a phone it fills the screen
// and slides up from the bottom.

/** Set while asking whether to throw away what was typed, once going back would close the dialog. */
const Leaving = createContext<{ stay: () => void; leave: () => void } | null>(null);

/**
 * docs/ui.md#交互: an open dialog is a step in the history, and going back closes it as its close
 * button does; with `unsaved` input in it, it asks first.
 */
function Dialog({ unsaved = false, ...props }: DialogPrimitive.Root.Props & { unsaved?: boolean }) {
  const actions = useRef<DialogPrimitive.Root.Actions>(null);
  const [asking, setAsking] = useState(false);
  useLayerStep(props.open === true, () => (unsaved ? setAsking(true) : actions.current?.close()));
  return (
    <Leaving
      value={
        asking
          ? {
              stay: () => setAsking(false),
              leave: () => {
                setAsking(false);
                actions.current?.close();
              },
            }
          : null
      }
    >
      <DialogPrimitive.Root data-slot="dialog" actionsRef={actions} {...props} />
    </Leaving>
  );
}

function DialogContent({ className, children, ...props }: DialogPrimitive.Popup.Props) {
  const leaving = use(Leaving);
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Backdrop className="fixed inset-0 z-50 bg-[rgb(20_17_16/0.4)] transition-opacity duration-180 ease-out data-ending-style:opacity-0 data-ending-style:duration-120 data-ending-style:ease-in data-starting-style:opacity-0" />
      <div className="fixed inset-0 z-50 grid grid-cols-[minmax(0,1fr)] place-items-center p-6 phone:p-0">
        <DialogPrimitive.Popup
          data-slot="dialog-content"
          className={cn(
            "flex max-h-[calc(100dvh-3rem)] min-h-0 w-[660px] max-w-full min-w-0 flex-col rounded-xl bg-background shadow-pop transition-[opacity,scale,translate] duration-180 ease-out outline-none data-ending-style:scale-96 data-ending-style:opacity-0 data-ending-style:duration-120 data-ending-style:ease-in data-starting-style:scale-96 data-starting-style:opacity-0 phone:h-full phone:max-h-dvh phone:w-full phone:rounded-none phone:pt-(--title-bar) phone:data-ending-style:translate-y-full phone:data-ending-style:scale-100 phone:data-starting-style:translate-y-full phone:data-starting-style:scale-100",
            className,
          )}
          data-leaving={leaving ? "" : undefined}
          {...props}
        >
          {children}
          {leaving && (
            // In place of the footer's buttons, laid out as Confirm's.
            <div role="alert" className="flex flex-wrap items-center gap-2 px-6 pt-3.5 pb-5 phone:px-4">
              <p className="grow">放弃未保存的内容？</p>
              <Button variant="quiet" onClick={leaving.stay}>
                继续编辑
              </Button>
              <Button onClick={leaving.leave}>放弃</Button>
            </div>
          )}
        </DialogPrimitive.Popup>
      </div>
    </DialogPrimitive.Portal>
  );
}

/** `closable={false}` for a dialog that has to be answered with one of its buttons. */
function DialogHeader({ children, closable = true }: { children: React.ReactNode; closable?: boolean }) {
  return (
    <div className="flex items-center gap-3 pt-[18px] pr-[18px] pb-1.5 pl-6 phone:pl-4">
      <DialogPrimitive.Title className="grow text-xl font-semibold">{children}</DialogPrimitive.Title>
      {closable && (
        <DialogPrimitive.Close render={<Button variant="ghost" size="icon" aria-label="关闭" />}>
          <XIcon strokeWidth={1.6} />
        </DialogPrimitive.Close>
      )}
    </div>
  );
}

function DialogBody({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex-1 overflow-y-auto px-6 py-2 phone:px-4", className)} {...props} />;
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("flex items-center gap-2 px-6 pt-3.5 pb-5 in-data-leaving:hidden phone:px-4", className)}
      {...props}
    />
  );
}

const DialogClose = DialogPrimitive.Close;

export { Dialog, DialogBody, DialogClose, DialogContent, DialogFooter, DialogHeader };
