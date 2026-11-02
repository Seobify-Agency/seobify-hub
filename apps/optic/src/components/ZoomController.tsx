import { useTranslation } from "../i18n";
import "./ZoomController.css";

interface ZoomControllerProps {
  zoom: number;
  zoomLevels: number[];
  onZoomChange: (level: number) => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetPan: () => void;
}

export default function ZoomController({
  zoom,
  zoomLevels,
  onZoomChange,
  onZoomIn,
  onZoomOut,
  onResetPan,
}: ZoomControllerProps) {
  const { t } = useTranslation();
  const options = zoomLevels.some((l) => Math.abs(l - zoom) < 1e-6)
    ? zoomLevels
    : [...zoomLevels, zoom].sort((a, b) => a - b);
  return (
    <div className="zoom-controller">
      <button className="zoom-btn" onClick={onZoomOut} title={t("zoom.zoomOut")}>
        −
      </button>
      <select
        className="zoom-select"
        value={zoom}
        onChange={(e) => onZoomChange(Number(e.target.value))}
      >
        {options.map((level) => (
          <option key={level} value={level}>
            {Math.round(level * 100)}%
          </option>
        ))}
      </select>
      <button className="zoom-btn" onClick={onZoomIn} title={t("zoom.zoomIn")}>
        +
      </button>
      <button
        className="zoom-btn"
        onClick={() => {
          onZoomChange(1);
          onResetPan();
        }}
        title={t("zoom.resetPan")}
      >
        ⌂
      </button>
      <span className="zoom-hint" title={t("zoom.wheelHint")}>
        {t("zoom.wheelHintShort")}
      </span>
    </div>
  );
}
