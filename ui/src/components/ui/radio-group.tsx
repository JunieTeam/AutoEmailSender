import { Radio } from "@base-ui/react/radio";
import { RadioGroup as RadioGroupPrimitive } from "@base-ui/react/radio-group";
import { cn } from "cn";

// docs/ui.md#尺寸: like the checkbox, 16px outlined in --input and filled with the brand colour when
// chosen, but round and with a dot.

const RadioGroup = RadioGroupPrimitive;

function RadioGroupItem({ className, ...props }: Radio.Root.Props) {
  return (
    <Radio.Root
      data-slot="radio"
      className={cn(
        "grid size-4 shrink-0 place-items-center rounded-full border-[1.5px] border-input bg-background data-checked:border-primary data-checked:bg-primary data-disabled:opacity-40",
        className,
      )}
      {...props}
    >
      <Radio.Indicator className="size-1.5 rounded-full bg-primary-foreground" />
    </Radio.Root>
  );
}

export { RadioGroup, RadioGroupItem };
