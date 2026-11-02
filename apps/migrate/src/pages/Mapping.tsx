// Screen 3 — Mapping review: filters, table with selection, candidate panel, bulk actions,
// validation panel and pattern suggestions.
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import type { Breakdown, MappingRow, PlanSummary, UrlRow } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber, formatPercent } from "../lib/format";
import { DEFAULT_FILTER, STATUSES, confidenceTone, countBy, directories, filterMappings, pathOf, splitReason, statusTone } from "../lib/migrate";
import type { MappingFilterState } from "../lib/migrate";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import PatternSuggestions from "../components/PatternSuggestions";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import ValidationPanel from "../components/ValidationPanel";
import { Badge, Card, Empty, ErrorAlert, Field, Loading, Page, Segment } from "../components/ui";

export default function Mapping({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("nav.mapping")} onNavigate={onNavigate}>
      {(plan) => <MappingInner plan={plan} onNavigate={onNavigate} />}
    </PlanGate>
  );
}

export function useReasonLabel() {
  const { t } = useI18n();
  return (reason: string) => {
    const { code, n } = splitReason(reason);
    return n ? `${t(`reason.${code}`)} #${n}` : t(`reason.${code}`);
  };
}

type SortKey = "old" | "new" | "confidence" | "status" | "value";

function sortRows(rows: MappingRow[], key: SortKey, desc: boolean): MappingRow[] {
  const val = (m: MappingRow): string | number => {
    switch (key) {
      case "old":
        return pathOf(m.oldUrl);
      case "new":
        return pathOf(m.newUrl);
      case "confidence":
        return m.confidence;
      case "status":
        return STATUSES.indexOf(m.status);
      default:
        return m.value ?? -1;
    }
  };
  return [...rows].sort((a, b) => {
    const va = val(a);
    const vb = val(b);
    const cmp = typeof va === "number" && typeof vb === "number" ? va - vb : String(va).localeCompare(String(vb));
    return (desc ? -cmp : cmp) || a.confidence - b.confidence || a.oldUrl.localeCompare(b.oldUrl);
  });
}

