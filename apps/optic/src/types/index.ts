export type MetadataOption = "none" | "all" | "exif" | "icc" | "xmp";

export type BatchItemStatus = "loading" | "pending" | "processing" | "completed" | "failed";

export interface EncodingConfig {
  quality: number;
  alphaQuality: number;
  method: number;
  lossless: boolean;
  nearLossless: number | null;
  metadata: MetadataOption;
}

export type ImageFormatLabel = "png" | "jpeg" | "webp";

export type SanitizeMode = "off" | "safe" | "sensor_natural" | "studio_master";

export type OutputFormat = "webp" | "original" | "png" | "jpeg";

export interface BrandStamp {
  artist: string;
  copyright: string;
}

export interface SanitizeConfig {
  mode: SanitizeMode;
  deblendCorner: boolean;
  outputFormat: OutputFormat;
  /** Clean attribution written after every foreign trace is removed. */
  brandStamp: BrandStamp | null;
  /** Name batch outputs with SEO slugs derived from the source file names. */
  seoSlug: boolean;
}

export type Severity = "info" | "low" | "medium" | "high" | "critical";

export interface Finding {
  category: string;
  severity: Severity;
  location: string;
  name: string;
  detail: string;
  sizeBytes: number;
  valuePreview: string;
  removable: boolean;
}

export interface InspectReport {
  format: string;
  fileSize: number;
  width: number;
  height: number;
  findings: Finding[];
  critical: number;
  high: number;
  medium: number;
  low: number;
  info: number;
  traceCount: number;
  clean: boolean;
  summary: string;
}

export interface PipelineStats {
  mode: SanitizeMode;
  inputWidth: number;
  inputHeight: number;
  outputWidth: number;
  outputHeight: number;
  ssim: number;
  psnr: number;
  ssimSource: number;
  psnrSource: number;
  stages: string[];
  cornerOverlayDetected: boolean;
  cornerOverlayAlpha: number;
  cornerOverlaySize: string;
  cornerOverlayShape: string;
  cornerOverlayMargin: number;
  spectralPeaksAttenuated: number;
  durationMs: number;
}

export interface SourceImage {
  id: string;
  filePath: string;
  fileName: string;
  fileSize: number;
  width: number;
  height: number;
  hasAlpha: boolean;
  imageData: string;
  format: ImageFormatLabel;
  mimeType: string;
}

export interface OptimizedImage {
  fileSize: number;
  reductionPercent: number;
  encodingDurationMs: number;
  imageData: string;
  tempFilePath: string;
  outputFormat: ImageFormatLabel;
  mimeType: string;
  width: number;
  height: number;
  findingsBefore: number;
  findingsAfter: number;
  clean: boolean;
  inspectAfter: InspectReport;
  pipeline: PipelineStats | null;
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  config: EncodingConfig;
}

export interface BatchQueueItem {
  id: string;
  filePath: string;
  fileName: string;
  /** `null` while the file is still being read. */
  sourceImage: SourceImage | null;
  position: number;
  status: BatchItemStatus;
  optimizedImage: OptimizedImage | null;
  outputPath: string | null;
  error: string | null;
}

export interface CwebpStatus {
  available: boolean;
  binaryPath: string | null;
  version: string | null;
  platform: string;
}

export interface ExportResult {
  outputPath: string;
  fileSize: number;
  findingsAfter: number;
  clean: boolean;
}

export interface BatchExportResult {
  results: BatchExportItemResult[];
  totalCompleted: number;
  totalFailed: number;
}

export interface BatchExportItemResult {
  id: string;
  status: "completed" | "failed";
  outputPath?: string;
  fileSize?: number;
  reductionPercent?: number;
  encodingDurationMs?: number;
  findingsBefore?: number;
  findingsAfter?: number;
  error?: string;
}

export interface BatchProgressPayload {
  itemId: string;
  status: "completed" | "failed";
  outputPath?: string;
  fileSize?: number;
  reductionPercent?: number;
  findingsAfter?: number;
  currentIndex: number;
  totalItems: number;
  error?: string;
}

export interface SavePathResult {
  path: string;
}

