import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import rocket from "../assets/seobify-rocket.svg";
import { useTranslation } from "../i18n";
import { SEVERITY_BADGE } from "../services/privacyService";
import {
  cleanFileName,
  docSeverityCounts,
  documentExtension,
  isSupportedDocumentPath,
  pickDocumentFiles,
  planCounts,
  scopeApplies,
  selectDocumentSavePath,
  selectOutputDirectory,
  sortedDocFindings,
} from "../services/documentService";
import type { DocumentQueue } from "../hooks/useDocumentQueue";
import type { DocCleanOptions, DocFinding, DocItem, DocMode, DocScope, Severity } from "../types";
import { DEFAULT_BRAND_STAMP } from "../types";
import "./PrivacyStudio.css";
import "./DocumentsView.css";

interface DocumentsViewProps {
  docs: DocumentQueue;
  options: DocCleanOptions;
  onOptionsChange: (options: DocCleanOptions) => void;
  onError: (message: string) => void;
}

const SEVERITIES: Severity[] = ["critical", "high", "medium", "low"];

export default function DocumentsView({ docs, options, onOptionsChange, onError }: DocumentsViewProps) {
  const { t } = useTranslation();
  const [isDragging, setIsDragging] = useState(false);
  const { items, selected, addFiles } = docs;

  const add = (paths: string[]) => {
    if (paths.length === 0) return;
    const accepted = paths.filter(isSupportedDocumentPath);
    if (accepted.length === 0) {
      onError(t("docs.errorUnsupported"));
      return;
    }
    const skipped = addFiles(paths);
    if (skipped > 0) onError(t("docs.errorSkipped", skipped));
  };

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "over") setIsDragging(true);
      else if (event.payload.type === "leave") setIsDragging(false);
      else if (event.payload.type === "drop") {
        setIsDragging(false);
        add(event.payload.paths);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
    // `add` only closes over these stable callbacks and the current language.
  }, [addFiles, onError, t]);

  const browse = async () => add(await pickDocumentFiles());

  if (items.length === 0) {
    return (
      <div className={`drop-zone ${isDragging ? "drop-zone--active" : ""}`} onClick={browse} style={{ cursor: "pointer" }}>
        <div className="drop-zone__content">
          <div className="drop-zone__icon">
            <img src={rocket} alt="" className="drop-zone__rocket" draggable={false} />
          </div>
          <p className="drop-zone__eyebrow">{t("docs.eyebrow")}</p>
          <p className="drop-zone__text">{t("docs.dropTitle")}</p>
          <p className="drop-zone__hint">{t("docs.dropHint")}</p>
          <ul className="drop-zone__features">
            <li>{t("docs.badgeOffice")}</li>
            <li>{t("docs.badgePdf")}</li>
            <li>{t("docs.badgeReview")}</li>
          </ul>
        </div>
      </div>
    );
  }

  const handleCleanAll = async () => {
    const dir = await selectOutputDirectory();
    if (dir) await docs.cleanAll(options, dir);
  };

  const doneCount = items.filter((i) => i.status === "done").length;
  const cleanCount = items.filter((i) => i.outcome?.clean).length;

  return (
    <div className="docs">
      <aside className="docs__list">
        <div className="docs__list-header">
          <h3 className="docs__list-title">
            {t("docs.listTitle")} <span className="docs__count">{items.length}</span>
          </h3>
          <button type="button" className="btn-primary btn-primary--sm" onClick={handleCleanAll} disabled={docs.isCleaningAll} title={t("docs.cleanAllHint")}>
            {docs.isCleaningAll ? t("docs.cleaning") : t("docs.cleanAll")}
          </button>
        </div>
        {doneCount > 0 && <div className="docs__list-summary">{t("docs.summary", cleanCount, doneCount)}</div>}
        <ul className="docs__items">
          {items.map((item) => (
            <DocRow key={item.id} item={item} selected={item.id === docs.selectedId} onSelect={() => docs.setSelectedId(item.id)} onRemove={() => docs.removeItem(item.id)} />
          ))}
        </ul>
      </aside>

      <main className="docs__main">{selected ? <DocDetail item={selected} options={options} /> : <div className="app__empty">{t("docs.selectDoc")}</div>}</main>

      <DocPanel item={selected} options={options} onOptionsChange={onOptionsChange} docs={docs} />

      {isDragging && (
        <div className="drop-overlay">
          <div className="drop-overlay__card">
            <img src={rocket} alt="" className="drop-overlay__rocket" draggable={false} />
            <span>{t("docs.overlay")}</span>
          </div>
        </div>
      )}
    </div>
  );
}

