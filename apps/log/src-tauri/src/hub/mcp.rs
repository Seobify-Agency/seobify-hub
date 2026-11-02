//! Model Context Protocol server over stdio (JSON-RPC 2.0, newline-delimited).
//!
//! Every hub tool exposes the shared hub tools (`hub_info`, `list_projects`,
//! `create_project`, `spider_projects`, `spider_urls`) plus its own [`ToolSet`].
//! Results carry both a text block (pretty JSON) and `structuredContent`.

use serde_json::{json, Value};
use std::future::Future;
use std::io::{self, BufRead, Write};
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const DEFAULT_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// A tool's MCP surface. `call` receives only names listed by `tools()`.
pub trait ToolSet: Sync {
    /// One paragraph telling the model what this server is for and how to chain its tools.
    fn instructions(&self) -> String;
    /// Tool definitions (`name`, `description`, `inputSchema`); build them with [`tool`].
    fn tools(&self) -> Vec<Value>;
    /// Executes a tool. `Ok` payloads must be JSON objects; `Err` becomes an `isError` result.
    fn call<'a>(&'a self, name: &'a str, args: Value) -> BoxFuture<'a, Result<Value, String>>;
}

/// Builds a tool definition. `properties` is the JSON-schema `properties` object.
pub fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false }
    })
}

pub fn serve_stdio<T: ToolSet>(toolset: &T) {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => runtime.block_on(handle_message(toolset, message)),
            Err(e) => Some(error_response(Value::Null, -32700, &format!("Parse error: {e}"))),
        };
        if let Some(response) = response {
            if let Ok(text) = serde_json::to_string(&response) {
                let _ = out.write_all(text.as_bytes());
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
        }
    }
}

/// Handles one JSON-RPC message; `None` for notifications. Public for tests.
pub async fn handle_message<T: ToolSet>(toolset: &T, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = id.is_none();
    if method.is_empty() {
        return (!is_notification).then(|| error_response(id.unwrap_or(Value::Null), -32600, "Invalid Request: missing method"));
    }
    if method.starts_with("notifications/") {
        return None;
    }
    let outcome: Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => Ok(initialize(toolset, &params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": all_tools(toolset) })),
        "tools/call" => tools_call(toolset, &params).await,
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    if is_notification {
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, msg)) => error_response(id, code, &msg),
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize<T: ToolSet>(toolset: &T, params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL);
    let version = if SUPPORTED_PROTOCOLS.contains(&requested) { requested } else { DEFAULT_PROTOCOL };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": crate::MCP_SERVER_NAME, "title": crate::PRODUCT_NAME, "version": crate::VERSION },
        "instructions": format!(
            "{} Shared Seobify Hub tools: hub_info (data paths), list_projects / create_project (projects are referenced by id or name in every tool), spider_projects and spider_urls (read Seobify Spider crawls).",
            toolset.instructions()
        )
    })
}

fn all_tools<T: ToolSet>(toolset: &T) -> Vec<Value> {
    let mut tools = hub_tools();
    tools.extend(toolset.tools());
    tools
}

async fn tools_call<T: ToolSet>(toolset: &T, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().filter(|a| a.is_object()).unwrap_or_else(|| json!({}));
    let result = if HUB_TOOL_NAMES.contains(&name) {
        call_hub_tool(name, &args).await
    } else if toolset.tools().iter().any(|t| t["name"] == name) {
        toolset.call(name, args).await
    } else {
        return Err((-32602, format!("Unknown tool: {name}")));
    };
    Ok(match result {
        Ok(payload) => {
            let text = serde_json::to_string_pretty(&payload).unwrap_or_default();
            let structured = if payload.is_object() { payload } else { json!({ "result": payload }) };
            json!({ "content": [{ "type": "text", "text": text }], "structuredContent": structured, "isError": false })
        }
        Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    })
}

// ---------------------------------------------------------------- argument helpers

pub fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

pub fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    arg_str(args, key).ok_or_else(|| format!("Missing required argument: {key}"))
}

pub fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
}

pub fn arg_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
}

pub fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

pub fn arg_str_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(s)) => s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// Resolves the `project` argument (id or name) to a project row.
pub fn project_arg(conn: &rusqlite::Connection, args: &Value) -> Result<super::db::Project, String> {
    let key = args
        .get("project")
        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
        .filter(|s| !s.trim().is_empty() && s != "null")
        .ok_or("Missing required argument: project (id or name; see list_projects)")?;
    super::db::find_project(conn, &key)?.ok_or_else(|| format!("Project not found: {key} (see list_projects)"))
}

/// JSON-schema snippet for the standard `project` argument.
pub fn project_schema() -> Value {
    json!({ "type": ["string", "integer"], "description": "Project id or name (see list_projects)." })
}