export interface OutputDirResult {
  path: string;
}

export type BackgroundMode =
  | "checkerboard"
  | "dark"
  | "light"
  | "navy"
  | "purple"
  | "custom"
  | "web-mockup";

export type ComparisonMode = "side-by-side" | "split-slider";

export type InspectionMode =
  | "none"
  | "difference"
  | "alpha-edge"
  | "high-contrast"
  | "magnifier";

export type SsimRating =
  | "imperceptible"
  | "excellent"
  | "good"
  | "fair"
  | "poor";

export interface SsimResult {
  ssim: number;
  ssimPercent: number;
  rating: SsimRating;
}

export type BandingSeverity = "none" | "low" | "medium" | "high";

export interface BandingRisk {
  detected: boolean;
  severity: BandingSeverity;
  message: string;
  recommendation?: Partial<EncodingConfig>;
}

export interface ResponsiveVariant {
  label: string;
  suffix: string;
  width: number;
  height: number;
  scale: number;
  enabled: boolean;
}

export interface SeoPictureConfig {
  altText: string;
  isLcp: boolean;
  loading: "lazy" | "eager";
  decoding: "async" | "auto" | "sync";
  fetchPriority: "high" | "low" | "auto";
  variants: ResponsiveVariant[];
}

export const SUPPORTED_EXTENSIONS = ["png", "jpg", "jpeg", "webp"] as const;

export const DEFAULT_BRAND_STAMP: BrandStamp = {
  artist: "Seobify",
  copyright: "© 2026 Seobify — seobify.com",
};

export const DEFAULT_SANITIZE_CONFIG: SanitizeConfig = {
  mode: "off",
  deblendCorner: false,
  outputFormat: "webp",
  brandStamp: null,
  seoSlug: false,
};

export const DEFAULT_ENCODING_CONFIG: EncodingConfig = {
  quality: 80,
  alphaQuality: 100,
  method: 4,
  lossless: false,
  nearLossless: null,
  metadata: "none",
};


// ---------------------------------------------------------------------------
// Documents (DOCX/XLSX/PPTX, ODT/ODS/ODP/ODG, PDF)
// ---------------------------------------------------------------------------

export type DocMode = "safe" | "strict";

/** Which cleaning step removes a finding; `report` = shown only (macros, JavaScript, embedded files, brand stamp). */
export type DocScope = "safe" | "strict" | "notes" | "report";

export interface DocFinding extends Finding {
  scope: DocScope;
}

export interface DocInspectReport {
  format: string;
  family: "word" | "spreadsheet" | "presentation" | "drawing" | "pdf";
  fileSize: number;
  findings: DocFinding[];
  critical: number;
  high: number;
  medium: number;
  low: number;
  info: number;
  traceCount: number;
  safeCount: number;
  strictCount: number;
  notesCount: number;
  clean: boolean;
  summary: string;
}

export interface DocCleanOptions {
  mode: DocMode;
  removeNotes: boolean;
  brandStamp: BrandStamp | null;
}

export interface DocProcessOutcome {
  inputPath: string;
  outputPath: string;
  format: string;
  mode: DocMode;
  originalSize: number;
  fileSize: number;
  tracesBefore: number;
  tracesAfter: number;
  tracesRemoved: number;
  clean: boolean;
  stampVerified: string | null;
  warnings: string[];
  durationMs: number;
  before: DocInspectReport;
  after: DocInspectReport;
}

export type DocItemStatus = "inspecting" | "ready" | "cleaning" | "done" | "failed";

export interface DocItem {
  id: string;
  filePath: string;
  fileName: string;
  status: DocItemStatus;
  report: DocInspectReport | null;
  outcome: DocProcessOutcome | null;
  error: string | null;
}

export const DOCUMENT_EXTENSIONS = [
  "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "pptx", "pptm", "potx", "potm", "ppsx", "ppsm",
  "odt", "ott", "ods", "ots", "odp", "otp", "odg", "otg", "pdf",
] as const;

export const DEFAULT_DOC_OPTIONS: DocCleanOptions = {
  mode: "safe",
  removeNotes: false,
  brandStamp: null,
};
