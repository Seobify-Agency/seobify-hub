import { useCallback, useEffect, useRef, useState } from "react";
import FileDropZone from "./components/FileDropZone";
import ComparisonView from "./components/ComparisonView";
import ZoomController from "./components/ZoomController";
import SettingsPanel from "./components/SettingsPanel";
import ExportDialog from "./components/ExportDialog";
import BatchQueue from "./components/BatchQueue";
import SetupScreen from "./components/SetupScreen";
import Header from "./components/Header";
import DocumentsView from "./components/DocumentsView";
import { useEncoding } from "./hooks/useEncoding";
import { useZoom } from "./hooks/useZoom";
import { useInspection } from "./hooks/useInspection";
import { useBatchQueue } from "./hooks/useBatchQueue";
import { useDocumentQueue } from "./hooks/useDocumentQueue";
import { detectCwebp } from "./services/encodeService";
import { loadImage, pickImageFiles } from "./services/imageService";
import { inspectMetadata } from "./services/metadataService";
import { pickDocumentFiles } from "./services/documentService";
import { runImageAnalysis, autoCalibrateQuality } from "./services/analysisService";
import { useTranslation } from "./i18n";
import type {
  BackgroundMode,
  BandingRisk,
  ComparisonMode,
  CwebpStatus,
  DocCleanOptions,
  EncodingConfig,
  InspectReport,
  Preset,
  SanitizeConfig,
  SourceImage,
  SsimResult,
} from "./types";
import { DEFAULT_DOC_OPTIONS, DEFAULT_ENCODING_CONFIG, DEFAULT_SANITIZE_CONFIG } from "./types";
import "./App.css";

type AppMode = "single" | "batch" | "documents";

