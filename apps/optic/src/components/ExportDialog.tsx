import { useEffect, useState } from "react";
import { exportImage, selectSavePath } from "../services/encodeService";
import { outputExtension } from "../services/privacyService";
import { toSeoSlug } from "../services/seoService";
import { useTranslation } from "../i18n";
import type { OptimizedImage, SourceImage } from "../types";
import SeoDialog from "./SeoDialog";
import "./ExportDialog.css";

interface ExportDialogProps {
  sourceImage?: SourceImage | null;
  optimizedImage: OptimizedImage | null;
  defaultFileName: string;
}

export default function ExportDialog({
  sourceImage,
  optimizedImage,
  defaultFileName,
}: ExportDialogProps) {
  const [isExporting, setIsExporting] = useState(false);
  const [exportResult, setExportResult] = useState<string | null>(null);
  const [exportVerdict, setExportVerdict] = useState<string | null>(null);
  const [isSeoOpen, setIsSeoOpen] = useState(false);
  const [slug, setSlug] = useState(() => toSeoSlug(defaultFileName));
  const { t } = useTranslation();

  useEffect(() => {
    setSlug(toSeoSlug(defaultFileName));
  }, [defaultFileName]);

  const effectiveSlug = toSeoSlug(slug || defaultFileName);

  const handleExport = async () => {
    if (!optimizedImage) return;

    const ext = outputExtension(optimizedImage.outputFormat, optimizedImage.outputFormat);
    const outName = `${effectiveSlug}.${ext}`;
    const result = await selectSavePath(outName, ext);
    if (!result?.path) return;

    setIsExporting(true);
    setExportResult(null);
    setExportVerdict(null);
    try {
      const exported = await exportImage(optimizedImage.tempFilePath, result.path);
      setExportResult(t("export.success"));
      setExportVerdict(
        exported.clean
          ? t("export.verifiedClean")
          : t("export.remaining", exported.findingsAfter)
      );
      setTimeout(() => {
        setExportResult(null);
        setExportVerdict(null);
      }, 6000);
    } catch (err) {
      setExportResult(`${t("export.failed")} ${err}`);
    } finally {
      setIsExporting(false);
    }
  };

  return (
    <>
      <div className="export-bar">
        <button
          className="btn-primary"
          disabled={!optimizedImage || isExporting}
          onClick={handleExport}
        >
          {isExporting ? t("export.exporting") : optimizedImage ? `${t("export.buttonFormat")} ${optimizedImage.outputFormat.toUpperCase()}` : t("export.button")}
        </button>

        <label className="export-bar__slug" title={t("export.slugHint")}>
          <span className="export-bar__slug-label">{t("export.slugLabel")}</span>
          <input
            type="text"
            className="export-bar__slug-input"
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
            onBlur={() => setSlug(effectiveSlug)}
            spellCheck={false}
          />
          <span className="export-bar__slug-ext">
            .{optimizedImage ? outputExtension(optimizedImage.outputFormat, optimizedImage.outputFormat) : "webp"}
          </span>
          <button
            type="button"
            className="export-bar__slug-reset"
            onClick={() => setSlug(toSeoSlug(defaultFileName))}
            title={t("export.slugReset")}
          >
            ↺
          </button>
        </label>

        {sourceImage && (
          <button
            type="button"
            className="btn-seo"
            disabled={!optimizedImage}
            onClick={() => setIsSeoOpen(true)}
            title="Responsive HTML5 <picture> etiketleri ve Core Web Vitals metriklerini görüntüle"
          >
            ⚡ {t("export.seoButton") || "SEO & HTML Kodu"}
          </button>
        )}

        {exportResult && (
          <span
            className={`export-bar__result ${exportResult.startsWith(t("export.failed")) ? "export-bar__result--error" : ""}`}
          >
            {exportResult}
            {exportVerdict && <> · {exportVerdict}</>}
          </span>
        )}
      </div>

      {sourceImage && (
        <SeoDialog
          sourceImage={sourceImage}
          optimizedImage={optimizedImage}
          isOpen={isSeoOpen}
          onClose={() => setIsSeoOpen(false)}
          slug={effectiveSlug}
          onSlugChange={setSlug}
        />
      )}
    </>
  );
}
