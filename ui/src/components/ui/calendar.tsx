import type { ComponentProps } from "react";
import { useQuery } from "@tanstack/react-query";
import { DayButton, DayPicker, type ChevronProps, type DayButtonProps } from "react-day-picker";
import { zhCN } from "react-day-picker/locale";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { call, failureOf } from "@/api/call";
import type { Holidays } from "@/api/types";
import { FailureNote } from "@/components/failure";
import { Button } from "@/components/ui/button";
import { cn } from "cn";

/** A `YYYY-MM-DD` day as the calendar has its days: from midnight in the user's time. */
function day(date: string) {
  return new Date(`${date}T00:00`);
}

/** The days China takes off and the weekend days worked for them, as calendar days. */
function holidayDays({ daysOff, makeUpDays }: Holidays) {
  return {
    dayOff: daysOff.map(([from, until]) => ({ from: day(from), to: day(until) })),
    makeUp: makeUpDays.map(day),
  };
}

/** shadcn calendar, with only the day selections used by sending, marking China's holidays. */
export function Calendar({ className, ...props }: ComponentProps<typeof DayPicker>) {
  // The table is the app's own: it never changes while the app runs.
  const holidays = useQuery({
    queryKey: ["listHolidays"],
    queryFn: () => call("listHolidays", {}),
    staleTime: Infinity,
    select: holidayDays,
  });
  return (
    <DayPicker
      locale={zhCN}
      showOutsideDays
      className={cn("relative w-fit max-w-full rounded-lg bg-muted p-2 text-sm", className)}
      classNames={{
        months: "relative",
        month_caption: "flex h-10 items-center justify-center font-medium",
        nav: "absolute inset-x-0 top-0 flex justify-between",
        button_previous: "grid size-10 place-items-center rounded-md hover:bg-press disabled:opacity-40",
        button_next: "grid size-10 place-items-center rounded-md hover:bg-press disabled:opacity-40",
        month_grid: "w-full border-collapse",
        weekday: "h-8 text-xs font-normal text-muted-foreground",
        day: "p-0 text-center",
        day_button: "relative size-10 rounded-md hover:bg-press focus-visible:outline-2 focus-visible:outline-ring",
        selected: "[&>button]:bg-primary [&>button]:text-primary-foreground [&>button]:hover:bg-primary-hover",
        today: "font-semibold",
        outside: "text-faint",
        disabled: "text-faint opacity-40",
        hidden: "invisible",
        // As wide as the days, however long the words.
        footer: "w-0 min-w-full text-destructive",
      }}
      modifiers={holidays.data}
      // Always there, if empty: a screen reader announces what appears in a live region it already
      // knows, not always one that appears with its words. A fragment, as the calendar leaves out a
      // footer that is false.
      footer={
        <>
          {/* Without it, a day off reads the same as any other: say so rather than leave it unmarked. */}
          {holidays.isLoadingError && (
            <FailureNote className="px-1 pt-2" lead="节假日标记未能载入：" failure={failureOf(holidays.error)}>
              <Button variant="ghost" size="sm" className="ml-2" onClick={() => void holidays.refetch()}>
                重试
              </Button>
            </FailureNote>
          )}
        </>
      }
      components={{
        Chevron: CalendarChevron,
        DayButton: HolidayDayButton,
      }}
      {...props}
    />
  );
}

function CalendarChevron({ orientation }: ChevronProps) {
  return orientation === "left" ? <ChevronLeft className="size-4" /> : <ChevronRight className="size-4" />;
}

const MARKS = {
  dayOff: { mark: "休", label: "休息日", color: "text-warning" },
  makeUp: { mark: "班", label: "调休上班", color: "text-success" },
};

/** A day, with 休 in its corner when China takes it off and 班 when a weekend day is worked. */
function HolidayDayButton({ children, ...props }: DayButtonProps) {
  const { modifiers } = props;
  const holiday = modifiers.dayOff ? MARKS.dayOff : modifiers.makeUp ? MARKS.makeUp : null;
  return (
    <DayButton {...props} aria-label={holiday ? `${props["aria-label"]}，${holiday.label}` : props["aria-label"]}>
      {children}
      {holiday && (
        <span
          aria-hidden
          className={cn(
            "absolute top-0.5 right-0.5 text-xs leading-none font-normal",
            // Outside the month and when chosen, it takes the day's own color.
            !modifiers.selected && !modifiers.outside && holiday.color,
          )}
        >
          {holiday.mark}
        </span>
      )}
    </DayButton>
  );
}
