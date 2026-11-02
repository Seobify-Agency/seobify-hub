// Typed wrappers around the Seobify Log Tauri commands (same Rust functions as the MCP tools).
import { call } from "./api";

export type Category = "search" | "ai_training" | "ai_assistant" | "seo_tool" | "social" | "other_bot";
export const CATEGORIES: Category[] = ["search", "ai_training", "ai_assistant", "seo_tool", "social", "other_bot"];

export interface Bounds {
  minDay: string | null;
  maxDay: string | null;
  botHits: number;
  humanRequests: number;
  files: number;
}

export interface RangeArgs {
  from?: string | null;
  to?: string | null;
}

export interface Overview {
  range: { from: string; to: string };
  totalBotRequests: number;
  humanRequests: number;
  uniqueUrls: number;
  verification: { verified: number; fake: number; unverifiable: number; verifiedShare: number; fakeShare: number };
  status: { s2xx: number; s3xx: number; s4xx: number; s5xx: number };
  googlebot: { requests: number; smartphone: number; desktop: number; fakeClaims: number; smartphoneShare: number };
  byCategory: { category: string; requests: number }[];
  humanStatus: { class: string; requests: number }[];
  humanDevices: { device: string; requests: number }[];
  daily: { days: string[]; series: Record<string, number[]> };
  topUrls: { url: string; hits: number; googlebotHits: number; errors: number }[];
  bounds: Bounds;
}

export interface BotRow {
  botId: string;
  name: string;
  category: Category;
  owner: string;
  verifiable: boolean;
  requests: number;
  uniqueUrls: number;
  s2xx: number;
  s3xx: number;
  s4xx: number;
  s5xx: number;
  avgResponseMs: number | null;
  firstSeen: string | null;
  lastSeen: string | null;
  verified: number;
  fake: number;
  unverifiable: number;
  bytes: number | null;
}

export interface BotDetail {
  botId: string;
  name: string | null;
  days: string[];
  requests: number[];
  fake: number[];
  errors: number[];
  urls: { url: string; hits: number; s2xx: number; s3xx: number; errors: number; lastSeen: string; fake: number }[];
}

export interface UrlRow {
  url: string;
  hits: number;
  googlebotHits: number;
  lastGooglebot: string | null;
  firstSeen: string;
  lastSeen: string;
  s2xx: number;
  s3xx: number;
  s4xx: number;
  s5xx: number;
  bots: number;
  perDay: number;
}

export interface UrlFilter {
  bot?: string | null;
  status?: string | null;
  dir?: string | null;
  params?: string | null;
  search?: string | null;
  limit?: number | null;
}

export interface UrlsResult {
  total: number;
  limit: number;
  urls: UrlRow[];
  dirs: { dir: string; hits: number }[];
}

export interface Spike {
  day: string;
  count: number;
  mean: number;
  stdDev: number;
  threshold: number;
}

export interface StatusResult {
  days: string[];
  total4xx: number;
  total5xx: number;
  daily4xx: number[];
  daily5xx: number[];
  byBot4xx: Record<string, number[]>;
  byBot5xx: Record<string, number[]>;
  errorUrls: { url: string; status: number; hits: number; lastSeen: string; bots: string }[];
  codes: { status: number; hits: number }[];
  spikes5xx: Spike[];
  spikes4xx: Spike[];
}

export interface WasteCategory {
  key: string;
  hits: number;
  share: number;
  urls: number;
  examples: { url: string; hits: number }[];
}

export interface WasteResult {
  scope: string;
  totalHits: number;
  wasteHits: number;
  wasteShare: number;
  spiderLinked: boolean;
  categories: WasteCategory[];
  parameters: { param: string; hits: number; share: number; urls: number; examples: string[] }[];
  spider: { spiderProject: string; crawlId: number; startUrl: string | null } | null;
}

