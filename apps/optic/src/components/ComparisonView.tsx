import { useCallback, useRef, useState, useEffect } from "react";
import type {
  BackgroundMode,
  ComparisonMode,
  InspectionMode,
  BandingRisk,
  EncodingConfig,
} from "../types";
import { useTranslation } from "../i18n";
import InspectionOverlay from "./InspectionOverlay";
import "./ComparisonView.css";

interface ComparisonViewProps {
  originalData: string | null;
  optimizedData: string | null;
  originalMimeType?: string;
  optimizedMimeType?: string;
  backgroundMode: BackgroundMode;
  onBackgroundModeChange?: (mode: BackgroundMode) => void;
  comparisonMode: ComparisonMode;
  onComparisonModeChange: (mode: ComparisonMode) => void;
  inspectionMode: InspectionMode;
  onInspectionModeChange: (mode: InspectionMode) => void;
  zoom: number;
  panX: number;
  panY: number;
  onPan: (dx: number, dy: number) => void;
  onZoomAt: (factor: number, cx: number, cy: number) => void;
  imageWidth: number;
  imageHeight: number;
  isEncoding: boolean;
  hasAlpha?: boolean;
  bandingRisk?: BandingRisk | null;
  onApplyRecommendation?: (rec: Partial<EncodingConfig>) => void;
}

