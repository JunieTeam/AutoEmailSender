import { useId } from "react";
import { Area, AreaChart, CartesianGrid, Tooltip, type TooltipContentProps, XAxis, YAxis } from "recharts";

export type Series = { name: string; unit: string; color: string };
/** A point in time: its axis label, its full name for the tooltip, one value per series. */
export type Point = { label: string; title: string; values: readonly number[] };

const AXIS_TEXT = { fill: "var(--muted-foreground)", fontSize: 11 };

/**
 * Series over time as smooth lines over a fading wash (docs/ui.md#图表). A thin line follows the
 * pointer or keyboard focus, with a dot on each series and the numbers gliding along beside it.
 */
export function TrendChart({
  series,
  points,
  caption,
}: {
  series: readonly Series[];
  points: Point[];
  caption: string;
}) {
  // Gradient ids are document-wide; React's own ids hold characters `url(#…)` does not take.
  const fill = `fill-${useId().replaceAll(/\W/g, "")}`;
  return (
    <AreaChart
      data={points}
      responsive
      style={{ width: "100%", height: 200 }}
      margin={{ top: 8, right: 4, bottom: 0, left: 0 }}
      title={caption}
    >
      <defs>
        {series.map((s, i) => (
          <linearGradient key={s.name} id={`${fill}-${i}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={s.color} stopOpacity={0.2} />
            <stop offset="100%" stopColor={s.color} stopOpacity={0} />
          </linearGradient>
        ))}
      </defs>
      <CartesianGrid vertical={false} stroke="var(--border)" />
      <XAxis
        dataKey="label"
        tick={AXIS_TEXT}
        tickLine={false}
        axisLine={{ stroke: "var(--border)" }}
        tickMargin={8}
        minTickGap={20}
        interval="preserveStartEnd"
      />
      <YAxis allowDecimals={false} tick={AXIS_TEXT} tickLine={false} axisLine={false} width="auto" tickCount={4} />
      <Tooltip
        cursor={{ stroke: "var(--faint)", strokeWidth: 1 }}
        animationDuration={200}
        animationEasing="ease-out"
        content={<Tip series={series} points={points} />}
      />
      {series.map((s, i) => (
        <Area
          key={s.name}
          name={s.name}
          type="monotone"
          dataKey={(p: Point) => p.values[i]}
          stroke={s.color}
          strokeWidth={2}
          fill={`url(#${fill}-${i})`}
          // A single point, such as the first month under 全部, has no line or area to show it.
          dot={points.length === 1 && { r: 3, fill: s.color, strokeWidth: 0 }}
          activeDot={{ r: 4, fill: s.color, stroke: "var(--background)", strokeWidth: 2 }}
        />
      ))}
    </AreaChart>
  );
}

/** Given `active` and `activeIndex` by Recharts, which clones it into the tooltip. */
function Tip({
  active,
  activeIndex,
  series,
  points,
}: Partial<Pick<TooltipContentProps, "active" | "activeIndex">> & { series: readonly Series[]; points: Point[] }) {
  const point = activeIndex == null ? undefined : points[Number(activeIndex)];
  if (!active || !point) return null;
  return (
    // Read out as the arrow keys move along the chart, as Recharts' own tooltip is.
    <div
      role="status"
      aria-live="polite"
      className="rounded-lg bg-background px-3 py-2 text-xs whitespace-nowrap shadow-[0_4px_16px_var(--pop-shadow)] ring-1 ring-border"
    >
      <p className="mb-1 font-medium">{point.title}</p>
      {series.map((s, i) => (
        <p key={s.name} className="flex items-center gap-1.5 text-muted-foreground">
          <span className="size-2 rounded-full" style={{ background: s.color }} />
          {s.name} <span className="ml-auto pl-3 text-foreground tabular-nums">{point.values[i]}</span> {s.unit}
        </p>
      ))}
    </div>
  );
}

export function Legend({ series }: { series: readonly Series[] }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      {series.map((s) => (
        <span key={s.name} className="flex items-center gap-1.5">
          <span className="size-2 rounded-full" style={{ background: s.color }} />
          {s.name}
        </span>
      ))}
    </div>
  );
}
