import type {
  EncodingConfig,
  MetadataOption,
  BackgroundMode,
  InspectReport,
  OptimizedImage,
  SanitizeConfig,
  SourceImage,
  Preset,
  SsimResult,
} from "../types";
import { useTranslation } from "../i18n";
import PresetSelector from "./PresetSelector";
import PrivacyStudio from "./PrivacyStudio";
import "./SettingsPanel.css";

interface SettingsPanelProps {
  config: EncodingConfig;
  onConfigChange: (config: EncodingConfig) => void;
  sourceImage: SourceImage | null;
  optimizedImage: OptimizedImage | null;
  backgroundMode: BackgroundMode;
  onBackgroundModeChange: (mode: BackgroundMode) => void;
  isEncoding: boolean;
  activePresetId: string | null;
  onPresetSelect: (preset: Preset) => void;
  ssimResult?: SsimResult | null;
  onAutoCalibrate?: () => Promise<void> | void;
  isCalibrating?: boolean;
  sanitize: SanitizeConfig;
  onSanitizeChange: (config: SanitizeConfig) => void;
  inspectReport: InspectReport | null;
  isInspecting: boolean;
}

export default function SettingsPanel({
  config,
  onConfigChange,
  sourceImage,
  optimizedImage,
  backgroundMode,
  onBackgroundModeChange,
  isEncoding,
  activePresetId,
  onPresetSelect,
  ssimResult,
  onAutoCalibrate,
  isCalibrating,
  sanitize,
  onSanitizeChange,
  inspectReport,
  isInspecting,
}: SettingsPanelProps) {
  const updateConfig = (partial: Partial<EncodingConfig>) => {
    const updated = { ...config, ...partial };
    if (!updated.lossless) {
      updated.nearLossless = null;
    }
    onConfigChange(updated);
  };

  const { t } = useTranslation();

  return (
    <div className="settings-panel">
      <h3 className="settings-panel__title">{t("settings.title")}</h3>

      <PresetSelector
        activePresetId={activePresetId}
        onSelect={onPresetSelect}
      />

      {/* Quality Slider */}
      <div className="setting-group">
        <label className="setting-label">
          <span>{t("settings.quality")}</span>
          <span className="setting-value">{config.quality}</span>
        </label>
        <input
          type="range"
          min={0}
          max={100}
          value={config.quality}
          onChange={(e) => updateConfig({ quality: Number(e.target.value) })}
          disabled={config.lossless || isCalibrating}
          className="setting-slider"
        />
        <div className="setting-hint">cwebp -q {config.quality}</div>

        {/* Zero-Loss Auto Calibration button */}
        {onAutoCalibrate && !config.lossless && (
          <div className="setting-sub-action">
            <button
              type="button"
              className="btn-auto-calibrate"
              disabled={isCalibrating || !sourceImage || isEncoding}
              onClick={onAutoCalibrate}
              title="İnsan gözünün ayırt edemeyeceği en düşük dosya boyutunu otomatik bulur (%99.2+ SSIM)"
            >
              {isCalibrating ? (
                <>
                  <div className="spinner spinner--xs" />
                  <span>Kalibre ediliyor...</span>
                </>
              ) : (
                <>⚡ Otomatik Kalibre Et (Zero-Loss)</>
              )}
            </button>
          </div>
        )}
      </div>

      {/* Alpha Quality Slider */}
      <div className="setting-group">
        <label className="setting-label">
          <span>{t("settings.alphaQuality")}</span>
          <span className="setting-value">{config.alphaQuality}</span>
        </label>
        <input
          type="range"
          min={0}
          max={100}
          value={config.alphaQuality}
          onChange={(e) =>
            updateConfig({ alphaQuality: Number(e.target.value) })
          }
          className="setting-slider"
        />
        <div className="setting-hint">cwebp -alpha_q {config.alphaQuality}</div>
      </div>

      {/* Method Slider */}
      <div className="setting-group">
        <label className="setting-label">
          <span>{t("settings.method")}</span>
          <span className="setting-value">{config.method}</span>
        </label>
        <input
          type="range"
          min={0}
          max={6}
          value={config.method}
          onChange={(e) => updateConfig({ method: Number(e.target.value) })}
          className="setting-slider"
        />
        <div className="setting-hint">
          cwebp -m {config.method} {t("settings.methodHint")}
        </div>
      </div>

      {/* Lossless Toggle */}
      <div className="setting-group">
        <label className="setting-label">
          <span>{t("settings.lossless")}</span>
          <label className="toggle">
            <input
              type="checkbox"
              checked={config.lossless}
              onChange={(e) => updateConfig({ lossless: e.target.checked })}
            />
            <span className="toggle__slider" />
          </label>
        </label>
      </div>

      {/* Near Lossless (only when lossless is on) */}
      {config.lossless && (
        <div className="setting-group">
          <label className="setting-label">
            <span>{t("settings.nearLossless")}</span>
            <span className="setting-value">
              {config.nearLossless ?? t("settings.off")}
            </span>
          </label>
          <div className="setting-row">
            <label className="toggle toggle--sm">
              <input
                type="checkbox"
                checked={config.nearLossless !== null}
                onChange={(e) =>
                  updateConfig({
                    nearLossless: e.target.checked ? 60 : null,
                  })
                }
              />
              <span className="toggle__slider" />
            </label>
            {config.nearLossless !== null && (
              <input
                type="range"
                min={0}
                max={100}
                value={config.nearLossless}
                onChange={(e) =>
                  updateConfig({ nearLossless: Number(e.target.value) })
                }
                className="setting-slider"
              />
            )}
          </div>
        </div>
      )}

      {/* Metadata */}
      <div className="setting-group">
        <label className="setting-label">{t("settings.metadata")}</label>
        <select
          value={config.metadata}
          onChange={(e) =>
            updateConfig({ metadata: e.target.value as MetadataOption })
          }
          className="setting-select"
        >
          <option value="none">{t("settings.metaNone")}</option>
          <option value="all">{t("settings.metaAll")}</option>
          <option value="exif">{t("settings.metaExif")}</option>
          <option value="icc">{t("settings.metaIcc")}</option>
          <option value="xmp">{t("settings.metaXmp")}</option>
        </select>
      </div>

      {/* Privacy & DSP Signal Studio */}
      <PrivacyStudio
        sanitize={sanitize}
        onSanitizeChange={onSanitizeChange}
        inspectReport={inspectReport}
        isInspecting={isInspecting}
        optimizedImage={optimizedImage}
        sourceFormat={sourceImage?.format ?? null}
        ssimResult={ssimResult}
        isEncoding={isEncoding}
      />

      {/* Background Mode */}
      <div className="setting-group">
        <label className="setting-label">{t("settings.background")}</label>
        <div className="setting-buttons">
          {(
            [
              ["checkerboard", t("settings.checker") || "Dama"],
              ["dark", t("settings.dark") || "Koyu"],
              ["light", t("settings.light") || "Açık"],
              ["navy", "Lacivert"],
              ["purple", "Mor"],
              ["web-mockup", "🌐 Mockup"],
            ] as [BackgroundMode, string][]
          ).map(([mode, label]) => (
            <button
              key={mode}
              className={`btn-sm ${backgroundMode === mode ? "btn-sm--active" : ""}`}
              onClick={() => onBackgroundModeChange(mode)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      {/* File Size Info */}
      {sourceImage && (
        <div className="file-info">
          <h4 className="file-info__title">{t("settings.fileInfo")}</h4>
          <div className="file-info__row">
            <span>{t("settings.original")}</span>
            <span>{formatFileSize(sourceImage.fileSize)}</span>
          </div>
          <div className="file-info__row">
            <span>{t("settings.dimensions")}</span>
            <span>
              {sourceImage.width} × {sourceImage.height}
            </span>
          </div>
          <div className="file-info__row">
            <span>{t("settings.alpha")}</span>
            <span>{sourceImage.hasAlpha ? t("settings.yes") : t("settings.no")}</span>
          </div>
          <div className="file-info__row">
            <span>{t("settings.format")}</span>
            <span>{sourceImage.format.toUpperCase()}</span>
          </div>
          {optimizedImage && (
            <>
              <div className="file-info__divider" />
              <div className="file-info__row">
                <span>{t("settings.output")}</span>
                <span>
                  {formatFileSize(optimizedImage.fileSize)} ·{" "}
                  {optimizedImage.outputFormat.toUpperCase()}
                </span>
              </div>
              <div className="file-info__row file-info__row--highlight">
                <span>{t("settings.reduction")}</span>
                <span>{optimizedImage.reductionPercent.toFixed(1)}%</span>
              </div>
              <div className="file-info__row">
                <span>{t("settings.encodeTime")}</span>
                <span>{optimizedImage.encodingDurationMs}ms</span>
              </div>

              {/* SSIM Live Metric */}
              {ssimResult && (
                <div className="file-info__row">
                  <span>SSIM Benzerlik</span>
                  <span
                    className={`ssim-badge ssim-badge--${ssimResult.rating}`}
                    title="Piksel Yapısal Benzerlik İndeksi (SSIM)"
                  >
                    %{ssimResult.ssimPercent} (
                    {ssimResult.rating === "imperceptible"
                      ? "Kusursuz"
                      : ssimResult.rating === "excellent"
                      ? "Mükemmel"
                      : ssimResult.rating === "good"
                      ? "İyi"
                      : "Fark Edilir"}
                    )
                  </span>
                </div>
              )}
            </>
          )}
          {isEncoding && (
            <div className="file-info__encoding">
              <div className="spinner spinner--sm" /> {t("settings.encoding")}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
