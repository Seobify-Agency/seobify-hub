// Typed wrappers around Tauri commands. Hub commands here; add tool commands at the bottom.
import { invoke } from "@tauri-apps/api/core";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error(`"${cmd}" needs the desktop app (run: npm run tauri dev)`);
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    throw new Error(typeof err === "string" ? err : err instanceof Error ? err.message : JSON.stringify(err));
  }
}

export interface AppInfo {
  product: string;
  key: string;
  version: string;
  dataDir: string;
  database: string;
  reportsDir: string;
  spiderHome: string;
  mcp: { name: string; command: string; args: string[] };
}

export interface Project {
  id: number;
  name: string;
  domain: string | null;
  spiderProject: string | null;
  notes: string | null;
  createdAt: string;
}

export interface Activity {
  id: number;
  at: string;
  projectId: number | null;
  kind: string;
  message: string;
}

export interface SpiderProject {
  name: string;
  path: string | null;
  sizeBytes: number | null;
}

export interface SpiderCrawl {
  id: number;
  name: string | null;
  startUrl: string | null;
  status: string | null;
  urlsDone: number | null;
  createdAt: string | null;
  finishedAt: string | null;
}

export interface CrawledUrl {
  url: string;
  finalUrl?: string;
  statusCode?: number;
  contentType?: string;
  indexability?: string;
  indexabilityReason?: string;
  title?: string;
  metaDescription?: string;
  h1?: string;
  canonical?: string;
  metaRobots?: string;
  wordCount?: number;
  depth?: number;
  responseTimeMs?: number;
}

export type ImportSource =
  | { kind: "spider"; project: string; crawlId?: number | null }
  | { kind: "file"; path: string }
  | { kind: "sitemap"; url: string }
  | { kind: "list"; urls: string[] };

export interface ImportPreview {
  source: string;
  total: number;
  preview: CrawledUrl[];
}

/** Report blocks rendered by the Rust report module (see hub/report.rs). */
export type ReportBlock =
  | { type: "heading"; text: string }
  | { type: "paragraph"; text: string }
  | { type: "stats"; items: [string, string][] }
  | { type: "table"; headers: string[]; rows: string[][] }
  | { type: "findings"; items: [string, string][] }
  | { type: "rawHtml"; html: string };

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  listProjects: () => call<Project[]>("list_projects"),
  createProject: (name: string, domain?: string, spiderProject?: string) =>
    call<Project>("create_project", { name, domain: domain || null, spiderProject: spiderProject || null }),
  deleteProject: (id: number) => call<void>("delete_project", { id }),
  recentActivity: (projectId?: number | null, limit = 50) => call<Activity[]>("recent_activity", { projectId: projectId ?? null, limit }),
  getSettings: () => call<Record<string, unknown>>("get_settings"),
  saveSettings: (settings: Record<string, unknown>) => call<void>("save_settings", { settings }),
  spiderProjects: () => call<{ source: "api" | "files"; projects: SpiderProject[] }>("spider_projects"),
  spiderCrawls: (project: string) => call<{ source: "api" | "files"; crawls: SpiderCrawl[] }>("spider_crawls", { project }),
  importPreview: (source: ImportSource, limit = 50) => call<ImportPreview>("import_preview", { source, limit }),
  writeTextFile: (path: string, contents: string, bom = false) => call<void>("write_text_file", { path, contents, bom }),
  saveReport: (title: string, blocks: ReportBlock[], lang: string, subtitle?: string, dest?: string) =>
    call<string>("save_report", { title, subtitle: subtitle ?? null, lang, blocks, dest: dest ?? null }),
};

// ---------------------------------------------------------------- tool commands
// Seobify Migrate: plans, URL sets, mappings, patterns, validation, exports, verification.

export interface MatchOptions {
  caseInsensitive: boolean;
  ignoreQuery: boolean;
  goneWithoutValue: boolean;
  fuzzyMin: number;
}

export interface Plan {
  id: number;
  projectId: number;
  name: string;
  oldBase: string;
  newBase: string;
  stagingBase: string | null;
  notes: string | null;
  options: MatchOptions;
  createdAt: string;
  updatedAt: string;
}

export interface PlanSummary extends Plan {
  oldUrls: number;
  newUrls: number;
  mapped: number;
  approved: number;
  patterns: number;
}

export interface PlanInput {
  name: string;
  oldBase: string;
  newBase: string;
  stagingBase?: string | null;
  notes?: string | null;
  options?: MatchOptions;
}

