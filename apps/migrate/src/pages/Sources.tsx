// Screen 2 — Sources: old/new URL sets (ImportDialog), priority values CSV, content fetch.
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";
import { isTauri, migrate } from "../lib/api";
import type { ImportSource, PlanSummary, ProgressEvent, Side, SideStats } from "../lib/api";
import { useAsync, useProjects, useToast } from "../lib/context";
import { formatNumber, formatPercent } from "../lib/format";
import { pathOf } from "../lib/migrate";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import ImportDialog from "../components/ImportDialog";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import { Badge, Card, DataTable, ErrorAlert, Page, Segment, Stat } from "../components/ui";

export default function Sources({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("nav.sources")} onNavigate={onNavigate}>
      {(plan) => <SourcesInner plan={plan} onNavigate={onNavigate} />}
    </PlanGate>
  );
}

function SideCard({ plan, side, stats, onImport, onChanged }: { plan: PlanSummary; side: Side; stats: SideStats | null; onImport: () => void; onChanged: () => void }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const pct = (n: number) => (stats && stats.total ? formatPercent(n / stats.total, locale, 0) : "—");
  const clear = async () => {
    if (!window.confirm(t("sources.confirmClear", { side: t(`sources.${side}`) }))) return;
    try {
      await migrate.clearSide(plan.id, side);
      onChanged();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };
  return (
    <Card
      title={t(`sources.${side}Title`)}
      hint={side === "old" ? plan.oldBase : plan.newBase}
      actions={
        <>
          <button type="button" className="btn btn--primary btn--sm" onClick={onImport}>{t("common.import")}</button>
          {!!stats?.total && <button type="button" className="btn btn--danger btn--sm" onClick={clear}>{t("sources.clear")}</button>}
        </>
      }
    >
      <div className="grid grid--stats">
        <Stat label={t("sources.urls")} value={formatNumber(stats?.total ?? 0, locale)} />
        <Stat label={t("sources.withTitle")} value={pct(stats?.withTitle ?? 0)} sub={formatNumber(stats?.withTitle ?? 0, locale)} />
        <Stat label={t("sources.status200")} value={pct(stats?.status200 ?? 0)} sub={stats?.non200 ? t("sources.non200", { count: formatNumber(stats.non200, locale) }) : undefined} />
        <Stat label={t("sources.indexable")} value={pct(stats?.indexable ?? 0)} />
        {side === "old" ? (
          <Stat label={t("sources.withValue")} value={formatNumber(stats?.withValue ?? 0, locale)} sub={stats?.totalValue ? t("sources.totalValue", { value: formatNumber(stats.totalValue, locale) }) : undefined} />
        ) : (
          <Stat label={t("sources.withSimhash")} value={formatNumber(stats?.withSimhash ?? 0, locale)} />
        )}
      </div>
    </Card>
  );
}

function SourcesInner({ plan, onNavigate }: { plan: PlanSummary; onNavigate: Navigate }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { project } = useProjects();
  const { version, bump } = usePlan();
  const overview = useAsync(() => migrate.overview(plan.id), [plan.id, version]);
  const [dialog, setDialog] = useState<Side | null>(null);
  const [replace, setReplace] = useState(false);
  const [view, setView] = useState<Side>("old");
  const urls = useAsync(() => migrate.listUrls(plan.id, view), [plan.id, view, version]);
  const [fetchSide, setFetchSide] = useState<"both" | Side>("both");
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    const off = listen<ProgressEvent>("migrate-progress", (e) => {
      if (e.payload.task === "fetch") setProgress({ done: e.payload.done, total: e.payload.total });
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);

  const doImport = async (side: Side, source: ImportSource) => {
    const r = await migrate.importUrls(plan.id, side, source, replace);
    toast(t("sources.imported", { count: formatNumber(r.inserted + r.updated, locale), side: t(`sources.${side}`), skipped: formatNumber(r.skippedResources, locale) }), "success");
    bump();
  };

  const pickValues = async () => {
    const path = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }] });
    if (typeof path !== "string") return;
    try {
      const r = await migrate.importValues(plan.id, path);
      toast(t("sources.valuesImported", { matched: formatNumber(r.matched, locale), rows: formatNumber(r.rows, locale), unmatched: formatNumber(r.unmatched, locale) }), r.matched ? "success" : "error");
      bump();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const fetchContent = async () => {
    setBusy(true);
    setProgress({ done: 0, total: 0 });
    try {
      const r = await migrate.fetchContent(plan.id, fetchSide === "both" ? null : fetchSide, true);
      toast(t("sources.fetched", { updated: formatNumber(r.updated, locale), requested: formatNumber(r.requested, locale), failed: formatNumber(r.failed, locale) }), r.failed ? "info" : "success");
      bump();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  const ov = overview.data;
  const ready = !!ov && ov.old.total > 0 && ov.new.total > 0;
  return (
    <Page title={t("nav.sources")} subtitle={t("sources.subtitle")} actions={<PlanSelect />}>
      <ErrorAlert error={overview.error} />
      <div className="row">
        <label className="checkbox">
          <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} />
          {t("sources.replace")}
        </label>
        <span className="spacer" />
        {ready && (
          <button type="button" className="btn btn--primary" onClick={() => onNavigate("mapping")}>
            {t("sources.toMapping")} →
          </button>
        )}
      </div>
      <div className="grid grid--2">
        <SideCard plan={plan} side="old" stats={ov?.old ?? null} onImport={() => setDialog("old")} onChanged={bump} />
        <SideCard plan={plan} side="new" stats={ov?.new ?? null} onImport={() => setDialog("new")} onChanged={bump} />
      </div>
      <div className="grid grid--2">
        <Card title={t("sources.valuesTitle")} hint={t("sources.valuesHint")}>
          <div className="row">
            <button type="button" className="btn" onClick={pickValues} disabled={!ov?.old.total}>{t("sources.valuesPick")}</button>
            {!!ov?.old.withValue && <Badge tone="success">{t("sources.valuesSet", { count: formatNumber(ov.old.withValue, locale) })}</Badge>}
          </div>
        </Card>
        <Card title={t("sources.fetchTitle")} hint={t("sources.fetchHint")}>
          <div className="row">
            <Segment<"both" | Side> value={fetchSide} onChange={setFetchSide} options={[{ value: "both", label: t("common.all") }, { value: "old", label: t("sources.old") }, { value: "new", label: t("sources.new") }]} />
            <button type="button" className="btn" disabled={busy || !ov || ov.old.total + ov.new.total === 0} onClick={fetchContent}>{t("sources.fetchRun")}</button>
          </div>
          {progress && (
            <div className="field">
              <div className="progress"><div className="progress__bar" style={{ width: `${progress.total ? (progress.done / progress.total) * 100 : 3}%` }} /></div>
              <span className="muted" style={{ fontSize: 12 }}>{formatNumber(progress.done, locale)} / {formatNumber(progress.total, locale)}</span>
            </div>
          )}
        </Card>
      </div>
      <Card title={t("sources.preview")} actions={<Segment<Side> value={view} onChange={setView} options={[{ value: "old", label: t("sources.old") }, { value: "new", label: t("sources.new") }]} />}>
        <ErrorAlert error={urls.error} />
        <DataTable
          rows={urls.data ?? []}
          rowKey={(u) => u.id}
          emptyText={t("sources.emptyList")}
          columns={[
            { key: "url", label: "URL", className: "url", value: (u) => pathOf(u.url), render: (u) => <span title={u.url}>{pathOf(u.url)}</span> },
            { key: "title", label: t("sources.title"), className: "truncate" },
            { key: "h1", label: "H1", className: "truncate" },
            { key: "statusCode", label: t("sources.status"), numeric: true, render: (u) => (u.statusCode ? <Badge tone={u.statusCode === 200 ? "success" : u.statusCode < 400 ? "warning" : "error"}>{u.statusCode}</Badge> : "—") },
            { key: "indexability", label: t("sources.indexability") },
            ...(view === "old" ? [{ key: "value", label: t("sources.value"), numeric: true }] : []),
            { key: "simhash", label: "Simhash", render: (u) => (u.simhash ? "✓" : "—") },
          ]}
        />
      </Card>
      {dialog && (
        <ImportDialog
          title={t(`sources.import_${dialog}`)}
          defaultSpiderProject={project?.spiderProject ?? null}
          confirmLabel={t("common.import")}
          onClose={() => setDialog(null)}
          onConfirm={(source) => doImport(dialog, source)}
        />
      )}
    </Page>
  );
}
