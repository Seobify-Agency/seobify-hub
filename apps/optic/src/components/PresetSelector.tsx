import type { Preset } from "../types";
import { PRESETS } from "../services/presetService";
import { useTranslation } from "../i18n";
import "./PresetSelector.css";

const PRESET_KEYS: Record<string, { name: string; desc: string }> = {
  "premium-hero": { name: "preset.premiumHero", desc: "preset.premiumHeroDesc" },
  "transparency-safe": { name: "preset.transparencySafe", desc: "preset.transparencySafeDesc" },
  "glow-safe": { name: "preset.glowSafe", desc: "preset.glowSafeDesc" },
  "balanced": { name: "preset.balanced", desc: "preset.balancedDesc" },
  "aggressive": { name: "preset.aggressive", desc: "preset.aggressiveDesc" },
  "near-lossless": { name: "preset.nearLossless", desc: "preset.nearLosslessDesc" },
};

interface PresetSelectorProps {
  activePresetId: string | null;
  onSelect: (preset: Preset) => void;
}

export default function PresetSelector({
  activePresetId,
  onSelect,
}: PresetSelectorProps) {
  const { t } = useTranslation();
  return (
    <div className="preset-selector">
      <h4 className="preset-selector__title">{t("preset.title")}</h4>
      <div className="preset-selector__list">
        {PRESETS.map((preset) => {
          const keys = PRESET_KEYS[preset.id];
          return (
            <button
              key={preset.id}
              className={`preset-card ${activePresetId === preset.id ? "preset-card--active" : ""}`}
              onClick={() => onSelect(preset)}
            >
              <span className="preset-card__name">{keys ? t(keys.name) : preset.name}</span>
              <span className="preset-card__desc">{keys ? t(keys.desc) : preset.description}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