export type Side = "old" | "new";

export interface UrlRow {
  id: number;
  side: Side;
  url: string;
  path: string;
  slug: string;
  title: string | null;
  h1: string | null;
  metaDescription: string | null;
  statusCode: number | null;
  indexability: string | null;
  canonical: string | null;
  wordCount: number | null;
  simhash: string | null;
  value: number | null;
}

export interface SideStats {
  total: number;
  withTitle: number;
  status200: number;
  non200: number;
  indexable: number;
  withValue: number;
  withSimhash: number;
  totalValue: number;
}

export interface ImportSummary {
  side: Side;
  source: string;
  received: number;
  inserted: number;
  updated: number;
  skippedResources: number;
  total: number;
  stats: SideStats;
}

export interface Breakdown {
  slug?: number;
  path?: number;
  title?: number;
  simhash?: number;
  depth?: number;
  dir?: number;
}

export interface Candidate {
  url: string;
  score: number;
  reason: string;
  breakdown: Breakdown;
}

export type MappingStatus = "auto" | "review" | "weak" | "manual" | "gone" | "keep";

export interface MappingRow {
  id: number;
  oldUrlId: number;
  oldUrl: string;
  oldTitle: string | null;
  oldStatus: number | null;
  value: number | null;
  newUrl: string | null;
  newTitle: string | null;
  newStatus: number | null;
  status: MappingStatus;
  confidence: number;
  reason: string;
  approved: boolean;
  httpStatus: number;
  candidates: Candidate[];
  updatedAt: string;
}

export interface MappingStats {
  total: number;
  byStatus: Record<string, number>;
  byReason: Record<string, number>;
  approved: number;
  unmapped: number;
}

export interface MatchSummary {
  processed: number;
  keptDecisions: number;
  elapsedMs: number;
  stats: MappingStats;
  matchRate: number;
  autoRate: number;
}

export interface Pattern {
  id: number;
  planId: number;
  position: number;
  regex: string;
  replacement: string;
  enabled: boolean;
  note: string | null;
}

export interface PatternWithCounts extends Pattern {
  matched: number;
  targetsFound: number;
}

export interface PatternPreview {
  matched: number;
  targetsFound: number;
  samples: { oldUrl: string; target: string; exists: boolean }[];
}

export interface PatternSuggestion {
  regex: string;
  replacement: string;
  support: number;
  affected: number;
  examples: [string, string][];
}

export interface Issue {
  kind: string;
  mappingId: number;
  source: string;
  target: string | null;
  detail: string | null;
}

export interface Validation {
  counts: Record<string, number>;
  issues: Issue[];
  blocking: number;
}

export interface ExportOptions {
  preserveQuery: boolean;
  includeHeader?: boolean | null;
  lang: string;
}

export interface ExportOutput {
  format: string;
  filename: string;
  content: string;
  rules: number;
  patternRules: number;
  skipped: number;
  warnings: string[];
  bom: boolean;
}

export interface ExportFormat {
  id: string;
  filename: string;
  label: string;
}

export interface Hop {
  url: string;
  status: number;
  location?: string;
}

export type VerifyKind = "post" | "staging";

export interface VerifyRun {
  id: number;
  planId: number;
  kind: VerifyKind;
  startedAt: string;
  finishedAt: string | null;
  total: number;
  ok: number;
  failed: number;
}

export interface VerifyResultRow {
  mappingId: number | null;
  sourceUrl: string;
  expectedUrl: string | null;
  checkedUrl: string;
  outcome: string;
  hops: Hop[];
  finalStatus: number | null;
  finalUrl: string | null;
  error: string | null;
}

export interface Comparison {
  previousRun: VerifyRun;
  currentRun: VerifyRun;
  fixed: VerifyResultRow[];
  broken: VerifyResultRow[];
  stillFailing: number;
  newChecks: number;
}

export interface VerifySummary {
  run: VerifyRun;
  successRate: number;
  byOutcome: Record<string, number>;
  results: VerifyResultRow[];
  comparison: Comparison | null;
  elapsedMs: number;
}

export interface Overview {
  plan: Plan;
  old: SideStats;
  new: SideStats;
  stats: MappingStats;
  patterns: number;
  validation: Record<string, number>;
  blockingIssues: number;
  matchRate: number;
  approvalRate: number;
  lastPost: VerifyRun | null;
  lastStaging: VerifyRun | null;
}

export interface FetchSummary {
  requested: number;
  updated: number;
  failed: number;
  errors: string[];
}

