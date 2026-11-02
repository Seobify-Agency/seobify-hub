// Screen 6 — Verification: post-launch / staging runs, results, run comparison, report.
import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { useI18n } from "../i18n";
import { api, isTauri, migrate } from "../lib/api";
import type { Comparison, PlanSummary, ProgressEvent, VerifyKind, VerifyResultRow, VerifySummary } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatDate, formatNumber, formatPercent, toCsv } from "../lib/format";
import { OUTCOMES, countBy, outcomeTone, pathOf } from "../lib/migrate";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import { ShareBar } from "../components/charts";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import { Badge, Card, DataTable, Empty, ErrorAlert, Field, Page, Segment, Stat } from "../components/ui";

const OUTCOME_COLORS: Record<string, string> = {
  ok: "var(--success)",
  wrong_target: "var(--sev-critical)",
  chain: "var(--warning)",
  temporary: "var(--sev-medium)",
  not_redirected: "var(--error)",
  target_error: "var(--sev-high)",
  loop: "var(--series-4)",
  error: "var(--sev-info)",
};

export default function Verify({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("nav.verify")} onNavigate={onNavigate}>
      {(plan) => <VerifyInner plan={plan} />}
    </PlanGate>
  );
}

function hopsText(r: VerifyResultRow): string {
  return r.hops.map((h) => `${h.status} ${pathOf(h.url)}`).join(" → ") || (r.error ?? "");
}

