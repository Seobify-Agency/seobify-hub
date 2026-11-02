// Shared UI primitives for Seobify Hub tools.
import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useI18n } from "../i18n";
import { formatNumber, formatPercent } from "../lib/format";

export function Page({ title, subtitle, actions, children }: { title: string; subtitle?: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <div className="page">
      <div className="page__head">
        <div>
          <h1 className="page__title">{title}</h1>
          {subtitle && <p className="page__subtitle">{subtitle}</p>}
        </div>
        {actions && <div className="page__actions">{actions}</div>}
      </div>
      {children}
    </div>
  );
}

export function Card({ title, hint, actions, children, className = "" }: { title?: ReactNode; hint?: ReactNode; actions?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <section className={`card ${className}`}>
      {(title || actions) && (
        <div className="card__head">
          <div>
            {title && <h2 className="card__title">{title}</h2>}
            {hint && <p className="card__hint">{hint}</p>}
          </div>
          {actions && <div className="row">{actions}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

export function Stat({ label, value, delta, invertDelta, sub }: { label: string; value: ReactNode; delta?: number | null; invertDelta?: boolean; sub?: ReactNode }) {
  const { locale } = useI18n();
  const good = delta !== null && delta !== undefined && (invertDelta ? delta < 0 : delta > 0);
  return (
    <div className="stat">
      <span className="stat__label">{label}</span>
      <span className="stat__value">{value}</span>
      {delta !== null && delta !== undefined && (
        <span className={`stat__delta ${good ? "stat__delta--up" : delta === 0 ? "" : "stat__delta--down"}`}>
          {delta > 0 ? "▲" : delta < 0 ? "▼" : "•"} {formatPercent(Math.abs(delta), locale)}
        </span>
      )}
      {sub && <span className="stat__sub">{sub}</span>}
    </div>
  );
}

export type Tone = "gold" | "success" | "warning" | "error" | "critical" | "high" | "medium" | "low" | "info";

export function Badge({ tone, children }: { tone?: Tone; children: ReactNode }) {
  return <span className={`badge ${tone ? `badge--${tone}` : ""}`}>{children}</span>;
}

export function Empty({ icon = "✦", title, text, action }: { icon?: string; title: string; text?: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty__icon" aria-hidden>
        {icon}
      </div>
      <div className="empty__title">{title}</div>
      {text && <div className="empty__text">{text}</div>}
      {action}
    </div>
  );
}

export function ErrorAlert({ error }: { error: string | null | undefined }) {
  if (!error) return null;
  return <div className="alert alert--error">{error}</div>;
}

export function Loading({ label }: { label?: string }) {
  const { t } = useI18n();
  return (
    <div className="row muted">
      <span className="spinner" /> {label ?? t("common.loading")}
    </div>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="field">
      <span className="field__label">{label}</span>
      {children}
      {hint && <span className="field__hint">{hint}</span>}
    </label>
  );
}

export function Segment<T extends string>({ value, options, onChange }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="segment" role="group">
      {options.map((o) => (
        <button key={o.value} type="button" className={`segment__btn ${value === o.value ? "segment__btn--active" : ""}`} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Modal({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal style={wide ? { width: "min(1040px, 100%)" } : undefined}>
        <div className="row">
          <h2 className="modal__title">{title}</h2>
          <span className="spacer" />
          <button type="button" className="btn btn--ghost btn--sm" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {children}
        {footer && <div className="modal__foot">{footer}</div>}
      </div>
    </div>
  );
}

export interface Column<T> {
  key: string;
  label: string;
  /** Cell renderer; defaults to `String(row[key])`. */
  render?: (row: T) => ReactNode;
  /** Sort/search value; defaults to `row[key]`. */
  value?: (row: T) => string | number | null | undefined;
  numeric?: boolean;
  className?: string;
  sortable?: boolean;
}

/** Sortable, searchable table with a row cap (keeps huge result sets responsive). */
export function DataTable<T>({
  rows,
  columns,
  initialSort,
  searchable = true,
  limit = 500,
  onRowClick,
  rowKey,
  emptyText,
  toolbar,
}: {
  rows: T[];
  columns: Column<T>[];
  initialSort?: { key: string; desc?: boolean };
  searchable?: boolean;
  limit?: number;
  onRowClick?: (row: T) => void;
  rowKey?: (row: T, index: number) => string | number;
  emptyText?: string;
  toolbar?: ReactNode;
}) {
  const { t, locale } = useI18n();
  const [sort, setSort] = useState(initialSort ?? null);
  const [query, setQuery] = useState("");
  const valueOf = (col: Column<T>, row: T) => (col.value ? col.value(row) : ((row as Record<string, unknown>)[col.key] as string | number | null | undefined));

  const filtered = useMemo(() => {
    const q = query.trim().toLocaleLowerCase(locale);
    let list = q
      ? rows.filter((row) => columns.some((c) => String(valueOf(c, row) ?? "").toLocaleLowerCase(locale).includes(q)))
      : rows.slice();
    if (sort) {
      const col = columns.find((c) => c.key === sort.key);
      if (col) {
        list = list.sort((a, b) => {
          const va = valueOf(col, a);
          const vb = valueOf(col, b);
          if (va === vb) return 0;
          if (va === null || va === undefined) return 1;
          if (vb === null || vb === undefined) return -1;
          const cmp = typeof va === "number" && typeof vb === "number" ? va - vb : String(va).localeCompare(String(vb), locale);
          return sort.desc ? -cmp : cmp;
        });
      }
    }
    return list;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, columns, sort, query, locale]);

  const shown = filtered.slice(0, limit);
  return (
    <div className="field" style={{ gap: 8 }}>
      {(searchable || toolbar) && (
        <div className="row">
          {searchable && <input className="input" style={{ maxWidth: 320 }} placeholder={t("common.search")} value={query} onChange={(e) => setQuery(e.target.value)} />}
          {toolbar}
          <span className="spacer" />
          <span className="muted" style={{ fontSize: 12 }}>
            {filtered.length > shown.length ? t("common.showing", { shown: formatNumber(shown.length, locale), total: formatNumber(filtered.length, locale) }) : t("common.rows", { count: formatNumber(filtered.length, locale) })}
          </span>
        </div>
      )}
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              {columns.map((c) => {
                const sortable = c.sortable !== false;
                const active = sort?.key === c.key;
                return (
                  <th
                    key={c.key}
                    className={`${c.numeric ? "num" : ""} ${sortable ? "sortable" : ""}`}
                    onClick={() => sortable && setSort(active ? { key: c.key, desc: !sort?.desc } : { key: c.key, desc: !!c.numeric })}
                  >
                    {c.label}
                    {active ? (sort?.desc ? " ▾" : " ▴") : ""}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {shown.length === 0 ? (
              <tr>
                <td colSpan={columns.length} className="muted" style={{ padding: 18, textAlign: "center" }}>
                  {emptyText ?? t("common.none")}
                </td>
              </tr>
            ) : (
              shown.map((row, i) => (
                <tr key={rowKey ? rowKey(row, i) : i} className={onRowClick ? "clickable" : ""} onClick={onRowClick ? () => onRowClick(row) : undefined}>
                  {columns.map((c) => {
                    const v = valueOf(c, row);
                    return (
                      <td key={c.key} className={`${c.numeric ? "num" : ""} ${c.className ?? ""}`} title={typeof v === "string" ? v : undefined}>
                        {c.render ? c.render(row) : c.numeric && typeof v === "number" ? formatNumber(v, locale, Number.isInteger(v) ? 0 : 2) : (v ?? "—")}
                      </td>
                    );
                  })}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
