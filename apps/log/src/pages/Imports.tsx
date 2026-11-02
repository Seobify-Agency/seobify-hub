import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import CloudflareCard from "../components/CloudflareCard";
import { useNav } from "../components/LogKit";
import { fmtTs } from "../components/LogKit";
import { Badge, Card, DataTable, Empty, ErrorAlert, Field, Modal, Page } from "../components/ui";
import { useI18n } from "../i18n";
import { useAsync, useProjects, useToast } from "../lib/context";
import { formatBytes, formatNumber } from "../lib/format";
import { logApi } from "../lib/logapi";
import type { FileResult, ImportProgress, ImportResult, ImportRow } from "../lib/logapi";
import { useRange } from "../lib/range";

const FORMATS = ["combined", "common", "json", "cloudflare", "iis", "cloudfront", "alb"];

export default function Imports() {
  const { t, locale } = useI18n();
  const toast = useToast();
  const nav = useNav();
  const { project, projects } = useProjects();
  const { refresh, version } = useRange();
  const [paths, setPaths] = useState<string[]>([]);
  const [format, setFormat] = useState("auto");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [errorsOf, setErrorsOf] = useState<ImportRow | null>(null);
  const history = useAsync(() => (project ? logApi.imports(project.id) : Promise.resolve({ imports: [] as ImportRow[] })), [project?.id, version]);

  useEffect(() => {
    const un = listen<ImportProgress>("log-import-progress", (e) => setProgress(e.payload));
    return () => {
      void un.then((f) => f());
    };
  }, []);

  if (!projects.length || !project) {
    return (
      <Page title={t("nav.imports")}>
        <Empty title={t("project.emptyTitle")} text={t("project.emptyText")} action={<button type="button" className="btn btn--primary" onClick={() => nav("projects")}>{t("project.new")}</button>} />
      </Page>
    );
  }

  const run = async (list: string[], force: boolean) => {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      const r = await logApi.importLogs(project.id, list, format === "auto" ? null : format, force);
      setResult(r);
      const s = r.summary;
      toast(t("imp.result", { imported: s.imported, dup: s.duplicates, skipped: s.skipped, errors: s.errors, lines: formatNumber(s.lines, locale), bots: formatNumber(s.botHits, locale), secs: formatNumber(s.durationMs / 1000, locale, 1) }), s.errors ? "error" : "success");
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const duplicates: FileResult[] = result?.summary.files.filter((f) => f.status === "duplicate") ?? [];
  const pct = progress && progress.bytesTotal > 0 ? Math.min(100, (progress.bytesRead / progress.bytesTotal) * 100) : 0;
  const rows = history.data?.imports ?? [];

  return (
    <Page title={t("nav.imports")} subtitle={t("imp.subtitle")}>
      <CloudflareCard project={project} onImported={refresh} />
      <Card title={t("imp.new")} hint={t("imp.formats")}>
        <div className="row">
          <button type="button" className="btn" disabled={busy} onClick={async () => {
            const picked = await open({ multiple: true, filters: [{ name: "Logs", extensions: ["log", "gz", "txt", "json", "jsonl", "ndjson", "csv"] }, { name: "*", extensions: ["*"] }] });
            if (Array.isArray(picked)) setPaths((p) => [...new Set([...p, ...picked])]);
            else if (typeof picked === "string") setPaths((p) => [...new Set([...p, picked])]);
          }}>{t("imp.files")}</button>
          <button type="button" className="btn" disabled={busy} onClick={async () => {
            const picked = await open({ directory: true, multiple: false });
            if (typeof picked === "string") setPaths((p) => [...new Set([...p, picked])]);
          }}>{t("imp.folder")}</button>
          <Field label={t("imp.format")}>
            <select className="select" value={format} disabled={busy} onChange={(e) => setFormat(e.target.value)}>
              <option value="auto">{t("imp.auto")}</option>
              {FORMATS.map((f) => <option key={f} value={f}>{t(`fmt.${f}`)}</option>)}
            </select>
          </Field>
          <span className="spacer" />
          {paths.length > 0 && <button type="button" className="btn btn--ghost btn--sm" disabled={busy} onClick={() => setPaths([])}>{t("imp.clear")}</button>}
          <button type="button" className="btn btn--primary" disabled={busy || !paths.length} onClick={() => run(paths, false)}>{busy ? <span className="spinner" /> : "⇪"} {busy ? t("imp.running") : t("imp.start")}</button>
        </div>
        {paths.length > 0 && <pre className="code">{paths.join("\n")}</pre>}
        {busy && progress && (
          <div className="field">
            <div className="progress"><div className="progress__bar" style={{ width: `${pct}%` }} /></div>
            <span className="muted" style={{ fontSize: 12.5 }}>
              {t(`imp.phase.${progress.phase}`)} · {t("imp.progress", { file: `${progress.fileIndex + 1}/${progress.fileCount} ${progress.file}`, lines: formatNumber(progress.lines, locale), rate: formatNumber(progress.linesPerSec, locale), failed: formatNumber(progress.failed, locale), bots: formatNumber(progress.botHits, locale) })}
            </span>
          </div>
        )}
        <ErrorAlert error={error} />
        {result && (
          <div className={`alert ${result.summary.errors ? "alert--error" : "alert--success"}`}>
            {t("imp.result", { imported: result.summary.imported, dup: result.summary.duplicates, skipped: result.summary.skipped, errors: result.summary.errors, lines: formatNumber(result.summary.lines, locale), bots: formatNumber(result.summary.botHits, locale), secs: formatNumber(result.summary.durationMs / 1000, locale, 1) })}
            {result.summary.files.filter((f) => f.message && f.status !== "duplicate").map((f) => <div key={f.path}>• {f.name}: {f.message}</div>)}
          </div>
        )}
        {duplicates.length > 0 && !busy && (
          <div className="alert alert--info row">
            <span>{t("imp.duplicates", { count: duplicates.length })}</span>
            <span className="spacer" />
            <button type="button" className="btn btn--sm" onClick={() => run(duplicates.map((d) => d.path), true)}>{t("imp.reimport")}</button>
          </div>
        )}
      </Card>
      <Card title={t("imp.history")}>
        <ErrorAlert error={history.error} />
        {rows.length === 0 ? (
          <p className="muted">{t("imp.noHistory")}</p>
        ) : (
          <DataTable
            rows={rows}
            rowKey={(r) => r.id}
            initialSort={{ key: "importedAt", desc: true }}
            columns={[
              { key: "name", label: t("col.file"), render: (r) => <span title={r.path}><b>{r.name}</b></span> },
              { key: "format", label: t("col.format"), render: (r) => <Badge tone="gold">{r.format}</Badge> },
              { key: "sizeBytes", label: t("col.size"), numeric: true, render: (r) => formatBytes(r.sizeBytes, locale) },
              { key: "lines", label: t("col.lines"), numeric: true },
              { key: "failed", label: t("col.failed"), numeric: true, render: (r) => (r.failed ? <button type="button" className="btn btn--ghost btn--sm" onClick={() => setErrorsOf(r)}>{formatNumber(r.failed, locale)} ⓘ</button> : "0") },
              { key: "botHits", label: t("col.botHits"), numeric: true },
              { key: "humanRequests", label: t("col.human"), numeric: true },
              { key: "durationMs", label: t("col.duration"), numeric: true, render: (r) => `${formatNumber(r.durationMs / 1000, locale, 1)} s` },
              { key: "firstTs", label: t("col.period"), render: (r) => `${fmtTs(r.firstTs?.slice(0, 10), locale)} → ${fmtTs(r.lastTs?.slice(0, 10), locale)}` },
              { key: "importedAt", label: t("col.importedAt"), render: (r) => fmtTs(r.importedAt, locale) },
              {
                key: "actions", label: "", sortable: false, render: (r) => (
                  <button type="button" className="btn btn--danger btn--sm" onClick={async () => {
                    if (!window.confirm(t("common.confirmDelete", { name: r.name }))) return;
                    try {
                      await logApi.deleteImport(project.id, r.id);
                      toast(t("imp.deleted"), "success");
                      refresh();
                    } catch (e) {
                      toast(e instanceof Error ? e.message : String(e), "error");
                    }
                  }}>{t("common.delete")}</button>
                ),
              },
            ]}
          />
        )}
      </Card>
      {errorsOf && (
        <Modal title={t("imp.errors", { name: errorsOf.name })} onClose={() => setErrorsOf(null)} wide>
          <p className="muted" style={{ fontSize: 12.5 }}>{t("imp.errorsHint")}</p>
          <pre className="code">{errorsOf.sampleErrors.map((e) => `#${e.line} [${e.reason}] ${e.text}`).join("\n")}</pre>
        </Modal>
      )}
    </Page>
  );
}
