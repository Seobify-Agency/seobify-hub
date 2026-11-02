import { describe, expect, it } from "vitest";
import type { MappingRow } from "../lib/api";
import en from "../i18n/en";
import tr from "../i18n/tr";
import { DEFAULT_FILTER, ISSUE_KINDS, OUTCOMES, STATUSES, confidenceTone, describeMove, directories, filterMappings, pathOf, splitReason, topDirectory } from "../lib/migrate";

function row(id: number, oldUrl: string, status: MappingRow["status"], confidence: number, extra: Partial<MappingRow> = {}): MappingRow {
  return {
    id,
    oldUrlId: id,
    oldUrl,
    oldTitle: null,
    oldStatus: 200,
    value: null,
    newUrl: "https://yeni.com/x",
    newTitle: null,
    newStatus: 200,
    status,
    confidence,
    reason: "fuzzy",
    approved: false,
    httpStatus: 301,
    candidates: [],
    updatedAt: "2026-10-29T00:00:00Z",
    ...extra,
  };
}

describe("migrate helpers", () => {
  it("decodes paths and finds directories", () => {
    expect(pathOf("https://a.com/hizmet/%C3%A7ay/?x=1")).toBe("/hizmet/çay/?x=1");
    expect(topDirectory("https://a.com/blog/2024/yazi")).toBe("/blog");
    expect(topDirectory("https://a.com/hakkimizda")).toBe("/");
  });

  it("splits reason codes and grades confidence", () => {
    expect(splitReason("pattern:3")).toEqual({ code: "pattern", n: "3" });
    expect(splitReason("same_slug")).toEqual({ code: "same_slug", n: null });
    expect(confidenceTone(92)).toBe("success");
    expect(confidenceTone(70)).toBe("warning");
    expect(confidenceTone(10)).toBe("error");
  });

  it("filters mappings by status, confidence, directory, approval and Turkish search", () => {
    const rows = [
      row(1, "https://eski.com/blog/kahve", "auto", 98, { oldTitle: "Kahve Rehberi" }),
      row(2, "https://eski.com/blog/çay", "review", 70, { approved: true }),
      row(3, "https://eski.com/urunler/fincan", "weak", 10),
    ];
    expect(filterMappings(rows, { ...DEFAULT_FILTER, status: "review" }).map((r) => r.id)).toEqual([2]);
    expect(filterMappings(rows, { ...DEFAULT_FILTER, min: 60 }).map((r) => r.id)).toEqual([1, 2]);
    expect(filterMappings(rows, { ...DEFAULT_FILTER, directory: "/blog" }).map((r) => r.id)).toEqual([1, 2]);
    expect(filterMappings(rows, { ...DEFAULT_FILTER, approval: "pending" }).map((r) => r.id)).toEqual([1, 3]);
    expect(filterMappings(rows, { ...DEFAULT_FILTER, query: "ÇAY" }).map((r) => r.id)).toEqual([2]);
    expect(filterMappings(rows, { ...DEFAULT_FILTER, query: "rehber" }).map((r) => r.id)).toEqual([1]);
    expect(directories(rows)).toEqual([{ dir: "/blog", count: 2 }, { dir: "/urunler", count: 1 }]);
  });

  it("describes directory moves", () => {
    expect(describeMove("^/urunler/(.+)$", "/magaza/$1")).toEqual({ from: "/urunler/", to: "/magaza/" });
    expect(describeMove("^/blog/\\d{4}/\\d{2}/(.+)$", "/makale/$1")).toEqual({ from: "/blog/####/##/", to: "/makale/" });
  });

  it("has labels for every status, outcome, issue and engine reason in both languages", () => {
    const reasons = ["pattern", "pattern_missing", "same_url", "same_path", "same_slug", "same_title", "same_h1", "fuzzy", "fuzzy_shift", "conflict", "parent", "homepage", "no_value_gone", "manual", "manual_gone", "manual_keep", "chain_collapsed"];
    const keys = [...STATUSES.map((s) => `status.${s}`), ...OUTCOMES.map((o) => `outcome.${o}`), ...ISSUE_KINDS.map((k) => `issue.${k}`), ...reasons.map((r) => `reason.${r}`)];
    for (const k of keys) {
      expect(tr[k], k).toBeTruthy();
      expect(en[k], k).toBeTruthy();
    }
  });
});