export default function ComparisonView({
  originalData,
  optimizedData,
  originalMimeType = "image/png",
  optimizedMimeType = "image/webp",
  backgroundMode,
  onBackgroundModeChange,
  comparisonMode,
  onComparisonModeChange,
  inspectionMode,
  onInspectionModeChange,
  zoom,
  panX,
  panY,
  onPan,
  onZoomAt,
  imageWidth,
  imageHeight,
  isEncoding,
  hasAlpha,
  bandingRisk,
  onApplyRecommendation,
}: ComparisonViewProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const [splitPos, setSplitPos] = useState(0.5);
  const [isDragging, setIsDragging] = useState(false);
  const [isPanning, setIsPanning] = useState(false);
  const lastPanPos = useRef({ x: 0, y: 0 });
  const originalCanvasRef = useRef<HTMLCanvasElement>(null);
  const optimizedCanvasRef = useRef<HTMLCanvasElement>(null);

  const bgClass =
    backgroundMode === "checkerboard"
      ? "bg-checkerboard"
      : backgroundMode === "dark"
        ? "bg-dark-matte"
        : backgroundMode === "light"
          ? "bg-light-matte"
          : backgroundMode === "navy"
            ? "bg-navy-matte"
            : backgroundMode === "purple"
              ? "bg-purple-matte"
              : backgroundMode === "web-mockup"
                ? "bg-web-mockup"
                : "bg-checkerboard";

  // Draw canvases
  const drawCanvas = useCallback(
    (
      canvas: HTMLCanvasElement | null,
      data: string | null,
      mimeType: string
    ) => {
      if (!canvas || !data) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const img = new Image();
      img.onload = () => {
        canvas.width = img.width;
        canvas.height = img.height;
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.drawImage(img, 0, 0);
      };
      img.src = `data:${mimeType};base64,${data}`;
    },
    []
  );

  useEffect(() => {
    drawCanvas(originalCanvasRef.current, originalData, originalMimeType);
  }, [originalData, originalMimeType, drawCanvas]);

  useEffect(() => {
    drawCanvas(optimizedCanvasRef.current, optimizedData, optimizedMimeType);
  }, [optimizedData, optimizedMimeType, drawCanvas]);

  // Split slider drag
  const handleSplitMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (comparisonMode !== "split-slider") return;
      e.preventDefault();
      setIsDragging(true);
    },
    [comparisonMode]
  );

  useEffect(() => {
    if (!isDragging) return;

    const handleMove = (e: MouseEvent) => {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const x = (e.clientX - rect.left) / rect.width;
      setSplitPos(Math.max(0, Math.min(1, x)));
    };

    const handleUp = () => setIsDragging(false);

    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [isDragging]);

  // Pan handling
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (
        comparisonMode === "split-slider" &&
        Math.abs(
          e.nativeEvent.offsetX -
            (containerRef.current?.clientWidth ?? 0) * splitPos
        ) < 10
      ) {
        return;
      }
      if (e.button === 0) {
        setIsPanning(true);
        lastPanPos.current = { x: e.clientX, y: e.clientY };
      }
    },
    [comparisonMode, splitPos]
  );

  useEffect(() => {
    if (!isPanning) return;

    const handleMove = (e: MouseEvent) => {
      const dx = e.clientX - lastPanPos.current.x;
      const dy = e.clientY - lastPanPos.current.y;
      lastPanPos.current = { x: e.clientX, y: e.clientY };
      onPan(dx, dy);
    };

    const handleUp = () => setIsPanning(false);

    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [isPanning, onPan]);

  const canvasStyle = {
    transform: `translate(${panX}px, ${panY}px) scale(${zoom})`,
    transformOrigin: "0 0",
  };

  // Wheel: Ctrl/⌘ + wheel zooms around the cursor (and blocks the WebView page zoom),
  // Shift + wheel pans horizontally, plain wheel pans vertically. A native non-passive
  // listener is required because React registers wheel handlers as passive.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) {
        const rect = el.getBoundingClientRect();
        const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
        onZoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
      } else if (e.shiftKey) {
        onPan(-e.deltaY, 0);
      } else {
        onPan(-e.deltaX, -e.deltaY);
      }
    };
    el.addEventListener("wheel", handleWheel, { passive: false });
    return () => el.removeEventListener("wheel", handleWheel);
  }, [onZoomAt, onPan]);

  return (
    <div className="comparison-view">
      {/* Toolbar */}
      <div className="comparison-toolbar">
        <div className="comparison-toolbar__group">
          <span className="comparison-toolbar__label">{t("compare.view")}</span>
          <button
            className={`btn-sm ${comparisonMode === "side-by-side" ? "btn-sm--active" : ""}`}
            onClick={() => onComparisonModeChange("side-by-side")}
          >
            {t("compare.sideBySide")}
          </button>
          <button
            className={`btn-sm ${comparisonMode === "split-slider" ? "btn-sm--active" : ""}`}
            onClick={() => onComparisonModeChange("split-slider")}
          >
            {t("compare.splitSlider")}
          </button>
        </div>

        <div className="comparison-toolbar__group">
          <span className="comparison-toolbar__label">{t("compare.inspect")}</span>
          {(
            [
              ["none", t("compare.none")],
              ["difference", t("compare.diff")],
              ["alpha-edge", t("compare.alpha")],
              ["high-contrast", t("compare.edges")],
              ["magnifier", t("compare.loupe")],
            ] as [InspectionMode, string][]
          ).map(([mode, label]) => (
            <button
              key={mode}
              className={`btn-sm ${inspectionMode === mode ? "btn-sm--active" : ""}`}
              onClick={() => onInspectionModeChange(mode)}
            >
              {label}
            </button>
          ))}
        </div>

        {onBackgroundModeChange && (
          <div className="comparison-toolbar__group">
            <span className="comparison-toolbar__label">
              {t("settings.background")}
            </span>
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
        )}
      </div>

      {/* Banding Alert Banner */}
      {bandingRisk && bandingRisk.detected && (
        <div className={`banding-alert banding-alert--${bandingRisk.severity}`}>
          <div className="banding-alert__text">
            <span className="banding-alert__icon">🛡️</span>
            <strong>Banding Radarı:</strong> {bandingRisk.message}
          </div>
          {bandingRisk.recommendation && onApplyRecommendation && (
            <button
              type="button"
              className="banding-alert__btn"
              onClick={() =>
                onApplyRecommendation(bandingRisk.recommendation!)
              }
            >
              Önerilen Ayarı Uygula (
              {bandingRisk.recommendation.alphaQuality
                ? `Alpha Q: ${bandingRisk.recommendation.alphaQuality} `
                : ""}
              {bandingRisk.recommendation.method
                ? `Method: ${bandingRisk.recommendation.method}`
                : ""}
              )
            </button>
          )}
        </div>
      )}

      {/* Canvas area */}
      <div
        ref={containerRef}
        className={`comparison-view__canvases ${bgClass}`}
        onMouseDown={handleMouseDown}
        style={{ cursor: isPanning ? "grabbing" : "grab" }}
      >
        {backgroundMode === "web-mockup" && (
          <div className="web-mockup-header">
            <div className="web-mockup-header__dots">
              <span className="dot dot--red" />
              <span className="dot dot--yellow" />
              <span className="dot dot--green" />
            </div>
            <div className="web-mockup-header__url">https://seobify.com/hero-preview</div>
            <div className="web-mockup-header__nav">
              <span>Ürün</span>
              <span>Doküman</span>
              <span>Fiyatlandırma</span>
              <span className="btn-mockup">Giriş Yap</span>
            </div>
          </div>
        )}

        {comparisonMode === "side-by-side" ? (
          <div className="comparison-view__side-by-side">
            <div className="comparison-view__pane">
              <div className="comparison-view__label">{t("compare.original")}</div>
              <canvas ref={originalCanvasRef} style={canvasStyle} />
            </div>
            <div className="comparison-view__divider" />
            <div className="comparison-view__pane">
              <div className="comparison-view__label">{t("compare.optimized")}</div>
              {isEncoding && (
                <div className="comparison-view__loading">
                  <div className="spinner" />
                </div>
              )}
              <canvas ref={optimizedCanvasRef} style={canvasStyle} />
            </div>
          </div>
        ) : (
          <div className="comparison-view__split">
            <div
              className="comparison-view__split-left"
              style={{ clipPath: `inset(0 ${(1 - splitPos) * 100}% 0 0)` }}
            >
              <div className="comparison-view__label">{t("compare.original")}</div>
              <canvas ref={originalCanvasRef} style={canvasStyle} />
            </div>
            <div
              className="comparison-view__split-right"
              style={{ clipPath: `inset(0 0 0 ${splitPos * 100}%)` }}
            >
              <div className="comparison-view__label">{t("compare.optimized")}</div>
              <canvas ref={optimizedCanvasRef} style={canvasStyle} />
            </div>
            <div
              className="comparison-view__slider-handle"
              style={{ left: `${splitPos * 100}%` }}
              onMouseDown={handleSplitMouseDown}
            >
              <div className="comparison-view__slider-line" />
            </div>
          </div>
        )}

        <InspectionOverlay
          originalImageData={originalData}
          optimizedImageData={optimizedData}
          mode={inspectionMode}
          width={imageWidth}
          height={imageHeight}
          zoom={zoom}
          panX={panX}
          panY={panY}
          hasAlpha={hasAlpha}
        />
      </div>
    </div>
  );
}