function MappingInner({ plan, onNavigate }: { plan: PlanSummary; onNavigate: Navigate }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { version, bump } = usePlan();
  const reasonLabel = useReasonLabel();
  const data = useAsync(() => migrate.listMappings(plan.id), [plan.id, version]);
  const [filter, setFilter] = useState<MappingFilterState>(DEFAULT_FILTER);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [activeId, setActiveId] = useState<number | null>(null);
  const [threshold, setThreshold] = useState(80);
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "value", desc: true });
  const [busy, setBusy] = useState(false);

  const rows = useMemo(() => data.data?.rows ?? [], [data.data]);
  const filtered = useMemo(() => sortRows(filterMappings(rows, filter, locale), sort.key, sort.desc), [rows, filter, locale, sort]);
  const shown = filtered.slice(0, 500);
  const active = rows.find((r) => r.id === activeId) ?? null;
  const counts = useMemo(() => countBy(rows, (r) => r.status), [rows]);
  const dirs = useMemo(() => directories(rows), [rows]);
  const approvedCount = rows.filter((r) => r.approved).length;
  const setF = <K extends keyof MappingFilterState>(k: K, v: MappingFilterState[K]) => setFilter((f) => ({ ...f, [k]: v }));

  useEffect(() => {
    if (activeId === null && shown.length) setActiveId(shown[0].id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data.data]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const autoMatch = () =>
    run(async () => {
      const r = await migrate.autoMatch(plan.id, true);
      toast(t("mapping.matched", { count: formatNumber(r.processed, locale), rate: formatPercent(r.matchRate, locale), ms: formatNumber(r.elapsedMs, locale) }), "success");
      bump();
    });

  const approveSelected = () =>
    run(async () => {
      const n = await migrate.approve(plan.id, [...selected]);
      toast(t("mapping.approvedN", { count: formatNumber(n, locale) }), "success");
      setSelected(new Set());
      bump();
    });

  const approveAbove = () =>
    run(async () => {
      const n = await migrate.approve(plan.id, [], threshold);
      toast(t("mapping.approvedN", { count: formatNumber(n, locale) }), "success");
      bump();
    });

  const decide = async (m: MappingRow, status: string | null, newUrl?: string) => {
    try {
      const updated = await migrate.setMapping(plan.id, m.id, status, newUrl ?? null);
      data.setData((prev) => (prev ? { ...prev, rows: prev.rows.map((r) => (r.oldUrlId === updated.oldUrlId ? updated : r)) } : prev));
      setActiveId(updated.id);
      bump();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const toggle = (id: number) => setSelected((s) => {
    const next = new Set(s);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  const allShownSelected = shown.length > 0 && shown.every((m) => selected.has(m.id));
  const header = (key: SortKey, label: string, numeric = false) => (
    <th className={`sortable ${numeric ? "num" : ""}`} onClick={() => setSort((s) => (s.key === key ? { key, desc: !s.desc } : { key, desc: numeric }))}>
      {label}
      {sort.key === key ? (sort.desc ? " ▾" : " ▴") : ""}
    </th>
  );

  if (data.loading && !data.data) {
    return (
      <Page title={t("nav.mapping")} actions={<PlanSelect />}>
        <Loading />
      </Page>
    );
  }

  return (
    <Page
      title={t("nav.mapping")}
      subtitle={t("mapping.subtitle")}
      actions={
        <>
          <PlanSelect />
          <button type="button" className="btn btn--primary" disabled={busy || !plan.oldUrls || !plan.newUrls} onClick={autoMatch}>
            {busy ? t("common.loading") : t("mapping.runAuto")}
          </button>
        </>
      }
    >
      <ErrorAlert error={data.error} />
      {rows.length === 0 ? (
        <Empty
          icon="⇄"
          title={t("mapping.emptyTitle")}
          text={plan.oldUrls && plan.newUrls ? t("mapping.emptyReady") : t("mapping.emptyText")}
          action={
            plan.oldUrls && plan.newUrls ? (
              <button type="button" className="btn btn--primary" disabled={busy} onClick={autoMatch}>{t("mapping.runAuto")}</button>
            ) : (
              <button type="button" className="btn btn--primary" onClick={() => onNavigate("sources")}>{t("nav.sources")}</button>
            )
          }
        />
      ) : (
        <>
          <Card>
            <div className="row" style={{ gap: 6 }}>
              <button type="button" className={`chip ${filter.status === "all" ? "chip--active" : ""}`} onClick={() => setF("status", "all")}>
                {t("common.all")} <b>{formatNumber(rows.length, locale)}</b>
              </button>
              {STATUSES.map((s) => (
                <button key={s} type="button" className={`chip ${filter.status === s ? "chip--active" : ""}`} onClick={() => setF("status", filter.status === s ? "all" : s)} disabled={!counts[s]}>
                  <Badge tone={statusTone(s)}>{t(`status.${s}`)}</Badge> <b>{formatNumber(counts[s] ?? 0, locale)}</b>
                </button>
              ))}
              <span className="spacer" />
              <span className="muted" style={{ fontSize: 12 }}>{t("mapping.approvedOf", { approved: formatNumber(approvedCount, locale), total: formatNumber(rows.length, locale) })}</span>
            </div>
            <div className="filters">
              <Field label={t("mapping.search")}>
                <input className="input" value={filter.query} placeholder={t("common.search")} onChange={(e) => setF("query", e.target.value)} />
              </Field>
              <Field label={t("mapping.directory")}>
                <select className="select" value={filter.directory} onChange={(e) => setF("directory", e.target.value)}>
                  <option value="">{t("common.all")}</option>
                  {dirs.map((d) => (
                    <option key={d.dir} value={d.dir}>{d.dir} ({formatNumber(d.count, locale)})</option>
                  ))}
                </select>
              </Field>
              <Field label={t("mapping.confidence")}>
                <div className="row" style={{ flexWrap: "nowrap", gap: 6 }}>
                  <input className="input" type="number" min={0} max={100} value={filter.min} onChange={(e) => setF("min", Number(e.target.value) || 0)} />
                  <span className="muted">–</span>
                  <input className="input" type="number" min={0} max={100} value={filter.max} onChange={(e) => setF("max", Number(e.target.value) || 100)} />
                </div>
              </Field>
              <Field label={t("mapping.approval")}>
                <Segment<MappingFilterState["approval"]>
                  value={filter.approval}
                  onChange={(v) => setF("approval", v)}
                  options={[{ value: "all", label: t("common.all") }, { value: "pending", label: t("mapping.pending") }, { value: "approved", label: t("mapping.approved") }]}
                />
              </Field>
            </div>
            <div className="row">
              <button type="button" className="btn btn--sm" disabled={busy || selected.size === 0} onClick={approveSelected}>
                {t("mapping.approveSelected", { count: formatNumber(selected.size, locale) })}
              </button>
              <span className="muted" style={{ fontSize: 12 }}>{t("mapping.threshold")}</span>
              <input className="input" style={{ width: 72, height: 28 }} type="number" min={0} max={100} value={threshold} onChange={(e) => setThreshold(Number(e.target.value) || 0)} />
              <button type="button" className="btn btn--sm" disabled={busy} onClick={approveAbove}>{t("mapping.approveAbove", { value: threshold })}</button>
              {selected.size > 0 && <button type="button" className="btn btn--sm btn--ghost" onClick={() => setSelected(new Set())}>{t("mapping.clearSelection")}</button>}
              <span className="spacer" />
              <span className="muted" style={{ fontSize: 12 }}>
                {filtered.length > shown.length ? t("common.showing", { shown: formatNumber(shown.length, locale), total: formatNumber(filtered.length, locale) }) : t("common.rows", { count: formatNumber(filtered.length, locale) })}
              </span>
            </div>
          </Card>
          <div className="split">
            <div className="table-wrap table-wrap--tall">
              <table className="table mapping-table">
                <thead>
                  <tr>
                    <th style={{ width: 28 }}>
                      <input
                        type="checkbox"
                        checked={allShownSelected}
                        onChange={() => setSelected((s) => {
                          const next = new Set(s);
                          shown.forEach((m) => (allShownSelected ? next.delete(m.id) : next.add(m.id)));
                          return next;
                        })}
                      />
                    </th>
                    {header("old", t("mapping.oldUrl"))}
                    {header("new", t("mapping.newUrl"))}
                    {header("confidence", `${t("mapping.confidenceShort")} · ${t("mapping.reason")}`)}
                    {header("status", t("mapping.status"))}
                    {header("value", t("mapping.value"), true)}
                  </tr>
                </thead>
                <tbody>
                  {shown.length === 0 && (
                    <tr>
                      <td colSpan={6} className="muted" style={{ padding: 18, textAlign: "center" }}>{t("mapping.noMatchFilter")}</td>
                    </tr>
                  )}
                  {shown.map((m) => (
                    <tr key={m.id} className={`clickable ${m.id === activeId ? "selected" : ""}`} onClick={() => setActiveId(m.id)}>
                      <td onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" checked={selected.has(m.id)} onChange={() => toggle(m.id)} />
                      </td>
                      <td className="cell-url">
                        <div className="mono truncate" title={m.oldUrl}>{pathOf(m.oldUrl)}</div>
                        <div className="muted truncate cell-sub" title={m.oldTitle ?? ""}>{m.oldTitle ?? "—"}</div>
                      </td>
                      <td className="cell-url">
                        <div className="mono truncate" title={m.newUrl ?? ""}>{m.status === "gone" ? "410" : pathOf(m.newUrl)}</div>
                        <div className="muted truncate cell-sub" title={m.newTitle ?? ""}>{m.newTitle ?? "—"}</div>
                      </td>
                      <td className="nowrap">
                        <Badge tone={confidenceTone(m.confidence)}>{m.confidence}</Badge>
                        <div className="muted cell-sub">{reasonLabel(m.reason)}</div>
                      </td>
                      <td className="nowrap">
                        <Badge tone={statusTone(m.status)}>{t(`status.${m.status}`)}</Badge>
                        {m.approved && <span className="approved-mark" title={t("mapping.approved")}> ✓</span>}
                      </td>
                      <td className="num">{m.value === null ? "—" : formatNumber(m.value, locale)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="split__side">
              {active ? <CandidatePanel plan={plan} m={active} onDecide={decide} /> : <Card><p className="muted">{t("mapping.pickRow")}</p></Card>}
            </div>
          </div>
          <div className="grid grid--2">
            <ValidationPanel planId={plan.id} version={version} onSelect={(id) => { setFilter(DEFAULT_FILTER); setActiveId(id); }} onChanged={bump} />
            <PatternSuggestions planId={plan.id} version={version} onApplied={bump} />
          </div>
        </>
      )}
    </Page>
  );
}

const BREAKDOWN_KEYS: (keyof Breakdown)[] = ["slug", "path", "title", "simhash", "depth", "dir"];

function CandidatePanel({ plan, m, onDecide }: { plan: PlanSummary; m: MappingRow; onDecide: (m: MappingRow, status: string | null, newUrl?: string) => Promise<void> }) {
  const { t, locale } = useI18n();
  const reasonLabel = useReasonLabel();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<UrlRow[]>([]);
  const [manual, setManual] = useState("");

  useEffect(() => {
    setQuery("");
    setResults([]);
    setManual("");
  }, [m.id]);

  useEffect(() => {
    if (query.trim().length < 2) {
      setResults([]);
      return;
    }
    const id = window.setTimeout(() => {
      migrate.searchNew(plan.id, query.trim(), 12).then(setResults).catch(() => setResults([]));
    }, 250);
    return () => window.clearTimeout(id);
  }, [query, plan.id]);

  return (
    <>
      <Card title={t("mapping.panelTitle")}>
        <div className="field" style={{ gap: 4 }}>
          <span className="field__label">{t("mapping.oldUrl")}</span>
          <span className="mono break">{pathOf(m.oldUrl)}</span>
          <span className="muted" style={{ fontSize: 12.5 }}>{m.oldTitle ?? "—"}</span>
          <span className="row" style={{ gap: 6 }}>
            {m.oldStatus && <Badge tone={m.oldStatus === 200 ? "success" : "warning"}>{m.oldStatus}</Badge>}
            {m.value !== null && <Badge tone="gold">{t("mapping.valueN", { value: formatNumber(m.value, locale) })}</Badge>}
          </span>
        </div>
        <div className="field" style={{ gap: 4 }}>
          <span className="field__label">{t("mapping.current")}</span>
          <span className="mono break">{m.status === "gone" ? "410 Gone" : pathOf(m.newUrl)}</span>
          {m.newTitle && <span className="muted" style={{ fontSize: 12.5 }}>{m.newTitle}</span>}
          <span className="row" style={{ gap: 6 }}>
            <Badge tone={statusTone(m.status)}>{t(`status.${m.status}`)}</Badge>
            <Badge tone={confidenceTone(m.confidence)}>{m.confidence}</Badge>
            <span className="muted" style={{ fontSize: 12 }}>{reasonLabel(m.reason)}</span>
          </span>
        </div>
        <div className="row">
          {m.approved ? (
            <button type="button" className="btn btn--sm" onClick={() => onDecide(m, "unapprove")}>{t("mapping.unapprove")}</button>
          ) : (
            <button type="button" className="btn btn--sm btn--primary" onClick={() => onDecide(m, "approve")}>{t("mapping.approve")}</button>
          )}
          <button type="button" className="btn btn--sm btn--danger" disabled={m.status === "gone"} onClick={() => onDecide(m, "gone")}>{t("mapping.markGone")}</button>
          <button type="button" className="btn btn--sm btn--ghost" disabled={m.status === "keep"} onClick={() => onDecide(m, "keep")}>{t("mapping.keep")}</button>
        </div>
      </Card>
      <Card title={t("mapping.candidates")}>
        {m.candidates.length === 0 && <p className="muted" style={{ fontSize: 13 }}>{t("mapping.noCandidates")}</p>}
        <div className="stack">
          {m.candidates.map((c) => {
            const current = c.url === m.newUrl;
            return (
              <div key={c.url} className={`cand ${current ? "cand--active" : ""}`}>
                <div className="row" style={{ gap: 6, flexWrap: "nowrap" }}>
                  <Badge tone={confidenceTone(c.score)}>{c.score}</Badge>
                  <span className="mono truncate" title={c.url}>{pathOf(c.url)}</span>
                </div>
                <span className="muted" style={{ fontSize: 11.5 }}>{reasonLabel(c.reason)}</span>
                {BREAKDOWN_KEYS.filter((k) => c.breakdown[k] !== undefined).map((k) => (
                  <div key={k} className="meter">
                    <span>{t(`breakdown.${k}`)}</span>
                    <span className="meter__bar"><span className="meter__fill" style={{ width: `${Math.round((c.breakdown[k] ?? 0) * 100)}%` }} /></span>
                    <span className="num">{Math.round((c.breakdown[k] ?? 0) * 100)}</span>
                  </div>
                ))}
                {!current && (
                  <div className="row">
                    <button type="button" className="btn btn--sm" onClick={() => onDecide(m, "manual", c.url)}>{t("mapping.useThis")}</button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </Card>
      <Card title={t("mapping.searchNew")}>
        <input className="input" value={query} placeholder={t("mapping.searchNewPlaceholder")} onChange={(e) => setQuery(e.target.value)} />
        {results.length > 0 && (
          <div className="stack">
            {results.map((u) => (
              <div key={u.id} className="result">
                <div className="truncate">
                  <div className="mono truncate" title={u.url}>{pathOf(u.url)}</div>
                  <div className="muted truncate" style={{ fontSize: 11.5 }}>{u.title ?? "—"}</div>
                </div>
                <button type="button" className="btn btn--sm" onClick={() => onDecide(m, "manual", u.url)}>{t("mapping.choose")}</button>
              </div>
            ))}
          </div>
        )}
        <Field label={t("mapping.manualUrl")} hint={t("mapping.manualHint")}>
          <div className="row" style={{ flexWrap: "nowrap" }}>
            <input className="input" style={{ flex: 1 }} value={manual} placeholder="/yeni-yol" onChange={(e) => setManual(e.target.value)} onKeyDown={(e) => e.key === "Enter" && manual.trim() && onDecide(m, "manual", manual.trim())} />
            <button type="button" className="btn btn--sm" disabled={!manual.trim()} onClick={() => onDecide(m, "manual", manual.trim())}>{t("common.save")}</button>
          </div>
        </Field>
      </Card>
    </>
  );
}
