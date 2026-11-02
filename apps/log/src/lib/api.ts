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
