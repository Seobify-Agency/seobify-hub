// Seobify Log building blocks: range bar, data gate, CSV / report buttons, badges.
import { createContext, useContext, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { useProjects, useToast } from "../lib/context";
import { formatNumber, toCsv } from "../lib/format";
import { logApi } from "../lib/logapi";
import type { RangeArgs } from "../lib/logapi";
import { useRange } from "../lib/range";
import type { PageId } from "../nav";
import { Badge, Empty, ErrorAlert, Loading, Segment } from "./ui";
import type { Tone } from "./ui";

export const NavContext = createContext<(page: PageId) => void>(() => {});
export const useNav = () => useContext(NavContext);

export function RangeBar() {
  const { t } = useI18n();
  const { preset, range, bounds, setPreset, setCustom } = useRange();
  const [from, setFrom] = useState(range.from ?? "");
  const [to, setTo] = useState(range.to ?? "");
  useEffect(() => {
    setFrom(range.from ?? bounds?.minDay ?? "");
    setTo(range.to ?? bounds?.maxDay ?? "");
  }, [range.from, range.to, bounds?.minDay, bounds?.maxDay]);
  return (
    <div className="row" style={{ gap: 8 }}>
      <Segment
        value={preset === "custom" ? "custom" : preset}
        onChange={setPreset}
        options={[
          { value: "7", label: t("range.last7") },
          { value: "28", label: t("range.last28") },
          { value: "90", label: t("range.last90") },
          { value: "all", label: t("range.all") },
        ]}
      />
      <input className="input" type="date" style={{ width: 150 }} value={from} min={bounds?.minDay ?? undefined} max={bounds?.maxDay ?? undefined} aria-label={t("range.from")} onChange={(e) => setFrom(e.target.value)} onBlur={() => from && to && setCustom(from, to)} />
      <span className="muted">→</span>
      <input className="input" type="date" style={{ width: 150 }} value={to} min={bounds?.minDay ?? undefined} max={bounds?.maxDay ?? undefined} aria-label={t("range.to")} onChange={(e) => setTo(e.target.value)} onBlur={() => from && to && setCustom(from, to)} />
    </div>
  );
}

/** Renders children only when a project with imported logs is selected; otherwise a useful empty state. */
export function LogGate({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const nav = useNav();
  const { project, projects, loading } = useProjects();
  const { bounds, boundsError } = useRange();
  if (loading) return <Loading />;
  if (!projects.length || !project) {
    return <Empty title={t("project.emptyTitle")} text={t("project.emptyText")} action={<button type="button" className="btn btn--primary" onClick={() => nav("projects")}>{t("project.new")}</button>} />;
  }
  if (boundsError) return <ErrorAlert error={boundsError} />;
  if (!bounds) return <Loading />;
  if (bounds.files === 0) {
    return <Empty icon="⇪" title={t("log.noData")} text={t("log.noDataText")} action={<button type="button" className="btn btn--primary" onClick={() => nav("imports")}>{t("log.importNow")}</button>} />;
  }
  return <>{children}</>;
}

/** Loads data for the active project and range; reloads after imports. */
export function useLogData<T>(loader: (projectId: number, range: RangeArgs) => Promise<T>, deps: unknown[] = []) {
  const { project } = useProjects();
  const { range, version, bounds } = useRange();
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const ready = !!project && !!bounds && bounds.files > 0;
  useEffect(() => {
    if (!ready || !project) return;
    let alive = true;
    setLoading(true);
    loader(project.id, range)
      .then((d) => alive && (setData(d), setError(null)))
      .catch((e) => alive && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, project?.id, range.from, range.to, version, ...deps]);
  return { data, error, loading };
}

export function CsvButton({ filename, headers, rows }: { filename: string; headers: string[]; rows: (string | number | null | undefined)[][] }) {
  const { t } = useI18n();
  const toast = useToast();
  return (
    <button
      type="button"
      className="btn btn--sm"
      disabled={!rows.length}
      onClick={async () => {
        const path = await save({ defaultPath: filename, filters: [{ name: "CSV", extensions: ["csv"] }] });
        if (!path) return;
        try {
          await api.writeTextFile(path, toCsv(headers, rows), true);
          toast(t("common.fileSaved", { path }), "success");
        } catch (e) {
          toast(e instanceof Error ? e.message : String(e), "error");
        }
      }}
    >
      ⇩ CSV
    </button>
  );
}

export function ReportButton() {
  const { t, lang } = useI18n();
  const toast = useToast();
  const { project } = useProjects();
  const { range } = useRange();
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="btn btn--primary"
      disabled={!project || busy}
      onClick={async () => {
        if (!project) return;
        setBusy(true);
        try {
          const r = await logApi.report(project.id, range, lang);
          toast(t("common.reportSaved", { path: r.path }), "success");
          await openPath(r.path).catch(() => {});
        } catch (e) {
          toast(e instanceof Error ? e.message : String(e), "error");
        } finally {
          setBusy(false);
        }
      }}
    >
      {busy ? <span className="spinner" /> : "▤"} {t("common.exportReport")}
    </button>
  );
}

export function StatusMix({ s2xx, s3xx, s4xx, s5xx }: { s2xx: number; s3xx: number; s4xx: number; s5xx: number }) {
  const { locale } = useI18n();
  const part = (v: number, color: string, label: string) =>
    v > 0 ? (
      <span title={label} style={{ color, fontVariantNumeric: "tabular-nums" }}>
        {formatNumber(v, locale)}
      </span>
    ) : (
      <span className="muted">·</span>
    );
  return (
    <span className="row nowrap" style={{ gap: 8, flexWrap: "nowrap" }}>
      {part(s2xx, "var(--success)", "2xx")}
      {part(s3xx, "var(--info)", "3xx")}
      {part(s4xx, "var(--warning)", "4xx")}
      {part(s5xx, "var(--error)", "5xx")}
    </span>
  );
}

export function statusTone(status: number | string | null | undefined): Tone {
  const s = typeof status === "string" ? Number(status.slice(0, 1)) * 100 : status ?? 0;
  if (s >= 500) return "error";
  if (s >= 400) return "warning";
  if (s >= 300) return "info";
  return "success";
}

export function StatusBadge({ status }: { status: number | string | null | undefined }) {
  return <Badge tone={statusTone(status)}>{status ?? "—"}</Badge>;
}

export function CatBadge({ category }: { category: string }) {
  const { t } = useI18n();
  const tone: Tone = category === "search" ? "gold" : category === "ai_training" ? "high" : category === "ai_assistant" ? "medium" : category === "seo_tool" ? "low" : category === "social" ? "success" : "info";
  return <Badge tone={tone}>{t(`cat.${category}`)}</Badge>;
}

export function VerBadges({ verified, fake, unverifiable }: { verified: number; fake: number; unverifiable: number }) {
  const { t, locale } = useI18n();
  return (
    <span className="row nowrap" style={{ gap: 4, flexWrap: "nowrap" }}>
      {verified > 0 && <Badge tone="success">{t("ver.verified")} {formatNumber(verified, locale)}</Badge>}
      {unverifiable > 0 && <Badge>{t("ver.unverifiable")} {formatNumber(unverifiable, locale)}</Badge>}
      {fake > 0 && <Badge tone="error">{t("ver.fake")} {formatNumber(fake, locale)}</Badge>}
    </span>
  );
}

/** `2026-10-28T14:00:00Z` → local short date-time; `2026-10-28` stays a date. */
export function fmtTs(value: string | null | undefined, locale: string): string {
  if (!value) return "—";
  if (value.length === 10) return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(new Date(`${value}T00:00:00`));
  const d = new Date(value);
  return Number.isNaN(d.getTime()) ? value : new Intl.DateTimeFormat(locale, { dateStyle: "short", timeStyle: "short" }).format(d);
}
