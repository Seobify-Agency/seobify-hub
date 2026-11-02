// Screen 5 — Export: format choice, options, preview and save.
import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import type { ExportOptions, ExportOutput, PlanSummary } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import { Badge, Card, ErrorAlert, Loading, Page, Segment, Stat } from "../components/ui";

const FORMAT_KEY = "seobify-migrate-format";
const CSV_FORMATS = new Set(["cloudflare", "wordpress", "csv"]);
const PREVIEW_LINES = 400;

export default function Export({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("nav.export")} onNavigate={onNavigate}>
      {(plan) => <ExportInner plan={plan} />}
    </PlanGate>
  );
}

function ExportInner({ plan }: { plan: PlanSummary }) {
  const { t, lang, locale } = useI18n();
  const toast = useToast();
  const { version } = usePlan();
  const formats = useAsync(() => migrate.exportFormats(), []);
  const [format, setFormat] = useState<string>(() => {
    try {
      return localStorage.getItem(FORMAT_KEY) || "apache";
    } catch {
      return "apache";
    }
  });
  const [preserveQuery, setPreserveQuery] = useState(true);
  const [includeAuto, setIncludeAuto] = useState(true);
  const [header, setHeader] = useState<boolean | null>(null);
  const [commentLang, setCommentLang] = useState<string>(lang);
  const [out, setOut] = useState<ExportOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const options: ExportOptions = { preserveQuery, includeHeader: header, lang: commentLang };

  useEffect(() => {
    let alive = true;
    setLoading(true);
    migrate
      .exportRules(plan.id, format, options, includeAuto)
      .then((o) => alive && (setOut(o), setError(null)))
      .catch((e) => alive && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan.id, format, preserveQuery, includeAuto, header, commentLang, version]);

  const choose = (f: string) => {
    setFormat(f);
    setHeader(null);
    try {
      localStorage.setItem(FORMAT_KEY, f);
    } catch {
      /* storage unavailable */
    }
  };

  const saveFile = async () => {
    if (!out) return;
    try {
      const defaultPath = await migrate.defaultExportPath(plan.id, format);
      const path = await save({ defaultPath, title: t("export.saveTitle") });
      if (!path) return;
      const written = await migrate.saveExport(plan.id, format, options, includeAuto, path);
      toast(t("common.fileSaved", { path: written }), "success");
      void revealItemInDir(written).catch(() => {});
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const copy = async () => {
    if (!out) return;
    await navigator.clipboard.writeText(out.content);
    toast(t("common.copied"), "success");
  };

  const lines = out ? out.content.split("\n") : [];
  const headerDefault = format === "cloudflare" ? false : true;
  return (
    <Page title={t("nav.export")} subtitle={t("export.subtitle")} actions={<PlanSelect />}>
      <Card title={t("export.format")}>
        {formats.loading && <Loading />}
        <div className="format-grid">
          {formats.data?.map((f) => (
            <button key={f.id} type="button" className={`format-btn ${format === f.id ? "format-btn--active" : ""}`} onClick={() => choose(f.id)}>
              <b>{f.label}</b>
              <span className="mono muted">{f.filename}</span>
            </button>
          ))}
        </div>
        <div className="row" style={{ gap: 18 }}>
          <label className="checkbox">
            <input type="checkbox" checked={includeAuto} onChange={(e) => setIncludeAuto(e.target.checked)} />
            {t("export.includeAuto")}
          </label>
          <label className="checkbox">
            <input type="checkbox" checked={preserveQuery} onChange={(e) => setPreserveQuery(e.target.checked)} />
            {t("export.preserveQuery")}
          </label>
          {CSV_FORMATS.has(format) && (
            <label className="checkbox">
              <input type="checkbox" checked={header ?? headerDefault} onChange={(e) => setHeader(e.target.checked)} />
              {t("export.header")}
            </label>
          )}
          <span className="row" style={{ gap: 8 }}>
            <span className="muted" style={{ fontSize: 12 }}>{t("export.commentLang")}</span>
            <Segment<string> value={commentLang} onChange={setCommentLang} options={[{ value: "tr", label: "TR" }, { value: "en", label: "EN" }]} />
          </span>
        </div>
      </Card>
      <ErrorAlert error={error} />
      {out && (
        <>
          <div className="grid grid--stats">
            <Stat label={t("export.rules")} value={formatNumber(out.rules, locale)} />
            <Stat label={t("export.patternRules")} value={formatNumber(out.patternRules, locale)} />
            <Stat label={t("export.skipped")} value={formatNumber(out.skipped, locale)} />
            <Stat label={t("export.file")} value={<span className="mono" style={{ fontSize: 16 }}>{out.filename}</span>} />
          </div>
          {out.warnings.length > 0 && (
            <div className="alert alert--info">
              {out.warnings.slice(0, 12).map((w, i) => (
                <div key={i}>• {w}</div>
              ))}
              {out.warnings.length > 12 && <div>… +{formatNumber(out.warnings.length - 12, locale)}</div>}
            </div>
          )}
          <Card
            title={t("export.preview")}
            hint={lines.length > PREVIEW_LINES ? t("export.previewCut", { shown: formatNumber(PREVIEW_LINES, locale), total: formatNumber(lines.length, locale) }) : undefined}
            actions={
              <>
                {loading && <Badge>{t("common.loading")}</Badge>}
                <button type="button" className="btn btn--sm" onClick={copy}>{t("common.copy")}</button>
                <button type="button" className="btn btn--sm btn--primary" disabled={!out.content} onClick={saveFile}>{t("common.save")}</button>
              </>
            }
          >
            <pre className="code code--tall">{lines.slice(0, PREVIEW_LINES).join("\n")}</pre>
          </Card>
        </>
      )}
    </Page>
  );
}