export interface ValuesSummary {
  rows: number;
  matched: number;
  unmatched: number;
}

export interface ReportPaths {
  html: string;
  csv: string;
}

/** Progress payload of the `migrate-progress` event. */
export interface ProgressEvent {
  task: "verify" | "fetch";
  done: number;
  total: number;
}

export const migrate = {
  listPlans: (projectId: number) => call<PlanSummary[]>("list_plans", { projectId }),
  createPlan: (projectId: number, input: PlanInput) => call<Plan>("create_plan", { projectId, input }),
  updatePlan: (id: number, input: PlanInput) => call<Plan>("update_plan", { id, input }),
  setPlanOptions: (planId: number, options: MatchOptions) => call<Plan>("set_plan_options", { planId, options }),
  deletePlan: (id: number) => call<void>("delete_plan", { id }),
  overview: (planId: number) => call<Overview>("plan_overview", { planId }),
  importUrls: (planId: number, side: Side, source: ImportSource, replace: boolean) => call<ImportSummary>("import_urls", { planId, side, source, replace }),
  listUrls: (planId: number, side: Side, query?: string, limit = 2000) => call<UrlRow[]>("list_urls", { planId, side, query: query ?? null, limit }),
  clearSide: (planId: number, side: Side) => call<number>("clear_side", { planId, side }),
  importValues: (planId: number, path: string) => call<ValuesSummary>("import_values", { planId, path }),
  fetchContent: (planId: number, side: Side | null, onlyMissing = true) => call<FetchSummary>("fetch_content", { planId, side, onlyMissing }),
  autoMatch: (planId: number, rematch = true) => call<MatchSummary>("auto_match", { planId, rematch }),
  listMappings: (planId: number) => call<{ rows: MappingRow[]; total: number }>("list_mappings", { planId, filter: null }),
  setMapping: (planId: number, mappingId: number, status: string | null, newUrl?: string | null) => call<MappingRow>("set_mapping", { planId, mappingId, status, newUrl: newUrl ?? null }),
  approve: (planId: number, ids: number[], minConfidence?: number | null) => call<number>("approve_mappings", { planId, ids, minConfidence: minConfidence ?? null }),
  searchNew: (planId: number, query: string, limit = 20) => call<UrlRow[]>("search_new_urls", { planId, query, limit }),
  listPatterns: (planId: number) => call<PatternWithCounts[]>("list_patterns", { planId }),
  addPattern: (planId: number, regex: string, replacement: string, note?: string) => call<Pattern>("add_pattern", { planId, regex, replacement, note: note || null }),
  updatePattern: (p: Pattern) => call<void>("update_pattern", { id: p.id, regex: p.regex, replacement: p.replacement, enabled: p.enabled, note: p.note }),
  deletePattern: (id: number) => call<void>("delete_pattern", { id }),
  movePattern: (id: number, delta: number) => call<void>("move_pattern", { id, delta }),
  previewPattern: (planId: number, regex: string, replacement: string) => call<PatternPreview>("preview_pattern", { planId, regex, replacement }),
  suggestPatterns: (planId: number) => call<PatternSuggestion[]>("suggest_patterns", { planId }),
  validate: (planId: number) => call<Validation>("validate_plan", { planId }),
  collapseChains: (planId: number) => call<number>("collapse_chains", { planId }),
  exportFormats: () => call<ExportFormat[]>("export_formats"),
  exportRules: (planId: number, format: string, options: ExportOptions, includeAuto: boolean) => call<ExportOutput>("export_rules", { planId, format, options, includeAuto }),
  saveExport: (planId: number, format: string, options: ExportOptions, includeAuto: boolean, path: string) => call<string>("save_export", { planId, format, options, includeAuto, path }),
  defaultExportPath: (planId: number, format: string) => call<string>("default_export_path", { planId, format }),
  verify: (planId: number, kind: VerifyKind, limit: number | null, delayMs: number, includeAuto: boolean) => call<VerifySummary>("verify_plan", { planId, kind, limit, delayMs, includeAuto }),
  listRuns: (planId: number) => call<VerifyRun[]>("list_runs", { planId }),
  runResults: (runId: number) => call<VerifyResultRow[]>("run_results", { runId }),
  compareRuns: (previous: number, current: number) => call<Comparison>("compare_runs", { previous, current }),
  report: (planId: number, lang: string) => call<ReportPaths>("generate_report", { planId, lang, dest: null }),
};
