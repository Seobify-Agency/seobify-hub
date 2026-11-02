import type {
  Finding,
  ImageFormatLabel,
  InspectReport,
  OutputFormat,
  Severity,
} from "../types";

export const SEVERITY_ORDER: Severity[] = [
  "critical",
  "high",
  "medium",
  "low",
  "info",
];

export const SEVERITY_BADGE: Record<Severity, string> = {
  critical: "CRITICAL",
  high: "HIGH",
  medium: "MED",
  low: "LOW",
  info: "INFO",
};

/** Resolves the file extension of the exported file for a given output choice. */
export function outputExtension(
  outputFormat: OutputFormat,
  sourceFormat: ImageFormatLabel
): "webp" | "png" | "jpg" {
  const target = outputFormat === "original" ? sourceFormat : outputFormat;
  if (target === "png") return "png";
  if (target === "jpeg") return "jpg";
  return "webp";
}

/** Replaces the source extension with the one matching the export format. */
export function outputFileName(
  sourceFileName: string,
  outputFormat: OutputFormat,
  sourceFormat: ImageFormatLabel
): string {
  const ext = outputExtension(outputFormat, sourceFormat);
  const stem = sourceFileName.replace(/\.(png|jpe?g|jfif|webp)$/i, "");
  return `${stem}.${ext}`;
}

/** Findings worth showing: removable traces first, ordered by severity. */
export function visibleFindings(report: InspectReport | null): Finding[] {
  if (!report) return [];
  return [...report.findings]
    .filter((f) => f.removable || f.severity !== "info")
    .sort(
      (a, b) =>
        SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity)
    );
}

export function severityCounts(report: InspectReport | null): Record<Severity, number> {
  const counts: Record<Severity, number> = {
    critical: 0,
    high: 0,
    medium: 0,
    low: 0,
    info: 0,
  };
  if (!report) return counts;
  for (const f of report.findings) {
    if (f.removable) counts[f.severity] += 1;
  }
  return counts;
}