export interface GapsResult {
  window: { from: string; to: string; days: number };
  summary: {
    spiderPages: number;
    indexable: number;
    indexableCrawled: number;
    indexableUncrawled: number;
    uncrawledShare: number;
    orphanCandidates: number;
    orphanCandidates2xx: number;
    orphanHits: number;
    problemPagesCrawled: number;
    problemHits: number;
    googlebotUrls: number;
  };
  orphans: { url: string; googlebotHits: number; lastGooglebot: string | null; status: string; s2xx: number; s3xx: number; s4xx: number; s5xx: number }[];
  uncrawled: { url: string; path: string; title: string | null; depth: number | null; lastGooglebot: string | null }[];
  problems: { url: string; path: string; status: number | null; indexability: string | null; reason: string | null; canonicalDiffers: boolean; googlebotHits: number; lastGooglebot: string | null }[];
  spider: { spiderProject: string; crawlId: number; source: string; startUrl: string | null; spiderUrls: number; comparedPages: number; domain: string | null };
}

export interface AiResult {
  days: string[];
  training: number[];
  assistant: number[];
  trainingTotal: number;
  assistantTotal: number;
  bots: { botId: string; name: string; owner: string; category: string; requests: number; uniquePaths: number; verified: number; unverifiable: number; fake: number; lastSeen: string | null; bytes: number | null; topDirs: { dir: string; hits: number }[] }[];
  topUrls: { path: string; hits: number; bots: number }[];
}

export interface RobotsResult {
  url: string;
  finalUrl: string;
  status: number;
  fetchedAt: string;
  bytes: number;
  bots: { token: string; category: string; verdict: "allowed" | "partial" | "blocked"; matched: "token" | "wildcard" | "none"; rules: string[] }[];
}

export interface FakeResult {
  fakeRequests: number;
  unverifiableRequests: number;
  sources: number;
  byBot: { botId: string; name: string; fake: number; claims: number; fakeShare: number }[];
  groups: { ip: string; botId: string; claims: string; requests: number; firstSeen: string; lastSeen: string; uniquePaths: number; sampleUrls: string[] }[];
}

export interface ImportRow {
  id: number;
  path: string;
  name: string;
  format: string;
  sizeBytes: number;
  lines: number;
  parsed: number;
  failed: number;
  botHits: number;
  humanRequests: number;
  firstTs: string | null;
  lastTs: string | null;
  importedAt: string;
  durationMs: number;
  sampleErrors: { line: number; reason: string; text: string }[];
}

export interface ImportProgress {
  file: string;
  fileIndex: number;
  fileCount: number;
  phase: "parse" | "verify" | "finalize" | "done";
  lines: number;
  parsed: number;
  failed: number;
  botHits: number;
  bytesRead: number;
  bytesTotal: number;
  linesPerSec: number;
  elapsedMs: number;
}

export interface FileResult {
  path: string;
  name: string;
  status: "imported" | "duplicate" | "skipped" | "error";
  fileId: number | null;
  format: string | null;
  lines: number;
  parsed: number;
  failed: number;
  botHits: number;
  humanRequests: number;
  durationMs: number;
  message: string | null;
}

export interface ImportResult {
  summary: { files: FileResult[]; imported: number; duplicates: number; skipped: number; errors: number; lines: number; parsed: number; failed: number; botHits: number; humanRequests: number; durationMs: number; linesPerSec: number };
}

export interface RangeSource {
  source: string;
  owner: string;
  url: string;
  enabled: boolean;
  fetchedAt: string | null;
  prefixes: number;
  error: string | null;
}

export interface ReportResult {
  path: string;
  findings: { severity: string; code: string; text: string }[];
  spiderIncluded: boolean;
}

const r = (projectId: number, range: RangeArgs) => ({ projectId, from: range.from || null, to: range.to || null });

