import { useTranslation } from "../i18n";
import {
  SEVERITY_BADGE,
  severityCounts,
  visibleFindings,
} from "../services/privacyService";
import type {
  ImageFormatLabel,
  InspectReport,
  OptimizedImage,
  OutputFormat,
  SanitizeConfig,
  SanitizeMode,
  SsimResult,
} from "../types";
import { DEFAULT_BRAND_STAMP } from "../types";
import "./PrivacyStudio.css";

interface PrivacyStudioProps {
  sanitize: SanitizeConfig;
  onSanitizeChange: (config: SanitizeConfig) => void;
  inspectReport: InspectReport | null;
  isInspecting: boolean;
  optimizedImage: OptimizedImage | null;
  sourceFormat: ImageFormatLabel | null;
  ssimResult?: SsimResult | null;
  isEncoding: boolean;
}

const MODES: SanitizeMode[] = ["off", "safe", "sensor_natural", "studio_master"];
const FORMATS: OutputFormat[] = ["webp", "original", "png", "jpeg"];

export default function PrivacyStudio({
  sanitize,
  onSanitizeChange,
  inspectReport,
  isInspecting,
  optimizedImage,
  sourceFormat,
  ssimResult,
  isEncoding,
}: PrivacyStudioProps) {
  const { t } = useTranslation();
  const findings = visibleFindings(inspectReport);
  const counts = severityCounts(inspectReport);
  const update = (partial: Partial<SanitizeConfig>) =>
    onSanitizeChange({ ...sanitize, ...partial });

  const modeLabel: Record<SanitizeMode, string> = {
    off: t("privacy.modeOff"),
    safe: t("privacy.modeSafe"),
    sensor_natural: t("privacy.modeSensor"),
    studio_master: t("privacy.modeStudio"),
  };
  const modeHint: Record<SanitizeMode, string> = {
    off: t("privacy.hintOff"),
    safe: t("privacy.hintSafe"),
    sensor_natural: t("privacy.hintSensor"),
    studio_master: t("privacy.hintStudio"),
  };
  const formatLabel: Record<OutputFormat, string> = {
    webp: t("privacy.fmtWebp"),
    original: t("privacy.fmtOriginal"),
    png: t("privacy.fmtPng"),
    jpeg: t("privacy.fmtJpeg"),
  };

  const pipeline = optimizedImage?.pipeline ?? null;
  const showVerification = !!optimizedImage && sanitize.mode !== "off";
  const stamp = sanitize.brandStamp;
  const stampVerified =
    optimizedImage?.inspectAfter?.findings.find((f) => f.category === "brand-stamp") ?? null;

  return (
    <div className="privacy-studio">
      <h4 className="privacy-studio__title">🛡️ {t("privacy.title")}</h4>

      {/* Inspection findings */}
      <div className="privacy-studio__section">
        <div className="privacy-studio__label">{t("privacy.findings")}</div>
        {isInspecting ? (
          <div className="privacy-studio__status">
            <div className="spinner spinner--xs" /> {t("privacy.inspecting")}
          </div>
        ) : !inspectReport ? (
          <div className="privacy-studio__status privacy-studio__status--muted">
            {t("privacy.noImage")}
          </div>
        ) : inspectReport.clean ? (
          <div className="privacy-badge privacy-badge--clean">
            {t("privacy.cleanSource")}
          </div>
        ) : (
          <>
            <div className="privacy-studio__counts">
              {counts.critical > 0 && (
                <span className="privacy-badge privacy-badge--critical">
                  {counts.critical} {SEVERITY_BADGE.critical}
                </span>
              )}
              {counts.high > 0 && (
                <span className="privacy-badge privacy-badge--high">
                  {counts.high} {SEVERITY_BADGE.high}
                </span>
              )}
              {counts.medium > 0 && (
                <span className="privacy-badge privacy-badge--medium">
                  {counts.medium} {SEVERITY_BADGE.medium}
                </span>
              )}
              {counts.low > 0 && (
                <span className="privacy-badge privacy-badge--low">
                  {counts.low} {SEVERITY_BADGE.low}
                </span>
              )}
              <span className="privacy-studio__total">
                {t("privacy.traces", inspectReport.traceCount)}
              </span>
            </div>
            <ul className="privacy-findings">
              {findings.map((f, i) => (
                <li
                  key={`${f.location}-${i}`}
                  className={`privacy-finding ${f.removable ? "" : "privacy-finding--inherent"}`}
                  title={`${f.location}\n${f.detail}${f.valuePreview ? `\n→ ${f.valuePreview}` : ""}`}
                >
                  <span className={`privacy-badge privacy-badge--${f.severity}`}>
                    {SEVERITY_BADGE[f.severity]}
                  </span>
                  <span className="privacy-finding__body">
                    <span className="privacy-finding__name">{f.name}</span>
                    {f.valuePreview && (
                      <span className="privacy-finding__preview">
                        {f.valuePreview}
                      </span>
                    )}
                  </span>
                  <span className="privacy-finding__size">
                    {formatBytes(f.sizeBytes)}
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>

      {/* Mode */}
      <div className="privacy-studio__section">
        <div className="privacy-studio__label">{t("privacy.mode")}</div>
        <select
          className="setting-select"
          value={sanitize.mode}
          onChange={(e) => update({ mode: e.target.value as SanitizeMode })}
        >
          {MODES.map((m) => (
            <option key={m} value={m}>
              {modeLabel[m]}
            </option>
          ))}
        </select>
        <div className="setting-hint privacy-studio__hint">
          {modeHint[sanitize.mode]}
        </div>
      </div>

      {/* Corner overlay toggle */}
      <div className="privacy-studio__section">
        <label className="setting-label">
          <span
            className={sanitize.mode !== "studio_master" ? "privacy-studio__dim" : ""}
            title={t("privacy.deblendHint")}
          >
            {t("privacy.deblend")}
          </span>
          <label className="toggle toggle--sm">
            <input
              type="checkbox"
              checked={sanitize.deblendCorner}
              disabled={sanitize.mode !== "studio_master"}
              onChange={(e) => update({ deblendCorner: e.target.checked })}
            />
            <span className="toggle__slider" />
          </label>
        </label>
      </div>

      {/* Output format */}
      <div className="privacy-studio__section">
        <div className="privacy-studio__label">{t("privacy.outputFormat")}</div>
        <select
          className="setting-select"
          value={sanitize.outputFormat}
          onChange={(e) => update({ outputFormat: e.target.value as OutputFormat })}
        >
          {FORMATS.map((f) => (
            <option key={f} value={f}>
              {formatLabel[f]}
              {f === "original" && sourceFormat ? ` (${sourceFormat.toUpperCase()})` : ""}
            </option>
          ))}
        </select>
      </div>

      {/* Corporate copyright & brand stamp */}
      <div className="privacy-studio__section">
        <label className="setting-label">
          <span title={t("privacy.brandHint")}>{t("privacy.brandStamp")}</span>
          <label className="toggle toggle--sm">
            <input
              type="checkbox"
              checked={!!stamp}
              onChange={(e) =>
                update({ brandStamp: e.target.checked ? { ...DEFAULT_BRAND_STAMP } : null })
              }
            />
            <span className="toggle__slider" />
          </label>
        </label>
        {stamp && (
          <div className="privacy-brand">
            <label className="privacy-brand__field">
              <span>{t("privacy.brandArtist")}</span>
              <input
                type="text"
                className="privacy-input"
                value={stamp.artist}
                onChange={(e) => update({ brandStamp: { ...stamp, artist: e.target.value } })}
                spellCheck={false}
              />
            </label>
            <label className="privacy-brand__field">
              <span>{t("privacy.brandCopyright")}</span>
              <input
                type="text"
                className="privacy-input"
                value={stamp.copyright}
                onChange={(e) => update({ brandStamp: { ...stamp, copyright: e.target.value } })}
                spellCheck={false}
              />
            </label>
            <div className="setting-hint privacy-studio__hint">{t("privacy.brandHint")}</div>
          </div>
        )}
      </div>

      {/* Post-process verification */}
      {showVerification && (
        <div className="privacy-studio__section privacy-studio__verify">
          <div className="privacy-studio__label">{t("privacy.verification")}</div>
          {isEncoding ? (
            <div className="privacy-studio__status">
              <div className="spinner spinner--xs" /> {t("privacy.processing")}
            </div>
          ) : optimizedImage.clean ? (
            <div className="privacy-badge privacy-badge--clean">
              {t("privacy.verifiedClean")}
            </div>
          ) : (
            <div className="privacy-badge privacy-badge--warn">
              {t("privacy.remaining", optimizedImage.findingsAfter)}
            </div>
          )}
          {stampVerified && (
            <div className="privacy-badge privacy-badge--stamp" title={stampVerified.detail}>
              🔏 {t("privacy.brandVerified")}: {stampVerified.valuePreview}
            </div>
          )}
          <div className="privacy-stats">
            <div className="privacy-stats__row">
              <span>{t("privacy.tracesBeforeAfter")}</span>
              <span>
                {optimizedImage.findingsBefore} → {optimizedImage.findingsAfter}
              </span>
            </div>
            {ssimResult && (
              <div className="privacy-stats__row">
                <span>{t("privacy.ssimLive")}</span>
                <span className={`ssim-badge ssim-badge--${ssimResult.rating}`}>
                  %{ssimResult.ssimPercent}
                </span>
              </div>
            )}
            {pipeline && (
              <>
                <div className="privacy-stats__row">
                  <span>{t("privacy.ssimSignal")}</span>
                  <span>{(pipeline.ssim * 100).toFixed(2)}%</span>
                </div>
                <div className="privacy-stats__row">
                  <span>{t("privacy.psnr")}</span>
                  <span>{pipeline.psnr.toFixed(1)} dB</span>
                </div>
                <div className="privacy-stats__row">
                  <span>{t("privacy.outputSize")}</span>
                  <span>
                    {pipeline.outputWidth} × {pipeline.outputHeight}
                  </span>
                </div>
                {sanitize.mode === "studio_master" && (
                  <div className="privacy-stats__row">
                    <span>{t("privacy.spectralPeaks")}</span>
                    <span>{pipeline.spectralPeaksAttenuated}</span>
                  </div>
                )}
                {sanitize.deblendCorner && sanitize.mode === "studio_master" && (
                  <div className="privacy-stats__row">
                    <span>{t("privacy.corner")}</span>
                    <span>
                      {pipeline.cornerOverlayDetected
                        ? `${pipeline.cornerOverlayShape} ${pipeline.cornerOverlaySize} +${pipeline.cornerOverlayMargin}px α=${pipeline.cornerOverlayAlpha.toFixed(2)}`
                        : t("privacy.cornerNone")}
                    </span>
                  </div>
                )}
                <div className="privacy-stats__row">
                  <span>{t("privacy.pipelineTime")}</span>
                  <span>{pipeline.durationMs} ms</span>
                </div>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
