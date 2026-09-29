import { type KeyboardEvent, useMemo, useRef, useState } from "react";
import { useChartAnimation } from "./useChartAnimation";
import {
  WIDTH,
  getBarCenter,
  getBarWidth,
  getBarX,
  shouldRenderCenterMax,
} from "./chartGeometry";

/**
 * BarChart — dependency-free SVG bar chart with entrance animation,
 * hover tooltip, and a yellow peak cap. Mirrors the visuals from
 * `rust/src/native_ui/charts.rs` (ChartBar::draw).
 *
 * Phase 10 additions:
 *   - entrance animation with staggered ease-out (respects
 *     `animations` prop and `prefers-reduced-motion`)
 *   - absolute-positioned hover tooltip
 *   - peak cap rendered as a separate rect filled with `--chart-peak`
 */

export interface BarChartPoint {
  label: string;
  value: number | null;
}

/**
 * Controlled bar selection. When set, the bars form a roving-tabindex listbox:
 * hover or focus selects a bar, Left/Right move the selection, Home/End jump to
 * the first/last bar, and the hover tooltip is left to the caller's detail view.
 */
export interface BarChartSelection {
  index: number;
  onSelect: (index: number) => void;
}

export interface BarChartProps {
  data: BarChartPoint[];
  color?: string;
  height?: number;
  valueFormatter?: (n: number) => string;
  ariaLabel: string;
  /** When false, bars render at their final size immediately. */
  animations?: boolean;
  /** Optional empty-state message rendered when `data.length === 0`. */
  emptyMessage?: string;
  selection?: BarChartSelection;
}

const DEFAULT_COLOR = "var(--chart-cost)";
const CAP_HEIGHT = 5;

export function BarChart({
  data,
  color = DEFAULT_COLOR,
  height = 56,
  valueFormatter,
  ariaLabel,
  animations = true,
  emptyMessage,
  selection,
}: BarChartProps) {
  const fmt = valueFormatter ?? ((v: number) => v.toFixed(2));
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [hover, setHover] = useState<{ i: number; x: number; y: number } | null>(null);
  const barRefs = useRef<Array<SVGRectElement | null>>([]);

  const anim = useChartAnimation(data.length, animations, [
    data.length,
    data[0]?.label,
    data[data.length - 1]?.label,
  ]);

  const { max, peakIndex } = useMemo(() => {
    let m = 0.0001;
    let p = -1;
    for (let i = 0; i < data.length; i++) {
      const v = data[i].value;
      if (v != null && v > m) {
        m = v;
        p = i;
      }
    }
    return { max: m, peakIndex: p };
  }, [data]);

  if (data.length === 0) {
    return (
      <div className="chart chart--bar">
        <div className="chart__empty">{emptyMessage ?? ""}</div>
      </div>
    );
  }

  const barWidth = getBarWidth(data.length);
  const plotHeight = Math.max(1, height - 4);

  const onMove = (e: React.MouseEvent<SVGRectElement>, i: number) => {
    const host = containerRef.current;
    if (!host) return;
    const hostRect = host.getBoundingClientRect();
    setHover({ i, x: e.clientX - hostRect.left, y: e.clientY - hostRect.top });
  };
  const onLeave = () => setHover(null);

  const selectedIndex = selection
    ? Math.min(Math.max(selection.index, 0), data.length - 1)
    : -1;
  const onKeyDown = (e: KeyboardEvent<SVGSVGElement>) => {
    if (!selection) return;
    let next: number;
    if (e.key === "ArrowLeft") next = selectedIndex - 1;
    else if (e.key === "ArrowRight") next = selectedIndex + 1;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = data.length - 1;
    else return;
    e.preventDefault();
    next = Math.min(Math.max(next, 0), data.length - 1);
    selection.onSelect(next);
    barRefs.current[next]?.focus();
  };

  return (
    <div className="chart chart--bar" ref={containerRef}>
      <svg
        width={WIDTH}
        height={height}
        viewBox={`0 0 ${WIDTH} ${height}`}
        className="chart__svg"
        role={selection ? "listbox" : "img"}
        aria-orientation={selection ? "horizontal" : undefined}
        aria-label={ariaLabel}
        onKeyDown={selection ? onKeyDown : undefined}
      >
        {data.map((p, i) => {
          const base = p.value == null ? 1 : p.value === 0 ? 1 : Math.max(3, (p.value / max) * plotHeight);
          const eased = anim.barProgress(i);
          const barH = base * eased;
          const x = getBarX(i, data.length);
          const y = height - barH;
          const isPeak = i === peakIndex && barH > CAP_HEIGHT;
          const bodyH = isPeak ? Math.max(0, barH - CAP_HEIGHT) : barH;
          const bodyY = isPeak ? y + CAP_HEIGHT : y;
          const isHovered = hover?.i === i;
          const isSelected = i === selectedIndex;

          return (
            <g key={`${p.label}-${i}`}>
              <rect
                x={x}
                y={bodyY}
                width={barWidth}
                height={bodyH}
                fill={color}
                opacity={
                  p.value == null
                    ? 0
                    : p.value === 0
                      ? 0.25
                      : selection
                        ? isSelected ? 1 : 0.6
                        : isHovered ? 1 : 0.9
                }
                rx={1}
                className="chart__bar"
                {...(selection
                  ? {
                      ref: (node: SVGRectElement | null) => {
                        barRefs.current[i] = node;
                      },
                      role: "option",
                      "aria-selected": isSelected,
                      "aria-label": p.value == null ? p.label : `${p.label}: ${fmt(p.value)}`,
                      tabIndex: isSelected ? 0 : -1,
                      "data-selected": isSelected ? "true" : "false",
                      onMouseEnter: () => selection.onSelect(i),
                      onFocus: () => selection.onSelect(i),
                    }
                  : {
                      onMouseMove:
                        p.value == null
                          ? undefined
                          : (e: React.MouseEvent<SVGRectElement>) => onMove(e, i),
                      onMouseLeave: onLeave,
                    })}
              >
                <title>
                  {p.value == null ? p.label : `${p.label}: ${fmt(p.value)}`}
                </title>
              </rect>
              {isPeak && (
                <rect
                  x={x}
                  y={y}
                  width={barWidth}
                  height={CAP_HEIGHT}
                  fill="var(--chart-peak)"
                  rx={1}
                  className="chart__peak-cap"
                  pointerEvents="none"
                />
              )}
            </g>
          );
        })}
      </svg>
      <div className="chart__axis">
        <span className="chart__axis-start" style={{ left: `${getBarCenter(0, data.length)}px` }}>
          {data[0].label}
        </span>
        {shouldRenderCenterMax(data.length) && (
          <span className="chart__axis-max" style={{ left: `${WIDTH / 2}px` }}>{fmt(max)}</span>
        )}
        <span className="chart__axis-end" style={{ left: `${getBarCenter(data.length - 1, data.length)}px` }}>
          {data[data.length - 1].label}
        </span>
      </div>
      {!selection && hover && !anim.running && (
        <div
          className="chart__tooltip"
          style={{ left: hover.x, top: hover.y }}
          role="tooltip"
        >
          <span className="chart__tooltip-label">{data[hover.i].label}</span>
          <strong>{fmt(data[hover.i].value ?? 0)}</strong>
        </div>
      )}
    </div>
  );
}