export const logApi = {
  bounds: (projectId: number) => call<Bounds>("log_bounds", { projectId }),
  overview: (projectId: number, range: RangeArgs) => call<Overview>("log_overview", r(projectId, range)),
  botStats: (projectId: number, range: RangeArgs, category?: string | null) => call<{ bots: BotRow[] }>("log_bot_stats", { ...r(projectId, range), category: category || null }),
  botDetail: (projectId: number, botId: string, range: RangeArgs) => call<BotDetail>("log_bot_detail", { ...r(projectId, range), botId }),
  urls: (projectId: number, range: RangeArgs, filter: UrlFilter) => call<UrlsResult>("log_urls", { ...r(projectId, range), filter }),
  status: (projectId: number, range: RangeArgs) => call<StatusResult>("log_status", r(projectId, range)),
  crawlWaste: (projectId: number, range: RangeArgs, scope: string) => call<WasteResult>("log_crawl_waste", { ...r(projectId, range), scope }),
  spiderGaps: (projectId: number, spiderProject: string | null, crawlId: number | null, days: number) =>
    call<GapsResult>("log_spider_gaps", { projectId, spiderProject, crawlId, days, limit: 5000 }),
  aiActivity: (projectId: number, range: RangeArgs) => call<AiResult>("log_ai_activity", r(projectId, range)),
  robots: (projectId: number) => call<RobotsResult>("log_robots", { projectId }),
  fakeBots: (projectId: number, range: RangeArgs) => call<FakeResult>("log_fake_bots", { ...r(projectId, range), limit: 500 }),
  imports: (projectId: number) => call<{ imports: ImportRow[] }>("log_imports", { projectId }),
  deleteImport: (projectId: number, fileId: number) => call<void>("log_delete_import", { projectId, fileId }),
  importLogs: (projectId: number, paths: string[], format: string | null, force: boolean) => call<ImportResult>("log_import", { projectId, paths, format, force }),
  report: (projectId: number, range: RangeArgs, lang: string, dest?: string | null) => call<ReportResult>("log_generate_report", { ...r(projectId, range), lang, dest: dest ?? null }),
  rangeStatus: () => call<{ sources: RangeSource[]; dnsCacheEntries: number }>("log_range_status"),
  refreshRanges: () => call<{ sources: RangeSource[] }>("log_refresh_ranges", { force: true }),
  clearIpCache: () => call<{ deleted: number }>("log_clear_ip_cache"),
  updateProject: (id: number, domain: string | null, spiderProject: string | null) => call<unknown>("update_project", { id, domain, spiderProject }),
  cfStatus: () => call<CfStatus>("log_cf_status"),
  cfSetToken: (token: string) => call<CfStatus>("log_cf_set_token", { token }),
  cfDeleteToken: () => call<CfStatus>("log_cf_delete_token"),
  cfLink: (projectId: number, zone: string) => call<unknown>("log_cf_link", { projectId, zone }),
  cfSync: (projectId: number, days?: number | null) => call<CfSyncResult>("log_cf_sync", { projectId, days: days ?? null }),
};

export interface CfZone {
  id: string;
  name: string;
  status: string;
  plan: string | null;
  accountId: string | null;
}

export interface CfLink {
  projectId: number;
  project: string;
  zoneId: string;
  zoneName: string;
  lastSyncAt: string | null;
  lastError: string | null;
  days: number;
  firstDay: string | null;
  lastDay: string | null;
  requests: number;
  botRequests: number;
}

export interface CfStatus {
  connected: boolean;
  masked?: string;
  zones?: CfZone[];
  error?: string | null;
  links: CfLink[];
}

export interface CfSyncResult {
  project: string;
  zone: string;
  firstSync: boolean;
  daysFetched: number;
  requests: number;
  botRequests: number;
  error: string | null;
}

export interface CfProgress {
  project: string;
  day: string;
  index: number;
  total: number;
}

/** Cloudflare dashboard page where API tokens are created. */
export const CF_TOKENS_URL = "https://dash.cloudflare.com/profile/api-tokens";

/** Day `n` days before `day` (YYYY-MM-DD, UTC arithmetic). */
export function shiftDay(day: string, n: number): string {
  const d = new Date(`${day}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() - n);
  return d.toISOString().slice(0, 10);
}

/** Range for a preset relative to the newest day with data. */
export function presetRange(preset: string, bounds: Pick<Bounds, "minDay" | "maxDay">): RangeArgs {
  if (!bounds.maxDay || preset === "all") return { from: null, to: null };
  const days = Number(preset);
  if (!Number.isFinite(days) || days <= 0) return { from: null, to: null };
  const from = shiftDay(bounds.maxDay, days - 1);
  return { from: bounds.minDay && from < bounds.minDay ? bounds.minDay : from, to: bounds.maxDay };
}

export const STATUS_COLORS = { s2xx: "var(--success)", s3xx: "var(--info)", s4xx: "var(--warning)", s5xx: "var(--error)" } as const;

export const CATEGORY_COLORS: Record<string, string> = {
  search: "var(--series-1)",
  ai_training: "var(--series-6)",
  ai_assistant: "var(--series-5)",
  seo_tool: "var(--series-2)",
  social: "var(--series-3)",
  other_bot: "var(--text-muted)",
  fake: "var(--error)",
  human: "var(--series-4)",
};