function StatusBadge({ item }: { item: DocItem }) {
  const { t } = useTranslation();
  if (item.status === "inspecting") return <span className="docs__status docs__status--busy">{t("docs.inspecting")}</span>;
  if (item.status === "cleaning") return <span className="docs__status docs__status--busy">{t("docs.cleaning")}</span>;
  if (item.status === "failed") return <span className="docs__status docs__status--failed">{t("docs.failed")}</span>;
  if (item.outcome) {
    return item.outcome.clean ? (
      <span className="docs__status docs__status--clean">✅ {t("docs.zeroLeft")}</span>
    ) : (
      <span className="docs__status docs__status--warn">⚠ {t("docs.left", item.outcome.tracesAfter)}</span>
    );
  }
  if (item.report?.clean) return <span className="docs__status docs__status--clean">{t("docs.cleanBadge")}</span>;
  return <span className="docs__status">{t("docs.traces", item.report?.traceCount ?? 0)}</span>;
}

function DocRow({ item, selected, onSelect, onRemove }: { item: DocItem; selected: boolean; onSelect: () => void; onRemove: () => void }) {
  const { t } = useTranslation();
  return (
    <li className={`docs__item ${selected ? "docs__item--selected" : ""}`} onClick={onSelect} title={item.error ?? item.filePath}>
      <span className="docs__format">{documentExtension(item.fileName).toUpperCase()}</span>
      <span className="docs__item-body">
        <span className="docs__item-name">{item.fileName}</span>
        <StatusBadge item={item} />
      </span>
      <button
        type="button"
        className="docs__remove"
        title={t("docs.remove")}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
      >
        ✕
      </button>
    </li>
  );
}

function scopeLabel(scope: DocScope, t: (key: string) => string): string {
  return { safe: t("docs.scopeSafe"), strict: t("docs.scopeStrict"), notes: t("docs.scopeNotes"), report: t("docs.scopeReport") }[scope];
}

function FindingRow({ f, options }: { f: DocFinding; options: DocCleanOptions }) {
  const { t } = useTranslation();
  const removes = f.removable && scopeApplies(f.scope, options);
  const state = !f.removable ? t("docs.reportOnly") : removes ? t("docs.willRemove") : t("docs.willKeep");
  return (
    <li className={`docs__finding ${removes ? "docs__finding--removes" : ""} ${f.removable ? "" : "privacy-finding--inherent"}`} title={`${f.location}\n${f.detail}\n${state}`}>
      <span className={`privacy-badge privacy-badge--${f.severity}`}>{SEVERITY_BADGE[f.severity]}</span>
      <span className="privacy-finding__body">
        <span className="privacy-finding__name">{f.name}</span>
        {f.valuePreview && <span className="privacy-finding__preview">{f.valuePreview}</span>}
        <span className="docs__finding-location">{f.location}</span>
      </span>
      <span className={`docs__scope docs__scope--${f.scope}`}>{scopeLabel(f.scope, t)}</span>
    </li>
  );
}

function DocDetail({ item, options }: { item: DocItem; options: DocCleanOptions }) {
  const { t } = useTranslation();
  const report = item.report;
  if (item.status === "inspecting") {
    return (
      <div className="app__empty">
        <div className="spinner" /> {t("docs.inspecting")}
      </div>
    );
  }
  if (!report) {
    return <div className="docs__error">{item.error}</div>;
  }
  const counts = docSeverityCounts(report);
  const plan = planCounts(report, options);
  const outcome = item.outcome;
  return (
    <div className="docs__detail">
      <div className="docs__detail-head">
        <h2 className="docs__detail-title">{item.fileName}</h2>
        <span className="docs__chip">{t(`docs.family.${report.family}`)}</span>
        <span className="docs__chip">{report.format.toUpperCase()}</span>
        <span className="docs__chip">{formatBytes(report.fileSize)}</span>
      </div>

      {outcome && (
        <div className={`docs__result ${outcome.clean ? "docs__result--clean" : "docs__result--warn"}`}>
          <div className="docs__result-line">
            {outcome.clean ? "✅" : "⚠"} {t("docs.removed", outcome.tracesRemoved)} · {outcome.clean ? t("docs.zeroLeft") : t("docs.left", outcome.tracesAfter)}
          </div>
          <div className="docs__result-path" title={outcome.outputPath}>
            {t("docs.savedTo")}: {outcome.outputPath}
          </div>
          {outcome.stampVerified && <div className="privacy-badge privacy-badge--stamp">🔏 {t("docs.stampVerified")}: {outcome.stampVerified}</div>}
          {outcome.warnings.map((w) => (
            <div key={w} className="docs__warning">
              {w}
            </div>
          ))}
          {!outcome.clean && (
            <ul className="privacy-findings docs__findings--compact">
              {sortedDocFindings(outcome.after)
                .filter((f) => f.removable)
                .map((f, i) => (
                  <FindingRow key={`after-${f.location}-${i}`} f={f} options={options} />
                ))}
            </ul>
          )}
        </div>
      )}

      <div className="docs__section-label">
        {t("docs.findings")}
        {report.clean ? null : (
          <span className="docs__counts">
            {SEVERITIES.filter((s) => counts[s] > 0).map((s) => (
              <span key={s} className={`privacy-badge privacy-badge--${s}`}>
                {counts[s]} {SEVERITY_BADGE[s]}
              </span>
            ))}
            <span className="privacy-studio__total">{t("docs.traces", report.traceCount)}</span>
          </span>
        )}
      </div>
      {report.clean && <div className="privacy-badge privacy-badge--clean">{t("docs.noTraces")}</div>}
      {!report.clean && (
        <div className="docs__plan">
          <span>{t("docs.plan", plan.removed)}</span>
          {plan.kept > 0 && <span className="docs__plan-kept">{t("docs.planKept", plan.kept)}</span>}
        </div>
      )}
      <ul className="privacy-findings docs__findings">
        {sortedDocFindings(report).map((f, i) => (
          <FindingRow key={`${f.location}-${i}`} f={f} options={options} />
        ))}
      </ul>
    </div>
  );
}

