// Shared "Import URLs" dialog: Seobify Spider crawl, local file, sitemap URL or pasted list.
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import type { ImportPreview, ImportSource, SpiderCrawl, SpiderProject } from "../lib/api";
import { formatDate, formatNumber } from "../lib/format";
import { ErrorAlert, Field, Loading, Modal, Segment } from "./ui";

type Kind = ImportSource["kind"];

export default function ImportDialog({ title, defaultSpiderProject, confirmLabel, onClose, onConfirm }: {
  title?: string;
  defaultSpiderProject?: string | null;
  confirmLabel?: string;
  onClose: () => void;
  /** Called with the chosen source after a successful preview. */
  onConfirm: (source: ImportSource, preview: ImportPreview) => void | Promise<void>;
}) {
  const { t, locale } = useI18n();
  const [kind, setKind] = useState<Kind>("spider");
  const [spiderSource, setSpiderSource] = useState<"api" | "files" | null>(null);
  const [projects, setProjects] = useState<SpiderProject[]>([]);
  const [spiderProject, setSpiderProject] = useState(defaultSpiderProject ?? "");
  const [crawls, setCrawls] = useState<SpiderCrawl[]>([]);
  const [crawlId, setCrawlId] = useState<number | null>(null);
  const [filePath, setFilePath] = useState("");
  const [sitemap, setSitemap] = useState("");
  const [list, setList] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.spiderProjects()
      .then((r) => {
        setProjects(r.projects);
        setSpiderSource(r.source);
        if (!spiderProject && r.projects.length) setSpiderProject(r.projects.find((p) => p.name === "default")?.name ?? r.projects[0].name);
      })
      .catch((e) => setError(String(e instanceof Error ? e.message : e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!spiderProject) return;
    setCrawls([]);
    setCrawlId(null);
    api.spiderCrawls(spiderProject).then((r) => setCrawls(r.crawls)).catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }, [spiderProject]);

  const source = (): ImportSource | null => {
    switch (kind) {
      case "spider":
        return spiderProject ? { kind, project: spiderProject, crawlId } : null;
      case "file":
        return filePath ? { kind, path: filePath } : null;
      case "sitemap":
        return sitemap.trim() ? { kind, url: sitemap.trim() } : null;
      case "list":
        return list.trim() ? { kind, urls: list.split(/\r?\n/) } : null;
    }
  };

  const runPreview = async () => {
    const s = source();
    if (!s) return;
    setBusy(true);
    setError(null);
    setPreview(null);
    try {
      setPreview(await api.importPreview(s, 8));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const confirm = async () => {
    const s = source();
    if (!s || !preview) return;
    setBusy(true);
    try {
      await onConfirm(s, preview);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={title ?? t("import.title")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
          <button type="button" className="btn" disabled={busy || !source()} onClick={runPreview}>{t("import.preview")}</button>
          <button type="button" className="btn btn--primary" disabled={busy || !preview || preview.total === 0} onClick={confirm}>
            {confirmLabel ?? t("common.import")}
          </button>
        </>
      }
    >
      <Segment<Kind>
        value={kind}
        onChange={(k) => { setKind(k); setPreview(null); setError(null); }}
        options={[
          { value: "spider", label: t("import.fromSpider") },
          { value: "file", label: t("import.fromFile") },
          { value: "sitemap", label: t("import.fromSitemap") },
          { value: "list", label: t("import.fromList") },
        ]}
      />
      {kind === "spider" && (
        <div className="grid grid--2">
          <Field label={t("import.spiderProject")} hint={spiderSource === "api" ? t("import.spiderOnline") : spiderSource === "files" ? t("import.spiderOffline") : undefined}>
            <select className="select" value={spiderProject} onChange={(e) => setSpiderProject(e.target.value)}>
              {projects.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
            </select>
          </Field>
          <Field label={t("import.crawl")}>
            <select className="select" value={crawlId ?? ""} onChange={(e) => setCrawlId(e.target.value ? Number(e.target.value) : null)}>
              <option value="">{t("import.latestCrawl")}</option>
              {crawls.map((c) => (
                <option key={c.id} value={c.id}>
                  #{c.id} · {c.startUrl ?? c.name ?? "?"} · {formatNumber(c.urlsDone ?? 0, locale)} URL · {c.status} · {formatDate(c.createdAt, locale, false)}
                </option>
              ))}
            </select>
          </Field>
        </div>
      )}
      {kind === "file" && (
        <div className="row">
          <input className="input" style={{ flex: 1 }} value={filePath} readOnly placeholder="C:\\…\\urls.csv" />
          <button type="button" className="btn" onClick={async () => {
            const picked = await open({ multiple: false, filters: [{ name: "URL list", extensions: ["csv", "tsv", "txt", "xml", "gz"] }] });
            if (typeof picked === "string") { setFilePath(picked); setPreview(null); }
          }}>{t("import.chooseFile")}</button>
        </div>
      )}
      {kind === "sitemap" && <input className="input" value={sitemap} placeholder={t("import.sitemapPlaceholder")} onChange={(e) => setSitemap(e.target.value)} />}
      {kind === "list" && <textarea className="textarea" rows={8} value={list} placeholder={t("import.listPlaceholder")} onChange={(e) => setList(e.target.value)} />}
      <ErrorAlert error={error} />
      {busy && <Loading />}
      {preview && (
        <div className="field">
          <div className="alert alert--info">{t("import.found", { count: formatNumber(preview.total, locale), source: preview.source })}</div>
          <pre className="code">{preview.preview.map((u) => `${u.statusCode ?? ""}\t${u.url}${u.title ? `\t${u.title}` : ""}`).join("\n")}</pre>
        </div>
      )}
    </Modal>
  );
}
