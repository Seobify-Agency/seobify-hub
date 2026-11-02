import { describe, it, expect } from "vitest";
import {
  cleanFileName,
  docSeverityCounts,
  documentExtension,
  isSupportedDocumentPath,
  planCounts,
  scopeApplies,
  sortedDocFindings,
} from "../services/documentService";
import { batchTarget } from "../hooks/useDocumentQueue";
import en from "../i18n/en";
import tr from "../i18n/tr";
import type { DocFinding, DocInspectReport } from "../types";
import { DEFAULT_DOC_OPTIONS } from "../types";

function finding(partial: Partial<DocFinding>): DocFinding {
  return {
    category: "document-properties",
    severity: "medium",
    location: "docProps/core.xml",
    name: "x",
    detail: "",
    sizeBytes: 0,
    valuePreview: "",
    removable: true,
    scope: "safe",
    ...partial,
  };
}

function report(findings: DocFinding[]): DocInspectReport {
  const removable = findings.filter((f) => f.removable);
  return {
    format: "docx",
    family: "word",
    fileSize: 1,
    findings,
    critical: 0,
    high: 0,
    medium: 0,
    low: 0,
    info: 0,
    traceCount: removable.length,
    safeCount: removable.filter((f) => f.scope === "safe").length,
    strictCount: removable.filter((f) => f.scope === "strict").length,
    notesCount: removable.filter((f) => f.scope === "notes").length,
    clean: removable.length === 0,
    summary: "",
  };
}

describe("document paths", () => {
  it("accepts Office, OpenDocument and PDF files case-insensitively", () => {
    for (const p of ["C:\\a\\Teklif.DOCX", "/x/b.xlsm", "c.pptx", "d.odt", "e.ods", "f.odp", "g.PDF", "h.dotx"]) {
      expect(isSupportedDocumentPath(p)).toBe(true);
    }
    for (const p of ["a.doc", "b.png", "c.txt", "noext"]) {
      expect(isSupportedDocumentPath(p)).toBe(false);
    }
    expect(documentExtension("C:\\dir.v2\\Rapor.Final.PDF")).toBe("pdf");
  });

  it("names the clean copy <stem>.clean.<ext>", () => {
    expect(cleanFileName("Teklif Ekim.docx")).toBe("Teklif Ekim.clean.docx");
    expect(cleanFileName("a.b.pdf")).toBe("a.b.clean.pdf");
  });
});

describe("cleaning plan", () => {
  const r = report([
    finding({ scope: "safe", severity: "high" }),
    finding({ scope: "strict", severity: "critical" }),
    finding({ scope: "notes", severity: "medium" }),
    finding({ scope: "report", removable: false, severity: "high", name: "VBA" }),
  ]);

  it("maps scopes to modes", () => {
    expect(scopeApplies("safe", DEFAULT_DOC_OPTIONS)).toBe(true);
    expect(scopeApplies("strict", DEFAULT_DOC_OPTIONS)).toBe(false);
    expect(scopeApplies("notes", { ...DEFAULT_DOC_OPTIONS, mode: "strict" })).toBe(false);
    expect(scopeApplies("notes", { ...DEFAULT_DOC_OPTIONS, mode: "strict", removeNotes: true })).toBe(true);
    expect(scopeApplies("report", { mode: "strict", removeNotes: true, brandStamp: null })).toBe(false);
  });

  it("counts removed and kept traces for the chosen mode", () => {
    expect(planCounts(r, DEFAULT_DOC_OPTIONS)).toEqual({ removed: 1, kept: 2 });
    expect(planCounts(r, { ...DEFAULT_DOC_OPTIONS, mode: "strict" })).toEqual({ removed: 2, kept: 1 });
    expect(planCounts(r, { ...DEFAULT_DOC_OPTIONS, mode: "strict", removeNotes: true })).toEqual({ removed: 3, kept: 0 });
    expect(planCounts(null, DEFAULT_DOC_OPTIONS)).toEqual({ removed: 0, kept: 0 });
  });

  it("lists traces by severity before report-only rows", () => {
    const sorted = sortedDocFindings(r);
    expect(sorted.map((f) => f.severity)).toEqual(["critical", "high", "medium", "high"]);
    expect(sorted[3].name).toBe("VBA");
    expect(docSeverityCounts(r)).toEqual({ critical: 1, high: 1, medium: 1, low: 0, info: 0 });
  });
});

describe("batch targets", () => {
  it("keeps names in another folder, adds .clean in the source folder and de-duplicates", () => {
    const used = new Set<string>();
    expect(batchTarget("C:\\in\\a.docx", "a.docx", "C:\\out", used)).toEqual({ outputDir: "C:\\out" });
    expect(batchTarget("C:\\other\\a.docx", "a.docx", "C:\\out", used)).toEqual({ outputPath: "C:\\out\\a-2.docx" });
    expect(batchTarget("C:\\out\\a.docx", "a.docx", "C:\\out\\", used)).toEqual({ outputDir: "C:\\out\\" });
    expect(used.has("a.clean.docx")).toBe(true);
  });
});

describe("i18n", () => {
  it("has the same keys in Turkish and English", () => {
    expect(Object.keys(tr).sort()).toEqual(Object.keys(en).sort());
  });
});
