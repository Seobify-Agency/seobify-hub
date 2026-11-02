import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import rocket from "../assets/seobify-rocket.svg";
import { loadImage, pickImageFiles } from "../services/imageService";
import { isSupportedImagePath } from "../hooks/useBatchQueue";
import { useTranslation } from "../i18n";
import type { SourceImage } from "../types";
import "./FileDropZone.css";

interface FileDropZoneProps {
  onImageLoaded: (image: SourceImage) => void;
  onMultipleFiles?: (paths: string[]) => void;
  onError: (error: string) => void;
  hasImage: boolean;
  /** In batch mode every drop (even a single file) goes to the queue. */
  preferBatch?: boolean;
}

export default function FileDropZone({
  onImageLoaded,
  onMultipleFiles,
  onError,
  hasImage,
  preferBatch = false,
}: FileDropZoneProps) {
  const { t } = useTranslation();
  const [isDragging, setIsDragging] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const appWindow = getCurrentWebviewWindow();
    const unlisten = appWindow.onDragDropEvent(async (event) => {
      if (event.payload.type === "over") {
        setIsDragging(true);
      } else if (event.payload.type === "drop") {
        setIsDragging(false);
        const paths = event.payload.paths;
        if (paths.length === 0) return;

        const imagePaths = paths.filter(isSupportedImagePath);

        if (imagePaths.length === 0) {
          onError(t("drop.errorUnsupported"));
          return;
        }

        const rejected = paths.length - imagePaths.length;
        if (rejected > 0) {
          onError(t("drop.errorSkipped", rejected));
        }

        // Multiple files (or batch mode) → batch queue
        if ((imagePaths.length > 1 || preferBatch) && onMultipleFiles) {
          onMultipleFiles(imagePaths);
          return;
        }

        // Single file → single image mode
        setIsLoading(true);
        try {
          const image = await loadImage(imagePaths[0]);
          if (image.fileSize > 50 * 1024 * 1024) {
            onError(t("drop.warningLarge"));
          }
          onImageLoaded(image);
        } catch (err) {
          onError(String(err));
        } finally {
          setIsLoading(false);
        }
      } else if (event.payload.type === "leave") {
        setIsDragging(false);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [onImageLoaded, onMultipleFiles, onError, preferBatch, t]);

  const handleClick = async () => {
    if (isLoading) return;
    const paths = await pickImageFiles();
    if (paths.length === 0) return;

    if ((paths.length > 1 || preferBatch) && onMultipleFiles) {
      onMultipleFiles(paths);
      return;
    }

    setIsLoading(true);
    try {
      const image = await loadImage(paths[0]);
      if (image.fileSize > 50 * 1024 * 1024) {
        onError(t("drop.warningLarge"));
      }
      onImageLoaded(image);
    } catch (err) {
      onError(String(err));
    } finally {
      setIsLoading(false);
    }
  };

  if (hasImage) {
    return isDragging ? (
      <div className="drop-overlay">
        <div className="drop-overlay__card">
          <img src={rocket} alt="" className="drop-overlay__rocket" draggable={false} />
          <span>{preferBatch ? t("drop.overlayBatch") : t("drop.overlaySingle")}</span>
        </div>
      </div>
    ) : null;
  }

  return (
    <div
      ref={dropRef}
      className={`drop-zone ${isDragging ? "drop-zone--active" : ""} ${isLoading ? "drop-zone--loading" : ""}`}
      onClick={handleClick}
      style={{ cursor: isLoading ? "wait" : "pointer" }}
    >
      {isLoading ? (
        <div className="drop-zone__content">
          <div className="spinner" />
          <p>{t("drop.loading")}</p>
        </div>
      ) : (
        <div className="drop-zone__content">
          <div className="drop-zone__icon">
            <img src={rocket} alt="" className="drop-zone__rocket" draggable={false} />
          </div>
          <p className="drop-zone__eyebrow">{t("drop.eyebrow")}</p>
          <p className="drop-zone__text">{t("drop.title")}</p>
          <p className="drop-zone__hint">{t("drop.hint")}</p>
          <ul className="drop-zone__features">
            <li>{t("drop.badgeLossless")}</li>
            <li>{t("drop.badgePrivacy")}</li>
            <li>{t("drop.badgeVitals")}</li>
          </ul>
        </div>
      )}
    </div>
  );
}