/// Runs blocking work (SQLite, file parsing) off the async runtime.
pub async fn blocking<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("task failed: {e}"))?
}

// ---------------------------------------------------------------- shared hub tools

const HUB_TOOL_NAMES: &[&str] = &["hub_info", "list_projects", "create_project", "spider_projects", "spider_urls"];

fn hub_tools() -> Vec<Value> {
    vec![
        tool("hub_info", "Product, version, data folder and database path of this Seobify Hub tool.", json!({}), &[]),
        tool("list_projects", "Lists projects (sites/clients) stored in this tool.", json!({}), &[]),
        tool(
            "create_project",
            "Creates a project. Names are unique (case-insensitive).",
            json!({
                "name": { "type": "string", "description": "Project name, e.g. the client or site name." },
                "domain": { "type": "string", "description": "Primary domain, e.g. example.com (optional)." },
                "spider_project": { "type": "string", "description": "Matching Seobify Spider project name (optional)." }
            }),
            &["name"],
        ),
        tool(
            "spider_projects",
            "Lists Seobify Spider projects and their crawls (live Spider API first, then the local DuckDB files).",
            json!({ "project": { "type": "string", "description": "Spider project name; when given, its crawls are listed." } }),
            &[],
        ),
        tool(
            "spider_urls",
            "Reads crawled URLs (status, indexability, title, meta description, H1, canonical, robots) from a Seobify Spider crawl.",
            json!({
                "spider_project": { "type": "string", "description": "Spider project name (default: default)." },
                "crawl_id": { "type": "integer", "description": "Crawl id; defaults to the newest completed crawl." },
                "limit": { "type": "integer", "description": "Max rows returned (default 200)." }
            }),
            &[],
        ),
    ]
}

async fn call_hub_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "hub_info" => Ok(json!({
            "product": crate::PRODUCT_NAME,
            "version": crate::VERSION,
            "dataDir": super::paths::data_dir(),
            "database": super::paths::db_path(),
            "reportsDir": super::paths::reports_dir(),
            "spiderHome": super::paths::spider_home(),
        })),
        "list_projects" => blocking(|| {
            let conn = super::db::open()?;
            Ok(json!({ "projects": super::db::list_projects(&conn)? }))
        })
        .await,
        "create_project" => {
            let name = req_str(args, "name")?.to_string();
            let domain = arg_str(args, "domain").map(str::to_string);
            let spider = arg_str(args, "spider_project").map(str::to_string);
            blocking(move || {
                let conn = super::db::open()?;
                Ok(json!({ "project": super::db::create_project(&conn, &name, domain.as_deref(), spider.as_deref())? }))
            })
            .await
        }
        "spider_projects" => match arg_str(args, "project") {
            Some(project) => {
                let (crawls, source) = super::spider::crawls(project).await?;
                Ok(json!({ "project": project, "source": source, "crawls": crawls }))
            }
            None => {
                let (projects, source) = super::spider::projects().await?;
                Ok(json!({ "source": source, "projects": projects }))
            }
        },
        "spider_urls" => {
            let project = arg_str(args, "spider_project").unwrap_or("default").to_string();
            let limit = arg_i64(args, "limit").unwrap_or(200).clamp(1, 100_000) as usize;
            let (urls, crawl_id, source) = super::spider::urls(&project, arg_i64(args, "crawl_id")).await?;
            let total = urls.len();
            Ok(json!({ "spiderProject": project, "crawlId": crawl_id, "source": source, "total": total, "urls": urls.into_iter().take(limit).collect::<Vec<_>>() }))
        }
        _ => Err(format!("Unknown hub tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Empty;
    impl ToolSet for Empty {
        fn instructions(&self) -> String {
            "Test server.".into()
        }
        fn tools(&self) -> Vec<Value> {
            vec![tool("echo", "Echoes its input.", json!({ "text": { "type": "string" } }), &["text"])]
        }
        fn call<'a>(&'a self, _name: &'a str, args: Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async move { Ok(json!({ "echo": req_str(&args, "text")? })) })
        }
    }

    #[tokio::test]
    async fn protocol_roundtrip() {
        let init = handle_message(&Empty, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).await.unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle_message(&Empty, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.is_none());
        let list = handle_message(&Empty, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"hub_info") && names.contains(&"echo"));
        let call = handle_message(&Empty, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}})).await.unwrap();
        assert_eq!(call["result"]["structuredContent"]["echo"], "hi");
        let missing = handle_message(&Empty, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{}}})).await.unwrap();
        assert_eq!(missing["result"]["isError"], true);
        let unknown = handle_message(&Empty, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope"}})).await.unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }
}
