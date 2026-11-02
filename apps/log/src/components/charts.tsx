// Dependency-free SVG charts sized to their container width.
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { formatCompact, formatNumber } from "../lib/format";

export interface Series {
  name: string;
  color?: string;
  /** x: ISO date (YYYY-MM-DD) or label; y: value. All series share the same x order. */
  points: { x: string; y: number | null }[];
}

const PALETTE = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)", "var(--series-6)"];

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(640);
  useLayoutEffect(() => {
    if (!ref.current) return;
    const el = ref.current;
    const ro = new ResizeObserver(() => setWidth(Math.max(240, el.clientWidth)));
    ro.observe(el);
    setWidth(Math.max(240, el.clientWidth));
    return () => ro.disconnect();
  }, []);
  return { ref, width };
}

/** "Nice" upper bound for an axis. */
export function niceMax(value: number): number {
  if (value <= 0) return 1;
  const exp = Math.pow(10, Math.floor(Math.log10(value)));
  const f = value / exp;
  const nice = f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10;
  return nice * exp;
}

export function LineChart({ series, height = 220, yFormat, invertY = false, area = true }: { series: Series[]; height?: number; yFormat?: (v: number) => string; invertY?: boolean; area?: boolean }) {
  const { locale } = useI18n();
  const { ref, width } = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const fmt = yFormat ?? ((v: number) => formatCompact(v, locale));
  const pad = { top: 12, right: 14, bottom: 26, left: 48 };
  const xs = series[0]?.points.map((p) => p.x) ?? [];
  const n = xs.length;

  const { min, max } = useMemo(() => {
    const values = series.flatMap((s) => s.points.map((p) => p.y)).filter((v): v is number => v !== null && Number.isFinite(v));
    if (!values.length) return { min: 0, max: 1 };
    const lo = invertY ? Math.min(...values) : 0;
    return { min: invertY ? Math.max(0, Math.floor(lo)) : 0, max: invertY ? Math.ceil(Math.max(...values)) : niceMax(Math.max(...values)) };
  }, [series, invertY]);

  const innerW = width - pad.left - pad.right;
  const innerH = height - pad.top - pad.bottom;
  const x = (i: number) => pad.left + (n <= 1 ? innerW / 2 : (i / (n - 1)) * innerW);
  const y = (v: number) => {
    const t = max === min ? 0.5 : (v - min) / (max - min);
    return pad.top + (invertY ? t : 1 - t) * innerH;
  };
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((t) => min + t * (max - min));
  const labelEvery = Math.max(1, Math.ceil(n / Math.max(2, Math.floor(innerW / 80))));

  if (!n) return <div ref={ref} className="muted">—</div>;
  return (
    <div ref={ref} style={{ position: "relative" }}>
      <svg className="chart" width={width} height={height} onMouseLeave={() => setHover(null)}
        onMouseMove={(e) => {
          const rect = (e.currentTarget as SVGSVGElement).getBoundingClientRect();
          const rel = (e.clientX - rect.left - pad.left) / Math.max(1, innerW);
          setHover(Math.min(n - 1, Math.max(0, Math.round(rel * (n - 1)))));
        }}>
        {ticks.map((tv, i) => (
          <g key={i}>
            <line className="grid-line" x1={pad.left} x2={width - pad.right} y1={y(tv)} y2={y(tv)} />
            <text x={pad.left - 8} y={y(tv) + 4} textAnchor="end">{fmt(tv)}</text>
          </g>
        ))}
        {xs.map((label, i) => (i % labelEvery === 0 || i === n - 1) && (
          <text key={i} x={x(i)} y={height - 8} textAnchor="middle">{label.length === 10 ? label.slice(5) : label}</text>
        ))}
        {series.map((s, si) => {
          const color = s.color ?? PALETTE[si % PALETTE.length];
          const pts = s.points.map((p, i) => (p.y === null ? null : [x(i), y(p.y)] as const));
          const segments: string[] = [];
          let current = "";
          pts.forEach((p) => {
            if (!p) { if (current) segments.push(current); current = ""; return; }
            current += `${current ? "L" : "M"}${p[0].toFixed(1)},${p[1].toFixed(1)}`;
          });
          if (current) segments.push(current);
          const first = pts.findIndex(Boolean);
          const lastIdx = pts.length - 1 - [...pts].reverse().findIndex(Boolean);
          return (
            <g key={s.name}>
              {area && series.length === 1 && segments.length === 1 && first >= 0 && (
                <path d={`${segments[0]}L${x(lastIdx)},${pad.top + innerH}L${x(first)},${pad.top + innerH}Z`} fill={color} opacity={0.12} />
              )}
              {segments.map((d, k) => <path key={k} d={d} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" />)}
              {hover !== null && pts[hover] && <circle cx={pts[hover]![0]} cy={pts[hover]![1]} r={3.5} fill={color} />}
            </g>
          );
        })}
        {hover !== null && <line x1={x(hover)} x2={x(hover)} y1={pad.top} y2={pad.top + innerH} stroke="var(--border-hover)" strokeDasharray="3 3" />}
      </svg>
      {hover !== null && (
        <div className="chart-tooltip" style={{ left: Math.min(width - 160, x(hover) + 10), top: 4 }}>
          <div className="muted">{xs[hover]}</div>
          {series.map((s, si) => (
            <div key={s.name}>
              <span style={{ color: s.color ?? PALETTE[si % PALETTE.length] }}>●</span> {s.name}: <b>{s.points[hover]?.y === null || s.points[hover] === undefined ? "—" : formatNumber(s.points[hover]!.y!, locale, 2)}</b>
            </div>
          ))}
        </div>
      )}
      {series.length > 1 && (
        <div className="chart-legend">
          {series.map((s, si) => (
            <span key={s.name}><i style={{ background: s.color ?? PALETTE[si % PALETTE.length] }} />{s.name}</span>
          ))}
        </div>
      )}
    </div>
  );
}

export function BarList({ items, max, format, onSelect }: { items: { label: string; value: number; hint?: string }[]; max?: number; format?: (v: number) => string; onSelect?: (label: string) => void }) {
  const { locale } = useI18n();
  const top = max ?? Math.max(1, ...items.map((i) => i.value));
  return (
    <div className="bar-list">
      {items.map((item) => (
        <div key={item.label} className="bar-list__row" title={item.hint ?? item.label} style={onSelect ? { cursor: "pointer" } : undefined} onClick={onSelect ? () => onSelect(item.label) : undefined}>
          <div className="bar-list__fill" style={{ width: `${Math.max(1, (item.value / top) * 100)}%` }} />
          <span className="bar-list__label">{item.label}</span>
          <span className="bar-list__value">{format ? format(item.value) : formatNumber(item.value, locale)}</span>
        </div>
      ))}
    </div>
  );
}

/** Stacked horizontal share bar (e.g. status code mix). */
export function ShareBar({ parts }: { parts: { label: string; value: number; color: string }[] }) {
  const { locale } = useI18n();
  const total = parts.reduce((s, p) => s + p.value, 0) || 1;
  return (
    <div className="field" style={{ gap: 8 }}>
      <div style={{ display: "flex", height: 12, borderRadius: 999, overflow: "hidden", background: "var(--bg-hover)" }}>
        {parts.filter((p) => p.value > 0).map((p) => (
          <div key={p.label} title={`${p.label}: ${formatNumber(p.value, locale)}`} style={{ width: `${(p.value / total) * 100}%`, background: p.color }} />
        ))}
      </div>
      <div className="chart-legend">
        {parts.map((p) => (
          <span key={p.label}><i style={{ background: p.color }} />{p.label} · {formatNumber(p.value, locale)}</span>
        ))}
      </div>
    </div>
  );
}