function VerifyInner({ plan }: { plan: PlanSummary }) {
  const { t, lang, locale } = useI18n();
  const toast = useToast();
  const { version, bump } = usePlan();
  const [kind, setKind] = useState<VerifyKind>("post");
  const [limit, setLimit] = useState(500);
  const [delay, setDelay] = useState(100);
  const [includeAuto, setIncludeAuto] = useState(true);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [summary, setSummary] = useState<VerifySummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runs = useAsync(() => migrate.listRuns(plan.id), [plan.id, version]);
  const [runId, setRunId] = useState<number | null>(null);
  const [results, setResults] = useState<VerifyResultRow[] | null>(null);
  const [outcome, setOutcome] = useState<string>("all");
  const [compareA, setCompareA] = useState<number | null>(null);
  const [compareB, setCompareB] = useState<number | null>(null);
  const [comparison, setComparison] = useState<Comparison | null>(null);

  useEffect(() => {
    if (!isTauri) return;
    const off = listen<ProgressEvent>("migrate-progress", (e) => {
      if (e.payload.task === "verify") setProgress({ done: e.payload.done, total: e.payload.total });
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);

  // Show the latest run when the screen opens.
  useEffect(() => {
    if (runId === null && runs.data && runs.data.length) setRunId(runs.data[0].id);
  }, [runs.data, runId]);

  useEffect(() => {
    if (runId === null) return;
    if (summary && summary.run.id === runId) {
      setResults(summary.results);
      return;
    }
    migrate.runResults(runId).then(setResults).catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }, [runId, summary]);

  const start = async () => {
    setRunning(true);
    setError(null);
    setProgress({ done: 0, total: 0 });
    try {
      const s = await migrate.verify(plan.id, kind, limit || null, delay, includeAuto);
      setSummary(s);
      setRunId(s.run.id);
      setComparison(s.comparison);
      toast(t("verify.done", { ok: formatNumber(s.run.ok, locale), total: formatNumber(s.run.total, locale) }), s.run.failed ? "info" : "success");
      bump();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
      setProgress(null);
    }
  };

  const compare = async () => {
    if (compareA === null || compareB === null) return;
    try {
      setComparison(await migrate.compareRuns(compareA, compareB));
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const report = async () => {
    try {
      const r = await migrate.report(plan.id, lang);
      toast(t("common.reportSaved", { path: r.html }), "success");
      void openPath(r.html).catch(() => {});
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const exportCsv = async () => {
    if (!results) return;
    const path = await save({ defaultPath: `verify-${runId}.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;
    const csv = toCsv(
      ["source", "checked", "expected", "outcome", "final_status", "final_url", "hops", "error"],
      results.map((r) => [r.sourceUrl, r.checkedUrl, r.expectedUrl, r.outcome, r.finalStatus, r.finalUrl, r.hops.map((h) => `${h.status} ${h.url}`).join(" > "), r.error]),
    );
    await api.writeTextFile(path, csv, true);
    toast(t("common.fileSaved", { path }), "success");
  };

  const run = runs.data?.find((r) => r.id === runId) ?? summary?.run ?? null;
  const counts = useMemo(() => countBy(results ?? [], (r) => r.outcome), [results]);
  const shown = (results ?? []).filter((r) => outcome === "all" || r.outcome === outcome);
  const isStaging = run?.kind === "staging";

  return (
    <Page
      title={t("nav.verify")}
      subtitle={t("verify.subtitle")}
      actions={
        <>
          <PlanSelect />
          <button type="button" className="btn" onClick={report}>{t("common.exportReport")}</button>
        </>
      }
    >
      <Card title={t("verify.runTitle")} hint={kind === "post" ? t("verify.postHint") : t("verify.stagingHint")}>
        <div className="filters">
          <Field label={t("verify.kind")}>
            <Segment<VerifyKind> value={kind} onChange={setKind} options={[{ value: "post", label: t("verify.post") }, { value: "staging", label: t("verify.staging") }]} />
          </Field>
          <Field label={t("verify.limit")}>
            <input className="input" type="number" min={1} value={limit} onChange={(e) => setLimit(Math.max(1, Number(e.target.value) || 1))} />
          </Field>
          <Field label={t("verify.delay")}>
            <input className="input" type="number" min={0} step={50} value={delay} onChange={(e) => setDelay(Math.max(0, Number(e.target.value) || 0))} />
          </Field>
          <Field label={t("verify.scope")}>
            <label className="checkbox" style={{ height: 34 }}>
              <input type="checkbox" checked={includeAuto} onChange={(e) => setIncludeAuto(e.target.checked)} />
              {t("export.includeAuto")}
            </label>
          </Field>
        </div>
        {kind === "staging" && !plan.stagingBase && <div className="alert alert--error">{t("verify.noStaging")}</div>}
        <div className="row">
          <button type="button" className="btn btn--primary" disabled={running || (kind === "staging" && !plan.stagingBase)} onClick={start}>
            {running ? t("verify.running") : t("verify.run")}
          </button>
          <span className="muted" style={{ fontSize: 12 }}>{t("verify.polite")}</span>
        </div>
        {progress && (
          <div className="field">
            <div className="progress"><div className="progress__bar" style={{ width: `${progress.total ? (progress.done / progress.total) * 100 : 2}%` }} /></div>
            <span className="muted" style={{ fontSize: 12 }}>{formatNumber(progress.done, locale)} / {formatNumber(progress.total, locale)}</span>
          </div>
        )}
        <ErrorAlert error={error} />
      </Card>

      {!run && !runs.loading && <Empty icon="✓" title={t("verify.emptyTitle")} text={t("verify.emptyText")} />}

      {run && (
        <>
          <div className="grid grid--stats">
            <Stat label={t("verify.kind")} value={<span style={{ fontSize: 18 }}>{t(`verify.${run.kind}`)}</span>} sub={formatDate(run.finishedAt ?? run.startedAt, locale)} />
            <Stat label={t("verify.checked")} value={formatNumber(run.total, locale)} />
            <Stat label={t("verify.ok")} value={formatNumber(run.ok, locale)} />
            <Stat label={t("verify.failed")} value={formatNumber(run.failed, locale)} />
            <Stat label={t("verify.success")} value={run.total ? formatPercent(run.ok / run.total, locale) : "—"} />
          </div>
          <Card title={t("verify.results")} actions={<button type="button" className="btn btn--sm" disabled={!results?.length} onClick={exportCsv}>{t("common.exportCsv")}</button>}>
            <ShareBar parts={OUTCOMES.map((o) => ({ label: t(`outcome.${o}`), value: counts[o] ?? 0, color: OUTCOME_COLORS[o] }))} />
            <div className="row" style={{ gap: 6 }}>
              <button type="button" className={`chip ${outcome === "all" ? "chip--active" : ""}`} onClick={() => setOutcome("all")}>{t("common.all")} <b>{formatNumber(results?.length ?? 0, locale)}</b></button>
              {OUTCOMES.filter((o) => counts[o]).map((o) => (
                <button key={o} type="button" className={`chip ${outcome === o ? "chip--active" : ""}`} onClick={() => setOutcome(o)}>
                  <Badge tone={outcomeTone(o)}>{t(`outcome.${o}`)}</Badge> <b>{formatNumber(counts[o], locale)}</b>
                </button>
              ))}
            </div>
            <DataTable
              rows={shown}
              rowKey={(r, i) => `${r.checkedUrl}-${i}`}
              columns={[
                { key: "outcome", label: t("verify.outcome"), render: (r) => <Badge tone={outcomeTone(r.outcome)}>{t(`outcome.${r.outcome}`)}</Badge> },
                { key: "sourceUrl", label: isStaging ? t("verify.target") : t("mapping.oldUrl"), className: "url", value: (r) => pathOf(r.checkedUrl), render: (r) => <span title={r.checkedUrl}>{isStaging ? r.checkedUrl : pathOf(r.sourceUrl)}</span> },
                ...(isStaging ? [] : [{ key: "expectedUrl", label: t("verify.expected"), className: "url", value: (r: VerifyResultRow) => pathOf(r.expectedUrl), render: (r: VerifyResultRow) => <span title={r.expectedUrl ?? ""}>{r.expectedUrl ? pathOf(r.expectedUrl) : "410"}</span> }]),
                { key: "finalStatus", label: t("verify.finalStatus"), numeric: true },
                { key: "hops", label: t("verify.hops"), value: (r) => hopsText(r), render: (r) => <span className="mono" style={{ fontSize: 11.5 }} title={r.hops.map((h) => `${h.status} ${h.url}`).join("\n")}>{hopsText(r)}</span> },
              ]}
            />
          </Card>
        </>
      )}

      <div className="grid grid--2">
        <Card title={t("verify.history")}>
          <ErrorAlert error={runs.error} />
          <DataTable
            rows={runs.data ?? []}
            rowKey={(r) => r.id}
            searchable={false}
            onRowClick={(r) => setRunId(r.id)}
            emptyText={t("verify.noRuns")}
            columns={[
              { key: "id", label: "#", numeric: true, render: (r) => <b style={{ color: r.id === runId ? "var(--accent-hover)" : undefined }}>{r.id}</b> },
              { key: "kind", label: t("verify.kind"), render: (r) => t(`verify.${r.kind}`) },
              { key: "finishedAt", label: t("verify.date"), render: (r) => <span className="muted">{formatDate(r.finishedAt, locale)}</span> },
              { key: "total", label: t("verify.checked"), numeric: true },
              { key: "ok", label: t("verify.ok"), numeric: true },
              { key: "failed", label: t("verify.failed"), numeric: true },
            ]}
          />
        </Card>
        <Card title={t("verify.compare")} hint={t("verify.compareHint")}>
          <div className="row" style={{ flexWrap: "nowrap" }}>
            <select className="select" value={compareA ?? ""} onChange={(e) => setCompareA(e.target.value ? Number(e.target.value) : null)}>
              <option value="">{t("verify.before")}</option>
              {runs.data?.map((r) => <option key={r.id} value={r.id}>#{r.id} · {t(`verify.${r.kind}`)} · {formatDate(r.finishedAt, locale)}</option>)}
            </select>
            <select className="select" value={compareB ?? ""} onChange={(e) => setCompareB(e.target.value ? Number(e.target.value) : null)}>
              <option value="">{t("verify.after")}</option>
              {runs.data?.map((r) => <option key={r.id} value={r.id}>#{r.id} · {t(`verify.${r.kind}`)} · {formatDate(r.finishedAt, locale)}</option>)}
            </select>
            <button type="button" className="btn" disabled={compareA === null || compareB === null || compareA === compareB} onClick={compare}>{t("verify.compareRun")}</button>
          </div>
          {comparison && (
            <div className="field">
              <div className="row" style={{ gap: 8 }}>
                <Badge tone="success">{t("verify.fixed", { count: formatNumber(comparison.fixed.length, locale) })}</Badge>
                <Badge tone="error">{t("verify.broken", { count: formatNumber(comparison.broken.length, locale) })}</Badge>
                <Badge tone="warning">{t("verify.stillFailing", { count: formatNumber(comparison.stillFailing, locale) })}</Badge>
                <Badge>{t("verify.newChecks", { count: formatNumber(comparison.newChecks, locale) })}</Badge>
              </div>
              <span className="muted" style={{ fontSize: 12 }}>#{comparison.previousRun.id} → #{comparison.currentRun.id}</span>
              {[...comparison.broken.map((r) => ["broken", r] as const), ...comparison.fixed.map((r) => ["fixed", r] as const)].slice(0, 40).map(([k, r], i) => (
                <div key={`${k}-${i}`} className="row" style={{ fontSize: 12, flexWrap: "nowrap" }}>
                  <Badge tone={k === "fixed" ? "success" : "error"}>{t(`verify.${k}Short`)}</Badge>
                  <span className="mono truncate">{pathOf(r.checkedUrl)}</span>
                  <span className="muted nowrap">{t(`outcome.${r.outcome}`)}</span>
                </div>
              ))}
            </div>
          )}
        </Card>
      </div>
    </Page>
  );
}
