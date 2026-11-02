import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  DocCleanOptions,
  DocFinding,
  DocInspectReport,
  DocProcessOutcome,
  DocScope,
  Severity,
} from "../types";
import { DOCUMENT_EXTENSIONS } from "../types";
import { SEVERITY_ORDER } from "./privacyService";

export function documentExtension(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function isSupportedDocumentPath(path: string): boolean {
  return (DOCUMENT_EXTENSIONS as readonly string[]).includes(documentExtension(path));
}

/** `Teklif.docx` → `Teklif.clean.docx` (the default name of the cleaned copy). */
export function cleanFileName(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  return dot > 0 ? `${fileName.slice(0, dot)}.clean${fileName.slice(dot)}` : `${fileName}.clean`;
}

/** Whether cleaning with `options` removes findings of `scope`. */
export function scopeApplies(scope: DocScope, options: DocCleanOptions): boolean {
  switch (scope) {
    case "safe":
      return true;
    case "strict":
      return options.mode === "strict";
    case "notes":
      return options.mode === "strict" && options.removeNotes;
    default:
      return false;
  }
}

/** Traces the chosen mode removes and those it leaves (removable findings only). */
export function planCounts(report: DocInspectReport | null, options: DocCleanOptions): { removed: number; kept: number } {
  if (!report) return { removed: 0, kept: 0 };
  let removed = 0;
  let kept = 0;
  for (const f of report.findings) {
    if (!f.removable) continue;
    if (scopeApplies(f.scope, options)) removed += 1;
    else kept += 1;
  }
  return { removed, kept };
}

/** Removable traces first (by severity), then report-only rows. */
export function sortedDocFindings(report: DocInspectReport | null): DocFinding[] {
  if (!report) return [];
  return [...report.findings].sort((a, b) => {
    if (a.removable !== b.removable) return a.removable ? -1 : 1;
    return SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity);
  });
}

export function docSeverityCounts(report: DocInspectReport | null): Record<Severity, number> {
  const counts: Record<Severity, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
  for (const f of report?.findings ?? []) {
    if (f.removable) counts[f.severity] += 1;
  }
  return counts;
}

export async function pickDocumentFiles(): Promise<string[]> {
  const selected = await open({
    multiple: true,
    filters: [{ name: "Documents", extensions: [...DOCUMENT_EXTENSIONS] }],
  });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export async function inspectDocument(filePath: string): Promise<DocInspectReport> {
  return invoke<DocInspectReport>("inspect_document", { filePath });
}

export async function cleanDocument(
  inputPath: string,
  options: DocCleanOptions,
  target: { outputPath?: string; outputDir?: string } = {}
): Promise<DocProcessOutcome> {
  return invoke<DocProcessOutcome>("clean_document", {
    inputPath,
    outputPath: target.outputPath ?? null,
    outputDir: target.outputDir ?? null,
    options,
  });
}

export async function selectDocumentSavePath(defaultName: string, extension: string): Promise<string | null> {
  const result = await invoke<{ path: string | null }>("select_document_save_path", { defaultName, extension });
  return result.path;
}

export async function selectOutputDirectory(): Promise<string | null> {
  const result = await invoke<{ path: string | null } | null>("select_output_directory");
  return result?.path ?? null;
}
