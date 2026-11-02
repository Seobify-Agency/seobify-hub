// Pure helpers for the Migrate screens (unit-tested in src/__tests__/migrate.test.ts).
import type { Tone } from "../components/ui";
import type { MappingRow, MappingStatus } from "./api";

export const STATUSES: MappingStatus[] = ["auto", "review", "weak", "manual", "gone", "keep"];
export const OUTCOMES = ["ok", "wrong_target", "chain", "temporary", "not_redirected", "target_error", "loop", "error"] as const;
export const ISSUE_KINDS = ["loop", "chain", "missing_target", "target_not_200", "self_redirect", "redundant", "multi_source"] as const;

export function statusTone(status: string): Tone {
  switch (status) {
    case "auto":
      return "success";
    case "review":
      return "warning";
    case "weak":
      return "error";
    case "manual":
      return "gold";
    case "gone":
      return "high";
    default:
      return "info";
  }
}

export function statusColor(status: string): string {
  switch (status) {
    case "auto":
      return "var(--success)";
    case "review":
      return "var(--warning)";
    case "weak":
      return "var(--error)";
    case "manual":
      return "var(--accent)";
    case "gone":
      return "var(--sev-high)";
    default:
      return "var(--info)";
  }
}

export function confidenceTone(confidence: number): Tone {
  return confidence >= 90 ? "success" : confidence >= 60 ? "warning" : "error";
}

export function outcomeTone(outcome: string): Tone {
  switch (outcome) {
    case "ok":
      return "success";
    case "chain":
    case "temporary":
      return "warning";
    case "loop":
    case "wrong_target":
      return "critical";
    default:
      return "error";
  }
}

export function issueTone(kind: string): Tone {
  switch (kind) {
    case "loop":
      return "critical";
    case "chain":
    case "missing_target":
      return "high";
    case "target_not_200":
    case "self_redirect":
      return "medium";
    case "redundant":
      return "low";
    default:
      return "info";
  }
}

/** `pattern:3` → `{ code: "pattern", n: "3" }`. */
export function splitReason(reason: string): { code: string; n: string | null } {
  const i = reason.indexOf(":");
  return i < 0 ? { code: reason || "none", n: null } : { code: reason.slice(0, i), n: reason.slice(i + 1) };
}

/** Path + query of a URL for compact display (`https://a.com/x?y` → `/x?y`). */
export function pathOf(url: string | null | undefined): string {
  if (!url) return "";
  try {
    const u = new URL(url);
    return decodeURIComponentSafe(u.pathname) + u.search;
  } catch {
    return url;
  }
}

function decodeURIComponentSafe(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** First path segment as a directory (`https://a.com/blog/x` → `/blog`, root → `/`). */
export function topDirectory(url: string): string {
  const path = pathOf(url).split("?")[0];
  const seg = path.split("/").filter(Boolean);
  return seg.length > 1 ? `/${seg[0]}` : "/";
}

export interface MappingFilterState {
  status: string;
  min: number;
  max: number;
  directory: string;
  query: string;
  approval: "all" | "approved" | "pending";
}

export const DEFAULT_FILTER: MappingFilterState = { status: "all", min: 0, max: 100, directory: "", query: "", approval: "all" };

export function filterMappings(rows: MappingRow[], f: MappingFilterState, locale = "tr-TR"): MappingRow[] {
  const q = f.query.trim().toLocaleLowerCase(locale);
  return rows.filter((m) => {
    if (f.status !== "all" && m.status !== f.status) return false;
    if (m.confidence < f.min || m.confidence > f.max) return false;
    if (f.directory && topDirectory(m.oldUrl) !== f.directory) return false;
    if (f.approval === "approved" && !m.approved) return false;
    if (f.approval === "pending" && m.approved) return false;
    if (q) {
      const hay = `${m.oldUrl} ${pathOf(m.oldUrl)} ${m.oldTitle ?? ""} ${m.newUrl ?? ""} ${pathOf(m.newUrl)} ${m.newTitle ?? ""}`.toLocaleLowerCase(locale);
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}

/** Directory → count, most common first. */
export function directories(rows: MappingRow[]): { dir: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const m of rows) counts.set(topDirectory(m.oldUrl), (counts.get(topDirectory(m.oldUrl)) ?? 0) + 1);
  return [...counts.entries()].map(([dir, count]) => ({ dir, count })).sort((a, b) => b.count - a.count || a.dir.localeCompare(b.dir));
}

export function countBy<T>(rows: T[], key: (row: T) => string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const r of rows) out[key(r)] = (out[key(r)] ?? 0) + 1;
  return out;
}

/** Turns a directory move suggestion into a readable sentence fragment (`/urunler/` → `/magaza/`). */
export function describeMove(regex: string, replacement: string): { from: string; to: string } {
  const from = regex.replace(/^\^/, "").replace(/\(\.\+\)\$$/, "").replace(/\\d\{(\d+)\}/g, (_, n: string) => "#".repeat(Number(n))).replace(/\\(.)/g, "$1");
  const to = replacement.replace(/\$1$/, "");
  return { from, to };
}
