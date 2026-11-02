import { describe, it, expect } from "vitest";
import {
  outputExtension,
  outputFileName,
  severityCounts,
  visibleFindings,
} from "../services/privacyService";
import type { Finding, InspectReport } from "../types";

function finding(partial: Partial<Finding>): Finding {
  return {
    category: "exif",
    severity: "medium",
    location: "APP1",
    name: "x",
    detail: "",
    sizeBytes: 0,
    valuePreview: "",
    removable: true,
    ...partial,
  };
}

function report(findings: Finding[]): InspectReport {
  const removable = findings.filter((f) => f.removable);
  return {
    format: "jpeg",
    fileSize: 100,
    width: 1,
    height: 1,
    findings,
    critical: 0,
    high: 0,
    medium: 0,
    low: 0,
    info: 0,
    traceCount: removable.length,
    clean: removable.length === 0,
    summary: "",
  };
}

describe("outputExtension / outputFileName", () => {
  it("maps output formats to file extensions", () => {
    expect(outputExtension("webp", "png")).toBe("webp");
    expect(outputExtension("png", "jpeg")).toBe("png");
    expect(outputExtension("jpeg", "png")).toBe("jpg");
    expect(outputExtension("original", "jpeg")).toBe("jpg");
    expect(outputExtension("original", "webp")).toBe("webp");
  });

  it("replaces the source extension case-insensitively", () => {
    expect(outputFileName("Photo.JPEG", "original", "jpeg")).toBe("Photo.jpg");
    expect(outputFileName("hero.png", "webp", "png")).toBe("hero.webp");
    expect(outputFileName("noext", "png", "webp")).toBe("noext.png");
  });
});

describe("visibleFindings / severityCounts", () => {
  it("orders by severity and hides non-removable info rows", () => {
    const r = report([
      finding({ name: "low", severity: "low" }),
      finding({ name: "ihdr", severity: "info", removable: false }),
      finding({ name: "crit", severity: "critical" }),
      finding({ name: "dqt", severity: "low", removable: false }),
      finding({ name: "high", severity: "high" }),
    ]);
    expect(visibleFindings(r).map((f) => f.name)).toEqual([
      "crit",
      "high",
      "low",
      "dqt",
    ]);
    const counts = severityCounts(r);
    expect(counts).toEqual({ critical: 1, high: 1, medium: 0, low: 1, info: 0 });
    expect(visibleFindings(null)).toEqual([]);
    expect(severityCounts(null).critical).toBe(0);
  });
});