export default function App() {
  const [cwebpStatus, setCwebpStatus] = useState<CwebpStatus | null>(null);
  const [cwebpChecked, setCwebpChecked] = useState(false);
  const [mode, setMode] = useState<AppMode>("single");
  const [sourceImage, setSourceImage] = useState<SourceImage | null>(null);
  const [config, setConfig] = useState<EncodingConfig>(DEFAULT_ENCODING_CONFIG);
  const [sanitize, setSanitize] = useState<SanitizeConfig>(DEFAULT_SANITIZE_CONFIG);
  const [inspectReport, setInspectReport] = useState<InspectReport | null>(null);
  const [isInspecting, setIsInspecting] = useState(false);
  const [backgroundMode, setBackgroundMode] =
    useState<BackgroundMode>("checkerboard");
  const [comparisonMode, setComparisonMode] =
    useState<ComparisonMode>("side-by-side");
  const [error, setError] = useState<string | null>(null);
  const [activePresetId, setActivePresetId] = useState<string | null>(null);

  // Advanced Superpower state
  const { t } = useTranslation();
  const [ssimResult, setSsimResult] = useState<SsimResult | null>(null);
  const [bandingRisk, setBandingRisk] = useState<BandingRisk | null>(null);
  const [isCalibrating, setIsCalibrating] = useState(false);

  useEffect(() => {
    detectCwebp()
      .then((status) => {
        setCwebpStatus(status);
        setCwebpChecked(true);
      })
      .catch(() => {
        setCwebpStatus({
          available: false,
          binaryPath: null,
          version: null,
          platform: "unknown",
        });
        setCwebpChecked(true);
      });
  }, []);

  const handleCwebpDetected = useCallback((status: CwebpStatus) => {
    setCwebpStatus(status);
  }, []);

  const handleCwebpLost = useCallback(() => {
    setCwebpStatus({
      available: false,
      binaryPath: null,
      version: null,
      platform: "unknown",
    });
  }, []);

  const batch = useBatchQueue();
  const docs = useDocumentQueue();
  const [docOptions, setDocOptions] = useState<DocCleanOptions>(DEFAULT_DOC_OPTIONS);

  // In batch mode, use the selected batch item as the active image; documents mode has none.
  const activeImage = mode === "documents" ? null : mode === "batch" ? batch.selectedImage : sourceImage;

  const {
    optimizedImage,
    isEncoding,
    error: encodeError,
  } = useEncoding(activeImage, config, sanitize, handleCwebpLost);

  // Inspect container metadata whenever the active source file changes
  useEffect(() => {
    const filePath = activeImage?.filePath;
    if (!filePath) {
      setInspectReport(null);
      setIsInspecting(false);
      return;
    }
    let isCancelled = false;
    setIsInspecting(true);
    inspectMetadata(filePath)
      .then((report) => {
        if (!isCancelled) setInspectReport(report);
      })
      .catch((err) => {
        console.warn("Metadata inspection error:", err);
        if (!isCancelled) setInspectReport(null);
      })
      .finally(() => {
        if (!isCancelled) setIsInspecting(false);
      });
    return () => {
      isCancelled = true;
    };
  }, [activeImage?.filePath]);

  const {
    zoom,
    panX,
    panY,
    setZoom,
    zoomIn,
    zoomOut,
    zoomAt,
    pan,
    resetPan,
    zoomLevels,
  } = useZoom();
  const { inspectionMode, setInspectionMode } = useInspection();

  // Run SSIM & Banding Radar analysis in background whenever images change
  useEffect(() => {
    if (!activeImage?.imageData || !optimizedImage?.imageData) {
      setSsimResult(null);
      setBandingRisk(null);
      return;
    }

    let isCancelled = false;
    runImageAnalysis(
      activeImage.imageData,
      optimizedImage.imageData,
      activeImage.width,
      activeImage.height,
      activeImage.mimeType,
      optimizedImage.mimeType
    )
      .then(({ ssim, banding }) => {
        if (!isCancelled) {
          setSsimResult(ssim);
          setBandingRisk(banding);
        }
      })
      .catch((err) => {
        console.warn("Background analysis error:", err);
      });

    return () => {
      isCancelled = true;
    };
  }, [
    activeImage?.imageData,
    optimizedImage?.imageData,
    activeImage?.width,
    activeImage?.height,
    activeImage?.mimeType,
    optimizedImage?.mimeType,
  ]);

  // Auto-Calibrate (Zero-Loss)
  const handleAutoCalibrate = useCallback(async () => {
    if (!activeImage) return;
    setIsCalibrating(true);
    try {
      const { optimalQuality, finalSsim } = await autoCalibrateQuality(
        activeImage,
        config,
        0.992
      );
      setConfig((prev) => ({
        ...prev,
        quality: optimalQuality,
        lossless: false,
      }));
      setActivePresetId(null);
      setSsimResult(finalSsim);
    } catch (err) {
      console.error("Auto-calibration error:", err);
    } finally {
      setIsCalibrating(false);
    }
  }, [activeImage, config]);

  // Apply Banding recommendation
  const handleApplyRecommendation = useCallback(
    (rec: Partial<EncodingConfig>) => {
      setConfig((prev) => ({ ...prev, ...rec }));
      setActivePresetId(null);
    },
    []
  );

  const handleImageLoaded = useCallback((image: SourceImage) => {
    setSourceImage(image);
    setMode("single");
    setError(null);
  }, []);

  const handleMultipleFiles = useCallback(
    (paths: string[]) => {
      setMode("batch");
      batch.addFiles(paths);
      setError(null);
    },
    [batch]
  );

  const handleError = useCallback((err: string) => {
    setError(err);
  }, []);

  // Header: add images via the native picker (routes to batch when in batch mode or
  // when several files are chosen) and close the current image / queue.
  const handleAddImages = useCallback(async () => {
    try {
      if (mode === "documents") {
        const skipped = docs.addFiles(await pickDocumentFiles());
        if (skipped > 0) setError(t("docs.errorSkipped", skipped));
        return;
      }
      const paths = await pickImageFiles();
      if (paths.length === 0) return;
      if (mode === "batch" || paths.length > 1) {
        handleMultipleFiles(paths);
        return;
      }
      const image = await loadImage(paths[0]);
      handleImageLoaded(image);
    } catch (err) {
      setError(String(err));
    }
  }, [mode, handleMultipleFiles, handleImageLoaded, docs, t]);

  const handleClose = useCallback(() => {
    if (mode === "documents") {
      docs.clear();
    } else if (mode === "batch") {
      batch.clearQueue();
    } else {
      setSourceImage(null);
    }
    setError(null);
  }, [mode, batch, docs]);

  // Auto-dismiss warnings after 6 seconds
  const errorTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (error && error.startsWith("Warning:")) {
      errorTimerRef.current = setTimeout(() => setError(null), 6000);
      return () => {
        if (errorTimerRef.current) clearTimeout(errorTimerRef.current);
      };
    }
  }, [error]);

  const handlePresetSelect = useCallback((preset: Preset) => {
    setConfig({ ...preset.config });
    setActivePresetId(preset.id);
  }, []);

  const handleConfigChange = useCallback((newConfig: EncodingConfig) => {
    setConfig(newConfig);
    setActivePresetId(null);
  }, []);

  const hasContent = mode === "documents" ? docs.items.length > 0 : mode === "batch" ? batch.queue.length > 0 : !!sourceImage;
  const displayError = error || encodeError;

  if (!cwebpChecked) {
    return (
      <div className="app">
        <p style={{ color: "#aaa", textAlign: "center", marginTop: "40vh" }}>
          {t("app.detecting")}
        </p>
      </div>
    );
  }

  // Documents never need cwebp, so that mode stays reachable without it.
  if (!cwebpStatus?.available && mode !== "documents") {
    return <SetupScreen onCwebpDetected={handleCwebpDetected} onContinueWithDocuments={() => setMode("documents")} />;
  }

  return (
    <div className="app">
      <Header
        appMode={mode}
        onAppModeChange={setMode}
        hasContent={hasContent}
        onAddImages={handleAddImages}
        onClose={handleClose}
      />

      {displayError && (
        <div className="app__error">
          <span>{displayError}</span>
          <button onClick={() => setError(null)} className="app__error-close">
            ✕
          </button>
        </div>
      )}

      {mode === "documents" && (
        <DocumentsView docs={docs} options={docOptions} onOptionsChange={setDocOptions} onError={handleError} />
      )}

      {mode !== "documents" && (
        <FileDropZone
          onImageLoaded={handleImageLoaded}
          onMultipleFiles={handleMultipleFiles}
          onError={handleError}
          hasImage={hasContent}
          preferBatch={mode === "batch"}
        />
      )}

      {hasContent && mode !== "documents" && (
        <div className="app__workspace">
          {/* Batch queue panel (left side) */}
          {mode === "batch" && (
            <BatchQueue
              queue={batch.queue}
              selectedId={batch.selectedId}
              onSelect={batch.setSelectedId}
              onRemove={batch.removeItem}
              onMove={batch.moveItem}
              onExportAll={(cfg) => batch.exportAll(cfg, sanitize)}
              config={config}
              isExporting={batch.isExporting}
              seoSlug={sanitize.seoSlug}
              onSeoSlugChange={(enabled) => setSanitize((prev) => ({ ...prev, seoSlug: enabled }))}
            />
          )}

          <div className="app__main">
            <div className="app__toolbar">
              <ZoomController
                zoom={zoom}
                zoomLevels={zoomLevels}
                onZoomChange={setZoom}
                onZoomIn={zoomIn}
                onZoomOut={zoomOut}
                onResetPan={resetPan}
              />
              <span className="app__toolbar-spacer" />
            </div>

            {activeImage ? (
              <ComparisonView
                originalData={activeImage.imageData}
                optimizedData={optimizedImage?.imageData ?? null}
                originalMimeType={activeImage.mimeType}
                optimizedMimeType={optimizedImage?.mimeType}
                backgroundMode={backgroundMode}
                onBackgroundModeChange={setBackgroundMode}
                comparisonMode={comparisonMode}
                onComparisonModeChange={setComparisonMode}
                inspectionMode={inspectionMode}
                onInspectionModeChange={setInspectionMode}
                zoom={zoom}
                panX={panX}
                panY={panY}
                onPan={pan}
                onZoomAt={zoomAt}
                imageWidth={activeImage.width}
                imageHeight={activeImage.height}
                isEncoding={isEncoding}
                hasAlpha={activeImage.hasAlpha}
                bandingRisk={bandingRisk}
                onApplyRecommendation={handleApplyRecommendation}
              />
            ) : (
              <div className="app__empty">
                <p>{t("app.selectImage")}</p>
              </div>
            )}
          </div>

          <SettingsPanel
            config={config}
            onConfigChange={handleConfigChange}
            sourceImage={activeImage}
            optimizedImage={optimizedImage}
            backgroundMode={backgroundMode}
            onBackgroundModeChange={setBackgroundMode}
            isEncoding={isEncoding}
            activePresetId={activePresetId}
            onPresetSelect={handlePresetSelect}
            ssimResult={ssimResult}
            onAutoCalibrate={handleAutoCalibrate}
            isCalibrating={isCalibrating}
            sanitize={sanitize}
            onSanitizeChange={setSanitize}
            inspectReport={inspectReport}
            isInspecting={isInspecting}
          />
        </div>
      )}

      {mode === "single" && sourceImage && (
        <ExportDialog
          sourceImage={sourceImage}
          optimizedImage={optimizedImage}
          defaultFileName={sourceImage.fileName}
        />
      )}
    </div>
  );
}
