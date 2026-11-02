import { describe, expect, it } from "vitest";
import en from "../i18n/en";
import tr from "../i18n/tr";
import { CATEGORIES, presetRange, shiftDay } from "../lib/logapi";
import { NAV } from "../nav";

const sources = import.meta.glob(["../**/*.tsx", "../**/*.ts", "!../__tests__/**"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

describe("log i18n", () => {
  it("every literal t() key used in the UI exists in Turkish and English", () => {
    const used = new Set<string>();
    for (const text of Object.values(sources)) {
      for (const m of text.matchAll(/\bt\("([a-zA-Z0-9_.]+)"/g)) used.add(m[1]);
    }
    expect(used.size).toBeGreaterThan(150);
    const missing = [...used].filter((k) => !(k in tr) || !(k in en));
    expect(missing).toEqual([]);
  });
  it("dynamic key families are complete", () => {
    const families: Record<string, string[]> = {
      cat: [...CATEGORIES, "fake", "human"],
      "budget.cat": ["parameters", "redirects", "client_errors", "server_errors", "static", "non_indexable"],
      "budget.rec": ["parameters", "redirects", "client_errors", "server_errors", "static", "non_indexable"],
      "budget.scope": ["googlebot", "search", "all"],
      fmt: ["combined", "common", "json", "cloudflare", "iis", "cloudfront", "alb"],
      "imp.phase": ["parse", "verify", "finalize", "done"],
      "ai.verdict": ["allowed", "partial", "blocked"],
      "ai.matched": ["token", "wildcard", "none"],
      device: ["mobile", "desktop", "tablet", "other"],
    };
    for (const [prefix, keys] of Object.entries(families)) {
      for (const k of keys) expect(tr[`${prefix}.${k}`], `${prefix}.${k}`).toBeTruthy();
    }
    for (const item of NAV) if ("id" in item) expect(tr[item.label]).toBeTruthy();
  });
});

describe("date presets", () => {
  it("shifts days across month boundaries", () => {
    expect(shiftDay("2026-03-01", 1)).toBe("2026-02-28");
    expect(shiftDay("2026-10-28", 27)).toBe("2026-10-01");
  });
  it("builds ranges relative to the newest log day", () => {
    const bounds = { minDay: "2026-10-01", maxDay: "2026-10-28" };
    expect(presetRange("7", bounds)).toEqual({ from: "2026-10-22", to: "2026-10-28" });
    expect(presetRange("90", bounds)).toEqual({ from: "2026-10-01", to: "2026-10-28" });
    expect(presetRange("all", bounds)).toEqual({ from: null, to: null });
    expect(presetRange("7", { minDay: null, maxDay: null })).toEqual({ from: null, to: null });
  });
});
