//! Seobify Migrate MCP tool set and database migrations.
//!
//! Tool tables live in `MIGRATIONS` (append only — never edit a released entry). Every MCP
//! tool calls the same `migrate::service` function as the GUI. Shared hub tools (hub_info,
//! list_projects, create_project, spider_projects, spider_urls) are added by `hub::mcp`.

use crate::hub::mcp::{self, blocking, BoxFuture, ToolSet};
use crate::hub::{db, spider::ImportSource};
use crate::migrate::model::{MappingRow, MatchOptions, Plan, PlanInput};
use crate::migrate::rules::{self, ExportOptions};
use crate::migrate::service;
use crate::migrate::store::{self, MappingFilter};
use serde_json::{json, Value};

/// Tool schema migrations, applied in order and tracked by `PRAGMA user_version`.
/// `plans` references `projects(id) ON DELETE CASCADE`; every other table cascades from `plans`.
pub const MIGRATIONS: &[&str] = &[
    // 1: plans, URL sets, patterns, mappings, verification runs.
    "CREATE TABLE IF NOT EXISTS plans (
        id INTEGER PRIMARY KEY,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        old_base TEXT NOT NULL,
        new_base TEXT NOT NULL,
        staging_base TEXT,
        notes TEXT,
        options_json TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        UNIQUE(project_id, name)
    );
    CREATE TABLE IF NOT EXISTS plan_urls (
        id INTEGER PRIMARY KEY,
        plan_id INTEGER NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
        side TEXT NOT NULL CHECK(side IN ('old','new')),
        url TEXT NOT NULL,
        path TEXT NOT NULL,
        slug TEXT NOT NULL DEFAULT '',
        title TEXT,
        h1 TEXT,
        meta_description TEXT,
        status_code INTEGER,
        indexability TEXT,
        canonical TEXT,
        word_count INTEGER,
        simhash TEXT,
        value REAL,
        UNIQUE(plan_id, side, url)
    );
    CREATE INDEX IF NOT EXISTS idx_plan_urls_side ON plan_urls(plan_id, side);
    CREATE TABLE IF NOT EXISTS patterns (
        id INTEGER PRIMARY KEY,
        plan_id INTEGER NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        regex TEXT NOT NULL,
        replacement TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        note TEXT,
        created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_patterns_plan ON patterns(plan_id, position);
    CREATE TABLE IF NOT EXISTS mappings (
        id INTEGER PRIMARY KEY,
        plan_id INTEGER NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
        old_url_id INTEGER NOT NULL REFERENCES plan_urls(id) ON DELETE CASCADE,
        new_url TEXT,
        status TEXT NOT NULL CHECK(status IN ('auto','review','weak','manual','gone','keep')),
        confidence INTEGER NOT NULL DEFAULT 0,
        reason TEXT NOT NULL DEFAULT '',
        candidates_json TEXT NOT NULL DEFAULT '[]',
        http_status INTEGER NOT NULL DEFAULT 301,
        approved INTEGER NOT NULL DEFAULT 0,
        updated_at TEXT NOT NULL,
        UNIQUE(plan_id, old_url_id)
    );
    CREATE INDEX IF NOT EXISTS idx_mappings_status ON mappings(plan_id, status);
    CREATE TABLE IF NOT EXISTS verify_runs (
        id INTEGER PRIMARY KEY,
        plan_id INTEGER NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
        kind TEXT NOT NULL CHECK(kind IN ('post','staging')),
        started_at TEXT NOT NULL,
        finished_at TEXT,
        total INTEGER NOT NULL DEFAULT 0,
        ok INTEGER NOT NULL DEFAULT 0,
        failed INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS verify_results (
        id INTEGER PRIMARY KEY,
        run_id INTEGER NOT NULL REFERENCES verify_runs(id) ON DELETE CASCADE,
        mapping_id INTEGER,
        source_url TEXT NOT NULL,
        expected_url TEXT,
        checked_url TEXT NOT NULL,
        outcome TEXT NOT NULL,
        hops_json TEXT NOT NULL DEFAULT '[]',
        final_status INTEGER,
        final_url TEXT,
        error TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_verify_results_run ON verify_results(run_id);",
];

pub struct Tools;

/// Tool-specific MCP tools (the shared hub tools are listed by `hub::mcp`).
pub const TOOL_NAMES: &[&str] = &[
    "list_plans",
    "create_plan",
    "import_urls",
    "auto_match",
    "list_mappings",
    "set_mapping",
    "add_pattern",
    "validate_plan",
    "export_rules",
    "verify",
    "generate_report",
];

fn plan_schema() -> Value {
    json!({ "type": ["string", "integer"], "description": "Plan id or name (see list_plans)." })
}

fn source_schema() -> Value {
    json!({
        "type": "object",
        "description": "URL source: {kind:'spider', project:'default', crawlId:158} (newest completed crawl when crawlId is omitted) | {kind:'file', path:'C:/…/urls.csv'} (CSV/TXT/sitemap.xml; CSV may carry title/h1/status columns) | {kind:'sitemap', url:'https://…/sitemap.xml'} | {kind:'list', urls:['https://…']}. For side='values' use {kind:'file', path} with a URL column and a numeric column (clicks, referring domains, value).",
        "properties": {
            "kind": { "type": "string", "enum": ["spider", "file", "sitemap", "list"] },
            "project": { "type": "string", "description": "Spider project (kind=spider)." },
            "crawlId": { "type": "integer", "description": "Spider crawl id (kind=spider)." },
            "path": { "type": "string", "description": "Local file path (kind=file)." },
            "url": { "type": "string", "description": "Sitemap URL (kind=sitemap)." },
            "urls": { "type": "array", "items": { "type": "string" }, "description": "URLs (kind=list)." }
        },
        "required": ["kind"]
    })
}

fn has_project(args: &Value) -> bool {
    args.get("project").map_or(false, |p| !p.is_null() && p.as_str() != Some(""))
}

/// Resolves `plan` (id or name; optional `project` narrows name lookups).
fn plan_arg(conn: &rusqlite::Connection, args: &Value) -> Result<Plan, String> {
    let key = args
        .get("plan")
        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
        .filter(|s| !s.trim().is_empty() && s != "null")
        .ok_or("Missing required argument: plan (id or name; see list_plans)")?;
    let project_id = if has_project(args) { Some(mcp::project_arg(conn, args)?.id) } else { None };
    store::find_plan(conn, &key, project_id)
}

async fn plan_id(args: &Value) -> Result<i64, String> {
    let a = args.clone();
    blocking(move || {
        let conn = db::open()?;
        Ok(plan_arg(&conn, &a)?.id)
    })
    .await
}

fn parse_source(v: Option<&Value>) -> Result<ImportSource, String> {
    let mut v = v.cloned().ok_or("Missing required argument: source ({kind:'spider'|'file'|'sitemap'|'list', …})")?;
    if let Some(obj) = v.as_object_mut() {
        if let Some(c) = obj.remove("crawl_id") {
            obj.entry("crawlId").or_insert(c);
        }
    }
    serde_json::from_value(v).map_err(|e| format!("Invalid source: {e}. Example: {{\"kind\":\"spider\",\"project\":\"default\",\"crawlId\":158}}"))
}

fn compact_mapping(m: &MappingRow) -> Value {
    json!({
        "id": m.id,
        "oldUrl": m.old_url,
        "oldTitle": m.old_title,
        "newUrl": m.new_url,
        "newTitle": m.new_title,
        "status": m.status,
        "confidence": m.confidence,
        "reason": m.reason,
        "approved": m.approved,
        "value": m.value,
        "candidates": m.candidates.iter().map(|c| json!({ "url": c.url, "score": c.score, "reason": c.reason })).collect::<Vec<_>>(),
    })
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

impl ToolSet for Tools {
    fn instructions(&self) -> String {
        "Seobify Migrate maps old URLs to new ones for site migrations, writes server redirect rules and verifies them. \
Typical chain: list_projects/create_project → create_plan(project, name, old_base, new_base, staging_base?) → import_urls(plan, side='old', source={kind:'spider', project:'default', crawlId}) and import_urls(plan, side='new', source=…) (optional side='values' for a clicks/backlinks CSV) → auto_match(plan) → list_mappings(plan, status='review' or 'weak') and set_mapping(plan, old_url, new_url | status='gone'|'keep'|'approve') → add_pattern(plan, regex, replacement, apply=true) for directory moves → validate_plan(plan, collapse_chains=true) → export_rules(plan, format='apache'|'nginx'|'iis'|'cloudflare'|'netlify'|'vercel'|'nextjs'|'wordpress'|'csv') → verify(plan, kind='staging' before launch or 'post' after launch) → generate_report(plan). \
Statuses: auto (confidence ≥ 90), review (60–89), weak (< 60), manual, gone (410), keep (same URL). Exports and verification use approved mappings plus unapproved auto ones (include_unapproved_auto=false exports approved mappings only)."
            .into()
    }

    fn tools(&self) -> Vec<Value> {
        vec![
            mcp::tool("list_plans", "Lists migration plans with URL, mapping, approval and pattern counts.", json!({ "project": mcp::project_schema() }), &[]),
            mcp::tool(
                "create_plan",
                "Creates a migration plan in a project: old site base, new site base and optional staging base (e.g. https://staging.new.com).",
                json!({
                    "project": mcp::project_schema(),
                    "name": { "type": "string", "description": "Plan name, e.g. \"2026 WordPress → Astro\"." },
                    "old_base": { "type": "string", "description": "Old site base URL, e.g. https://old.com" },
                    "new_base": { "type": "string", "description": "New site base URL, e.g. https://new.com" },
                    "staging_base": { "type": "string", "description": "Pre-launch test host (optional)." },
                    "notes": { "type": "string" },
                    "case_insensitive": { "type": "boolean", "description": "Compare paths case-insensitively (default true)." },
                    "ignore_query": { "type": "boolean", "description": "Ignore query strings when matching and in rules (default true)." },
                    "gone_without_value": { "type": "boolean", "description": "Unmatched URLs without a priority value get 410 instead of a parent/homepage fallback (default false)." }
                }),
                &["project", "name", "old_base", "new_base"],
            ),
            mcp::tool(
                "import_urls",
                "Fills the old or the new URL set of a plan from a Spider crawl, file, sitemap or list (title, H1, meta, status and indexability come along from Spider/CSV). side='values' imports priority values (clicks, referring domains) for old URLs. fetch_content=true downloads pages without title/H1 and computes a content fingerprint (max 4 parallel).",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "side": { "type": "string", "enum": ["old", "new", "values"] },
                    "source": source_schema(),
                    "replace": { "type": "boolean", "description": "Clear this side (and its mappings) first (default false = merge)." },
                    "fetch_content": { "type": "boolean", "description": "Fetch pages missing title/H1 after the import (default false)." },
                    "include_resources": { "type": "boolean", "description": "Keep non-HTML resources such as JS/JSON/images (default false)." }
                }),
                &["plan", "side", "source"],
            ),
            mcp::tool(
                "auto_match",
                "Runs the matching engine (patterns → same path → same slug → same title/H1 → fuzzy score → one-to-one → parent/homepage/410 fallbacks) and returns the status distribution.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "rematch": { "type": "boolean", "description": "true (default): recompute every mapping except user decisions; false: only URLs without a mapping." }
                }),
                &["plan"],
            ),
            mcp::tool(
                "list_mappings",
                "Lists mappings (highest priority value first) with confidence, reason, approval and the best candidates.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "status": { "type": "string", "enum": ["auto", "review", "weak", "manual", "gone", "keep"] },
                    "min_confidence": { "type": "integer" },
                    "max_confidence": { "type": "integer" },
                    "directory": { "type": "string", "description": "Old path prefix, e.g. /blog" },
                    "query": { "type": "string", "description": "Search in URLs and titles." },
                    "approved": { "type": "boolean" },
                    "limit": { "type": "integer", "description": "Max rows (default 50, max 1000)." }
                }),
                &["plan"],
            ),
            mcp::tool(
                "set_mapping",
                "Sets a user decision for one old URL: new_url = manual mapping; status 'gone' = 410, 'keep' = no redirect, 'approve'/'unapprove' = confirm the current suggestion. Without old_url, status='approve' with min_confidence approves every mapping at or above that confidence.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "old_url": { "type": "string", "description": "Old URL (full URL or path) or #<mapping id>." },
                    "new_url": { "type": "string", "description": "Target URL or path on the new site." },
                    "status": { "type": "string", "enum": ["manual", "gone", "keep", "approve", "unapprove"] },
                    "min_confidence": { "type": "integer", "description": "Bulk approve threshold (with status='approve' and no old_url)." }
                }),
                &["plan"],
            ),
            mcp::tool(
                "add_pattern",
                "Adds a regex pattern rule (applied in order, first match wins), e.g. regex ^/blog/\\d{4}/\\d{2}/(.+)$ → replacement /makale/$1. Returns how many old URLs it matches and how many targets exist; apply=true re-runs auto_match.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "regex": { "type": "string", "description": "Regex on the decoded old path (no trailing slash)." },
                    "replacement": { "type": "string", "description": "Target path or URL; $1…$9 insert capture groups." },
                    "note": { "type": "string" },
                    "apply": { "type": "boolean", "description": "Re-run auto_match after adding (default false)." }
                }),
                &["plan", "regex", "replacement"],
            ),
            mcp::tool(
                "validate_plan",
                "Checks mappings for chains (A→B→C), loops, targets missing from the new set, targets with a non-200 status, self redirects, case/trailing-slash-only rules and shared targets. collapse_chains=true rewrites chains to a single hop first.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "collapse_chains": { "type": "boolean" },
                    "limit": { "type": "integer", "description": "Max issues listed (default 50)." }
                }),
                &["plan"],
            ),
            mcp::tool(
                "export_rules",
                "Writes redirect rules for a server or platform and returns the file path, counts, warnings and a preview. Turkish characters are percent-encoded in targets and regex-escaped in rewrite sources.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "format": { "type": "string", "enum": rules::format_ids() },
                    "path": { "type": "string", "description": "Output file or folder (default: <reports>/exports/<project>/<plan>/<file name>)." },
                    "include_unapproved_auto": { "type": "boolean", "description": "Also export unapproved auto mappings (default true)." },
                    "preserve_query": { "type": "boolean", "description": "Keep visitors' query strings (default true)." },
                    "lang": { "type": "string", "enum": ["tr", "en"], "description": "Comment language (default tr)." }
                }),
                &["plan", "format"],
            ),
            mcp::tool(
                "verify",
                "Verifies redirects over HTTP (max 4 parallel, polite delay). kind='post': each old URL must answer one 301/308 to the expected target, which answers 200 (outcomes: ok, wrong_target, chain, temporary, not_redirected, target_error, loop, error). kind='staging': targets moved to the staging host must answer 200. Stores the run and compares it with the previous run of the same kind.",
                json!({
                    "plan": plan_schema(),
                    "project": mcp::project_schema(),
                    "kind": { "type": "string", "enum": ["post", "staging"] },
                    "limit": { "type": "integer", "description": "Max URLs checked, highest value first (default 500)." },
                    "delay_ms": { "type": "integer", "description": "Delay per request in ms (default 100)." },
                    "include_unapproved_auto": { "type": "boolean", "description": "Also verify unapproved auto mappings (default true)." }
                }),
                &["plan"],
            ),
            mcp::tool(
                "generate_report",
                "Saves the branded HTML report (plan summary, mapping statistics, validation, last verification, recommendations) and a mappings CSV; returns both paths.",
                json!({ "plan": plan_schema(), "project": mcp::project_schema(), "lang": { "type": "string", "enum": ["tr", "en"] } }),
                &["plan"],
            ),
        ]
    }

    fn call<'a>(&'a self, name: &'a str, args: Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            match name {
                "list_plans" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let project = if has_project(&args) { Some(mcp::project_arg(&conn, &args)?.id) } else { None };
                        let plans = store::list_plans(&conn, project)?;
                        Ok(json!({ "total": plans.len(), "plans": plans }))
                    })
                    .await
                }
                "create_plan" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let project = mcp::project_arg(&conn, &args)?;
                        let d = MatchOptions::default();
                        let options = MatchOptions {
                            case_insensitive: mcp::arg_bool(&args, "case_insensitive").unwrap_or(d.case_insensitive),
                            ignore_query: mcp::arg_bool(&args, "ignore_query").unwrap_or(d.ignore_query),
                            gone_without_value: mcp::arg_bool(&args, "gone_without_value").unwrap_or(d.gone_without_value),
                            ..d
                        };
                        let input = PlanInput {
                            name: mcp::req_str(&args, "name")?.into(),
                            old_base: mcp::req_str(&args, "old_base")?.into(),
                            new_base: mcp::req_str(&args, "new_base")?.into(),
                            staging_base: mcp::arg_str(&args, "staging_base").map(str::to_string),
                            notes: mcp::arg_str(&args, "notes").map(str::to_string),
                            options: Some(options),
                        };
                        let plan = store::create_plan(&conn, project.id, &input)?;
                        Ok(json!({ "plan": plan, "project": project.name, "next": "import_urls(plan, side='old', …) and import_urls(plan, side='new', …)" }))
                    })
                    .await
                }
                "import_urls" => {
                    let plan_id = plan_id(&args).await?;
                    let side = mcp::req_str(&args, "side")?.to_string();
                    let source = parse_source(args.get("source"))?;
                    if side == "values" {
                        let ImportSource::File { path } = source else { return Err("side='values' needs source {kind:'file', path:'…csv'}".into()) };
                        return blocking(move || {
                            let mut conn = db::open()?;
                            let s = service::import_values(&mut conn, plan_id, std::path::Path::new(&path))?;
                            Ok(json!({ "side": "values", "rows": s.rows, "matched": s.matched, "unmatched": s.unmatched }))
                        })
                        .await;
                    }
                    let replace = mcp::arg_bool(&args, "replace").unwrap_or(false);
                    let include = mcp::arg_bool(&args, "include_resources").unwrap_or(false);
                    let summary = service::import_urls(plan_id, side.clone(), source, replace, include).await?;
                    let mut out = serde_json::to_value(&summary).map_err(|e| e.to_string())?;
                    if mcp::arg_bool(&args, "fetch_content").unwrap_or(false) {
                        let f = service::fetch_content(plan_id, Some(side), true, 2000, 100, None).await?;
                        out["contentFetch"] = serde_json::to_value(f).map_err(|e| e.to_string())?;
                    }
                    Ok(out)
                }
                "auto_match" => {
                    blocking(move || {
                        let mut conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let s = service::auto_match(&mut conn, plan.id, mcp::arg_bool(&args, "rematch").unwrap_or(true))?;
                        Ok(json!({
                            "plan": plan.name,
                            "processed": s.processed,
                            "keptDecisions": s.kept_decisions,
                            "elapsedMs": s.elapsed_ms as u64,
                            "matchRate": round3(s.match_rate),
                            "autoRate": round3(s.auto_rate),
                            "byStatus": s.stats.by_status,
                            "byReason": s.stats.by_reason,
                            "next": "list_mappings(plan, status='review') to check suggestions; validate_plan(plan) before export_rules",
                        }))
                    })
                    .await
                }
                "list_mappings" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let limit = mcp::arg_i64(&args, "limit").unwrap_or(50).clamp(1, 1000) as usize;
                        let filter = MappingFilter {
                            status: mcp::arg_str(&args, "status").map(str::to_string),
                            min_confidence: mcp::arg_i64(&args, "min_confidence"),
                            max_confidence: mcp::arg_i64(&args, "max_confidence"),
                            directory: mcp::arg_str(&args, "directory").map(str::to_string),
                            query: mcp::arg_str(&args, "query").map(str::to_string),
                            approved: mcp::arg_bool(&args, "approved"),
                            limit: None,
                        };
                        let (rows, total) = service::list_mappings(&conn, plan.id, &filter)?;
                        let stats = store::mapping_stats(&conn, plan.id)?;
                        Ok(json!({
                            "plan": plan.name,
                            "total": total,
                            "matching": rows.len(),
                            "byStatus": stats.by_status,
                            "approved": stats.approved,
                            "rows": rows.iter().take(limit).map(compact_mapping).collect::<Vec<_>>(),
                        }))
                    })
                    .await
                }
                "set_mapping" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let status = mcp::arg_str(&args, "status");
                        match mcp::arg_str(&args, "old_url") {
                            Some(old) => {
                                let m = service::set_mapping(&conn, plan.id, old, mcp::arg_str(&args, "new_url"), status)?;
                                Ok(json!({ "mapping": compact_mapping(&m) }))
                            }
                            None => {
                                let min = mcp::arg_i64(&args, "min_confidence");
                                if status != Some("approve") || min.is_none() {
                                    return Err("Pass old_url, or status='approve' with min_confidence for a bulk approval".into());
                                }
                                let n = service::approve(&conn, plan.id, &[], min)?;
                                Ok(json!({ "approved": n, "minConfidence": min }))
                            }
                        }
                    })
                    .await
                }
                "add_pattern" => {
                    blocking(move || {
                        let mut conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let regex = mcp::req_str(&args, "regex")?;
                        let replacement = mcp::req_str(&args, "replacement")?;
                        let p = store::add_pattern(&conn, plan.id, regex, replacement, mcp::arg_str(&args, "note"))?;
                        let preview = service::preview_pattern(&conn, plan.id, regex, replacement, 10)?;
                        let mut out = json!({ "pattern": p, "matched": preview.matched, "targetsFound": preview.targets_found, "samples": preview.samples });
                        if mcp::arg_bool(&args, "apply").unwrap_or(false) {
                            let s = service::auto_match(&mut conn, plan.id, true)?;
                            out["autoMatch"] = json!({ "processed": s.processed, "byStatus": s.stats.by_status });
                        } else {
                            out["next"] = json!("auto_match(plan) to apply the pattern");
                        }
                        Ok(out)
                    })
                    .await
                }
                "validate_plan" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let collapsed = if mcp::arg_bool(&args, "collapse_chains").unwrap_or(false) { service::collapse_chains(&conn, plan.id)? } else { 0 };
                        let v = service::validate_plan(&conn, plan.id)?;
                        let limit = mcp::arg_i64(&args, "limit").unwrap_or(50).clamp(1, 1000) as usize;
                        let total = v.issues.len();
                        Ok(json!({ "plan": plan.name, "collapsedChains": collapsed, "counts": v.counts, "blocking": v.blocking, "totalIssues": total, "issues": v.issues.into_iter().take(limit).collect::<Vec<_>>() }))
                    })
                    .await
                }
                "export_rules" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let format = mcp::req_str(&args, "format")?.to_lowercase();
                        let opts = ExportOptions { preserve_query: mcp::arg_bool(&args, "preserve_query").unwrap_or(true), lang: mcp::arg_str(&args, "lang").unwrap_or("tr").to_string(), ..Default::default() };
                        let out = service::export(&conn, plan.id, &format, opts, mcp::arg_bool(&args, "include_unapproved_auto").unwrap_or(true))?;
                        let path = match mcp::arg_str(&args, "path") {
                            Some(p) => {
                                let p = std::path::PathBuf::from(p);
                                if p.is_dir() {
                                    p.join(&out.filename)
                                } else {
                                    p
                                }
                            }
                            None => service::default_export_path(&conn, &plan, &out.filename)?,
                        };
                        service::write_export(&out, &path)?;
                        db::log_activity(&conn, Some(plan.project_id), "export", &format!("{}: {} rules exported ({format})", plan.name, out.rules + out.pattern_rules))?;
                        let preview: Vec<&str> = out.content.lines().take(40).collect();
                        Ok(json!({
                            "plan": plan.name,
                            "format": out.format,
                            "path": path.display().to_string(),
                            "rules": out.rules,
                            "patternRules": out.pattern_rules,
                            "skipped": out.skipped,
                            "warnings": out.warnings,
                            "bytes": out.content.len(),
                            "preview": preview.join("\n"),
                        }))
                    })
                    .await
                }
                "verify" => {
                    let plan_id = plan_id(&args).await?;
                    let kind = mcp::arg_str(&args, "kind").unwrap_or("post").to_string();
                    let limit = mcp::arg_i64(&args, "limit").unwrap_or(500).clamp(1, 20_000) as usize;
                    let delay = mcp::arg_i64(&args, "delay_ms").unwrap_or(100).clamp(0, 10_000) as u64;
                    let s = service::verify(plan_id, kind, Some(limit), delay, mcp::arg_bool(&args, "include_unapproved_auto").unwrap_or(true), None).await?;
                    let failures: Vec<Value> = s
                        .results
                        .iter()
                        .filter(|r| r.outcome != "ok")
                        .take(50)
                        .map(|r| json!({ "source": r.source_url, "checked": r.checked_url, "expected": r.expected_url, "outcome": r.outcome, "finalStatus": r.final_status, "finalUrl": r.final_url, "error": r.error, "hops": r.hops }))
                        .collect();
                    let ok_sample: Vec<Value> = s.results.iter().filter(|r| r.outcome == "ok").take(5).map(|r| json!({ "source": r.source_url, "hops": r.hops })).collect();
                    Ok(json!({
                        "runId": s.run.id,
                        "kind": s.run.kind,
                        "total": s.run.total,
                        "ok": s.run.ok,
                        "failed": s.run.failed,
                        "successRate": round3(s.success_rate),
                        "byOutcome": s.by_outcome,
                        "elapsedMs": s.elapsed_ms as u64,
                        "failures": failures,
                        "okSample": ok_sample,
                        "comparison": s.comparison.map(|c| json!({ "previousRunId": c.previous_run.id, "fixed": c.fixed.len(), "broken": c.broken.len(), "stillFailing": c.still_failing, "newChecks": c.new_checks })),
                    }))
                }
                "generate_report" => {
                    blocking(move || {
                        let conn = db::open()?;
                        let plan = plan_arg(&conn, &args)?;
                        let lang = if mcp::arg_str(&args, "lang") == Some("en") { "en" } else { "tr" };
                        let paths = service::generate_report(&conn, plan.id, lang, None)?;
                        Ok(json!({ "plan": plan.name, "html": paths.html, "csv": paths.csv }))
                    })
                    .await
                }
                _ => Err(format!("Unknown tool: {name}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply() {
        let conn = crate::hub::db::open_memory().unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, super::MIGRATIONS.len());
    }

    #[test]
    fn tool_list_matches_names() {
        let names: Vec<String> = Tools.tools().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, TOOL_NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        for t in Tools.tools() {
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    /// Full MCP flow against a temporary data folder and a local HTTP server:
    /// create_project → create_plan → import_urls ×2 → add_pattern → auto_match →
    /// export_rules(nginx) → verify (twice, with run comparison) → validate → report.
    #[tokio::test(flavor = "multi_thread")]
    async fn mcp_flow_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SEOBIFY_DATA_DIR", dir.path());
        let base = crate::migrate::verify::test_server::start().await;
        async fn call(name: &str, args: Value) -> Value {
            let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
            let resp = mcp::handle_message(&Tools, msg).await.unwrap();
            assert_eq!(resp["result"]["isError"], false, "{name}: {}", resp["result"]["content"][0]["text"]);
            resp["result"]["structuredContent"].clone()
        }
        call("create_project", json!({ "name": "MCP test" })).await;
        let plan = call("create_plan", json!({ "project": "MCP test", "name": "Yerel", "old_base": base, "new_base": base })).await;
        let plan_id = plan["plan"]["id"].as_i64().unwrap();
        let olds: Vec<String> = ["/ok", "/temp", "/chain", "/missing"].iter().map(|p| format!("{base}{p}")).collect();
        let news: Vec<String> = ["/new-ok", "/new-temp", "/new-chain", "/new-missing"].iter().map(|p| format!("{base}{p}")).collect();
        let r = call("import_urls", json!({ "plan": plan_id, "side": "old", "source": { "kind": "list", "urls": olds } })).await;
        assert_eq!(r["inserted"], 4);
        let r = call("import_urls", json!({ "plan": "Yerel", "project": "MCP test", "side": "new", "source": { "kind": "list", "urls": news } })).await;
        assert_eq!(r["inserted"], 4);
        let p = call("add_pattern", json!({ "plan": plan_id, "regex": "^/(ok|temp|chain|missing)$", "replacement": "/new-$1" })).await;
        assert_eq!((p["matched"].as_i64(), p["targetsFound"].as_i64()), (Some(4), Some(4)));
        let m = call("auto_match", json!({ "plan": plan_id })).await;
        assert_eq!(m["processed"], 4);
        assert_eq!(m["byStatus"]["auto"], 4);
        let listed = call("list_mappings", json!({ "plan": plan_id, "limit": 10 })).await;
        assert_eq!(listed["total"], 4);
        let approved = call("set_mapping", json!({ "plan": plan_id, "status": "approve", "min_confidence": 90 })).await;
        assert_eq!(approved["approved"], 4);
        let out_path = dir.path().join("nginx.conf");
        let e = call("export_rules", json!({ "plan": plan_id, "format": "nginx", "path": out_path.display().to_string() })).await;
        assert_eq!((e["rules"].as_i64(), e["patternRules"].as_i64()), (Some(0), Some(1)));
        assert!(std::fs::read_to_string(&out_path).unwrap().contains("rewrite \"^/(ok|temp|chain|missing)$\" /new-$1 permanent;"));
        let v = call("verify", json!({ "plan": plan_id, "delay_ms": 0 })).await;
        assert_eq!(v["total"], 4);
        assert_eq!(v["byOutcome"]["ok"], 1);
        assert_eq!(v["byOutcome"]["temporary"], 1);
        assert_eq!(v["byOutcome"]["chain"], 1);
        assert_eq!(v["byOutcome"]["not_redirected"], 1);
        let v2 = call("verify", json!({ "plan": plan_id, "delay_ms": 0 })).await;
        assert_eq!(v2["comparison"]["stillFailing"], 3);
        let val = call("validate_plan", json!({ "plan": plan_id })).await;
        assert!(val["counts"].is_object());
        let rep = call("generate_report", json!({ "plan": plan_id, "lang": "en" })).await;
        assert!(std::path::Path::new(rep["html"].as_str().unwrap()).exists());
        let plans = call("list_plans", json!({ "project": "MCP test" })).await;
        assert_eq!(plans["total"], 1);
        let gone = call("set_mapping", json!({ "plan": plan_id, "old_url": "/missing", "status": "gone" })).await;
        assert_eq!(gone["mapping"]["status"], "gone");
    }
}
