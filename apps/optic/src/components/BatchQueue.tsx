import type { BatchQueueItem, EncodingConfig } from "../types";
import { useTranslation } from "../i18n";
import "./BatchQueue.css";

interface BatchQueueProps {
  queue: BatchQueueItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onRemove: (id: string) => void;
  onMove: (id: string, direction: "up" | "down") => void;
  onExportAll: (config: EncodingConfig) => void;
  config: EncodingConfig;
  isExporting: boolean;
  seoSlug: boolean;
  onSeoSlugChange: (enabled: boolean) => void;
}

export default function BatchQueue({
  queue,
  selectedId,
  onSelect,
  onRemove,
  onMove,
  onExportAll,
  config,
  isExporting,
  seoSlug,
  onSeoSlugChange,
}: BatchQueueProps) {
  const { t } = useTranslation();

  if (queue.length === 0) return null;

  const statusIcon = (status: string) => {
    switch (status) {
      case "completed":
        return "✓";
      case "failed":
        return "✗";
      case "processing":
        return "⟳";
      case "loading":
        return "…";
      default:
        return "○";
    }
  };

  return (
    <div className="batch-queue">
      <div className="batch-queue__header">
        <h3 className="batch-queue__title">
          {t("batch.title")} ({queue.length})
        </h3>
        <button
          className="btn-primary btn-primary--sm"
          onClick={() => onExportAll(config)}
          disabled={isExporting || queue.length === 0}
        >
          {isExporting ? t("export.exporting") : t("batch.exportAll")}
        </button>
      </div>

      <label className="batch-queue__option" title={t("batch.seoSlugHint")}>
        <span>{t("batch.seoSlug")}</span>
        <label className="toggle toggle--sm">
          <input
            type="checkbox"
            checked={seoSlug}
            onChange={(e) => onSeoSlugChange(e.target.checked)}
          />
          <span className="toggle__slider" />
        </label>
      </label>

      <div className="batch-queue__list">
        {queue.map((item) => (
          <div
            key={item.id}
            className={`batch-queue__item ${selectedId === item.id ? "batch-queue__item--selected" : ""} batch-queue__item--${item.status}`}
            onClick={() => onSelect(item.id)}
          >
            <span
              className={`batch-queue__status badge-${item.status}`}
            >
              {statusIcon(item.status)}
            </span>
            <div className="batch-queue__info">
              <span className="batch-queue__name">{item.fileName}</span>
              <span className="batch-queue__size">
                {item.sourceImage
                  ? formatFileSize(item.sourceImage.fileSize)
                  : item.status === "failed"
                    ? t("batch.loadFailed")
                    : t("batch.loading")}
              </span>
            </div>
            <div className="batch-queue__actions">
              <button
                className="batch-queue__btn"
                onClick={(e) => {
                  e.stopPropagation();
                  onMove(item.id, "up");
                }}
                title={t("batch.moveUp")}
              >
                ↑
              </button>
              <button
                className="batch-queue__btn"
                onClick={(e) => {
                  e.stopPropagation();
                  onMove(item.id, "down");
                }}
                title={t("batch.moveDown")}
              >
                ↓
              </button>
              <button
                className="batch-queue__btn batch-queue__btn--remove"
                onClick={(e) => {
                  e.stopPropagation();
                  onRemove(item.id);
                }}
                title={t("batch.remove")}
              >
                ✕
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
