import { useState, useMemo } from "react";
import type { OptimizedImage, SeoPictureConfig, SourceImage } from "../types";
import {
  generateDefaultVariants,
  generatePictureSnippet,
  calculateWebVitalsImpact,
  toSeoSlug,
} from "../services/seoService";
import { useTranslation } from "../i18n";
import "./SeoDialog.css";

interface SeoDialogProps {
  sourceImage: SourceImage;
  optimizedImage: OptimizedImage | null;
  isOpen: boolean;
  onClose: () => void;
  slug: string;
  onSlugChange: (slug: string) => void;
}

export default function SeoDialog({
  sourceImage,
  optimizedImage,
  isOpen,
  onClose,
  slug,
  onSlugChange,
}: SeoDialogProps) {
  const { t } = useTranslation();
  const [altText, setAltText] = useState(
    sourceImage.fileName.replace(/\.(png|jpg|jpeg|webp)$/i, "")
  );
  const [isLcp, setIsLcp] = useState(true);
  const [copiedCode, setCopiedCode] = useState(false);
  const [copiedPreload, setCopiedPreload] = useState(false);

  const initialVariants = useMemo(
    () => generateDefaultVariants(sourceImage.width, sourceImage.height),
    [sourceImage.width, sourceImage.height]
  );
  const [variants, setVariants] = useState(initialVariants);

  const config: SeoPictureConfig = useMemo(
    () => ({
      altText,
      isLcp,
      loading: isLcp ? "eager" : "lazy",
      decoding: "async",
      fetchPriority: isLcp ? "high" : "auto",
      variants,
    }),
    [altText, isLcp, variants]
  );

  const baseName = slug || toSeoSlug(sourceImage.fileName);
  const { pictureHtml, preloadTag } = useMemo(
    () =>
      generatePictureSnippet(
        baseName,
        sourceImage.width,
        sourceImage.height,
        config
      ),
    [baseName, sourceImage.width, sourceImage.height, config]
  );

  const vitals = useMemo(
    () =>
      calculateWebVitalsImpact(
        sourceImage.fileSize,
        optimizedImage?.fileSize ?? sourceImage.fileSize
      ),
    [sourceImage.fileSize, optimizedImage?.fileSize]
  );

  if (!isOpen) return null;

  const toggleVariant = (index: number) => {
    setVariants((prev) =>
      prev.map((v, i) => (i === index ? { ...v, enabled: !v.enabled } : v))
    );
  };

  const handleCopyCode = async () => {
    await navigator.clipboard.writeText(pictureHtml);
    setCopiedCode(true);
    setTimeout(() => setCopiedCode(false), 2500);
  };

  const handleCopyPreload = async () => {
    if (!preloadTag) return;
    await navigator.clipboard.writeText(preloadTag);
    setCopiedPreload(true);
    setTimeout(() => setCopiedPreload(false), 2500);
  };

  return (
    <div className="seo-dialog-backdrop" onClick={onClose}>
      <div className="seo-dialog" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="seo-dialog__header">
          <div className="seo-dialog__title-box">
            <span className="seo-dialog__badge">Core Web Vitals & SEO</span>
            <h3 className="seo-dialog__title">
              {t("seo.title") || "Responsive <picture> & SEO Kod Üreteci"}
            </h3>
          </div>
          <button className="seo-dialog__close" onClick={onClose}>
            ✕
          </button>
        </div>

        {/* Core Web Vitals Impact Card */}
        <div className="seo-vitals-card">
          <div className="seo-vitals-card__item">
            <span className="seo-vitals-card__label">Tasarruf Edilen Boyut</span>
            <span className="seo-vitals-card__val text-emerald">
              {vitals.savedKb} KB (%{vitals.savedPercent})
            </span>
          </div>
          <div className="seo-vitals-card__item">
            <span className="seo-vitals-card__label">4G İndirme Hızı</span>
            <span className="seo-vitals-card__val">
              ~{vitals.downloadTime4gMs}ms (-{vitals.savedTime4gMs}ms)
            </span>
          </div>
          <div className="seo-vitals-card__item">
            <span className="seo-vitals-card__label">3G İndirme Hızı</span>
            <span className="seo-vitals-card__val">
              ~{vitals.downloadTime3gMs}ms (-{vitals.savedTime3gMs}ms)
            </span>
          </div>
          <div className="seo-vitals-card__item">
            <span className="seo-vitals-card__label">LCP Etkisi</span>
            <span className="seo-vitals-card__badge-lcp">
              🟢 Mükemmel (LCP Dostu)
            </span>
          </div>
        </div>

        {/* Configuration row */}
        <div className="seo-dialog__settings">
          <div className="seo-dialog__field">
            <label>Görsel Açıklaması (alt tag):</label>
            <input
              type="text"
              value={altText}
              onChange={(e) => setAltText(e.target.value)}
              placeholder="Görsel içeriğini tanımlayın..."
              className="seo-input"
            />
          </div>

          <div className="seo-dialog__field">
            <label>{t("seo.slugLabel")}</label>
            <div className="seo-slug-row">
              <input
                type="text"
                value={slug}
                onChange={(e) => onSlugChange(e.target.value)}
                onBlur={() => onSlugChange(toSeoSlug(slug || sourceImage.fileName))}
                className="seo-input"
                spellCheck={false}
              />
              <button
                type="button"
                className="btn-secondary btn-slug-from-alt"
                onClick={() => onSlugChange(toSeoSlug(altText || sourceImage.fileName))}
                disabled={!altText.trim()}
                title={t("seo.slugFromAltHint")}
              >
                ✦ {t("seo.slugFromAlt")}
              </button>
            </div>
            <small className="seo-dialog__slug-preview">
              {baseName}.webp
            </small>
          </div>

          <div className="seo-dialog__lcp-toggle">
            <label className="toggle toggle--sm">
              <input
                type="checkbox"
                checked={isLcp}
                onChange={(e) => setIsLcp(e.target.checked)}
              />
              <span className="toggle__slider" />
            </label>
            <div className="seo-dialog__lcp-text">
              <strong>LCP / Hero Görseli</strong>
              <span>
                (Sayfa üstü ana görsel: fetchpriority="high" ve eager loading
                ekler)
              </span>
            </div>
          </div>
        </div>

        {/* Breakpoints checklist */}
        <div className="seo-dialog__breakpoints">
          <span className="seo-dialog__section-label">
            Responsive Boyutlar (srcset):
          </span>
          <div className="seo-dialog__pills">
            {variants.map((v, i) => (
              <button
                key={v.label}
                type="button"
                className={`seo-pill ${v.enabled ? "seo-pill--active" : ""}`}
                onClick={() => toggleVariant(i)}
              >
                <span>{v.label}</span>
                <small>
                  {v.width}×{v.height}
                </small>
              </button>
            ))}
          </div>
        </div>

        {/* Generated HTML Code */}
        <div className="seo-dialog__code-section">
          <div className="seo-dialog__code-header">
            <span>HTML5 &lt;picture&gt; Kodu:</span>
            <button className="btn-copy" onClick={handleCopyCode}>
              {copiedCode ? "✓ Kopyalandı!" : "📋 Kodu Kopyala"}
            </button>
          </div>
          <pre className="seo-code-box">
            <code>{pictureHtml}</code>
          </pre>
        </div>

        {/* Optional Preload Snippet for LCP */}
        {preloadTag && (
          <div className="seo-dialog__code-section">
            <div className="seo-dialog__code-header">
              <span>&lt;head&gt; İçin Preload Etiketi (En Hızlı LCP İçin):</span>
              <button className="btn-copy btn-copy--sm" onClick={handleCopyPreload}>
                {copiedPreload ? "✓ Kopyalandı!" : "📋 Kopyala"}
              </button>
            </div>
            <pre className="seo-code-box seo-code-box--sm">
              <code>{preloadTag}</code>
            </pre>
          </div>
        )}

        {/* Footer */}
        <div className="seo-dialog__footer">
          <span className="seo-dialog__hint">
            💡 Görselleri dışa aktardıktan sonra bu HTML kodunu web sayfanıza
            doğrudan yapıştırabilirsiniz.
          </span>
          <button className="btn-primary" onClick={onClose}>
            Kapat
          </button>
        </div>
      </div>
    </div>
  );
}