function DocPanel({
  item,
  options,
  onOptionsChange,
  docs,
}: {
  item: DocItem | null;
  options: DocCleanOptions;
  onOptionsChange: (o: DocCleanOptions) => void;
  docs: DocumentQueue;
}) {
  const { t } = useTranslation();
  const update = (partial: Partial<DocCleanOptions>) => onOptionsChange({ ...options, ...partial });
  const stamp = options.brandStamp;
  const busy = !item || item.status === "inspecting" || item.status === "cleaning" || !item.report;

  const saveAs = async () => {
    if (!item) return;
    const path = await selectDocumentSavePath(cleanFileName(item.fileName), documentExtension(item.fileName));
    if (path) await docs.cleanOne(item.id, options, path);
  };

  return (
    <aside className="docs__panel">
      <div className="privacy-studio">
        <h4 className="privacy-studio__title">🛡️ {t("docs.settings")}</h4>
        <div className="privacy-studio__section">
          <div className="privacy-studio__label">{t("docs.mode")}</div>
          <select className="setting-select" value={options.mode} onChange={(e) => update({ mode: e.target.value as DocMode })}>
            <option value="safe">{t("docs.modeSafe")}</option>
            <option value="strict">{t("docs.modeStrict")}</option>
          </select>
          <div className="setting-hint privacy-studio__hint">{options.mode === "safe" ? t("docs.hintSafe") : t("docs.hintStrict")}</div>
        </div>

        <div className="privacy-studio__section">
          <label className="setting-label docs__toggle-row">
            <span className={options.mode !== "strict" ? "privacy-studio__dim" : ""} title={t("docs.removeNotesHint")}>
              {t("docs.removeNotes")}
            </span>
            <label className="toggle">
              <input type="checkbox" checked={options.removeNotes} disabled={options.mode !== "strict"} onChange={(e) => update({ removeNotes: e.target.checked })} />
              <span className="toggle-slider" />
            </label>
          </label>
          <div className="setting-hint privacy-studio__hint">{t("docs.removeNotesHint")}</div>
        </div>

        <div className="privacy-studio__section">
          <label className="setting-label docs__toggle-row">
            <span title={t("docs.brandHint")}>{t("docs.brandStamp")}</span>
            <label className="toggle">
              <input type="checkbox" checked={!!stamp} onChange={(e) => update({ brandStamp: e.target.checked ? { ...DEFAULT_BRAND_STAMP } : null })} />
              <span className="toggle-slider" />
            </label>
          </label>
          {stamp && (
            <div className="privacy-brand">
              <label className="privacy-brand__field">
                <span>{t("docs.brandArtist")}</span>
                <input type="text" className="privacy-input" value={stamp.artist} spellCheck={false} onChange={(e) => update({ brandStamp: { ...stamp, artist: e.target.value } })} />
              </label>
              <label className="privacy-brand__field">
                <span>{t("docs.brandCopyright")}</span>
                <input type="text" className="privacy-input" value={stamp.copyright} spellCheck={false} onChange={(e) => update({ brandStamp: { ...stamp, copyright: e.target.value } })} />
              </label>
            </div>
          )}
          <div className="setting-hint privacy-studio__hint">{t("docs.brandHint")}</div>
        </div>

        <div className="privacy-studio__section docs__actions">
          <button type="button" className="btn-primary" disabled={busy} onClick={saveAs}>
            {item?.status === "cleaning" ? t("docs.cleaning") : t("docs.saveAs")}
          </button>
          <button type="button" className="btn-secondary" disabled={busy} onClick={() => item && docs.cleanOne(item.id, options)}>
            {t("docs.saveNext")}
          </button>
        </div>
      </div>
    </aside>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
