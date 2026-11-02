//! SQLite storage shared by the GUI and the MCP server (WAL, busy timeout, so both can
//! open the same file at once). Hub tables are created idempotently on every open; tool
//! tables come from `crate::tools::MIGRATIONS`, tracked by `PRAGMA user_version`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

const HUB_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    domain TEXT,
    spider_project TEXT,
    notes TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS activity (
    id INTEGER PRIMARY KEY,
    at TEXT NOT NULL,
    project_id INTEGER,
    kind TEXT NOT NULL,
    message TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activity_at ON activity(at);
";

pub fn open() -> Result<Connection, String> {
    open_at(&super::paths::db_path())
}

pub fn open_at(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("DB open {}: {e}", path.display()))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=8000; PRAGMA synchronous=NORMAL;")
        .map_err(|e| format!("DB pragmas: {e}"))?;
    conn.execute_batch(HUB_SCHEMA).map_err(|e| format!("DB hub schema: {e}"))?;
    migrate(&conn, crate::tools::MIGRATIONS)?;
    Ok(conn)
}

/// In-memory DB with the full schema (tests).
pub fn open_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys=ON;").map_err(|e| e.to_string())?;
    conn.execute_batch(HUB_SCHEMA).map_err(|e| e.to_string())?;
    migrate(&conn, crate::tools::MIGRATIONS)?;
    Ok(conn)
}

fn migrate(conn: &Connection, migrations: &[&str]) -> Result<(), String> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    for (index, sql) in migrations.iter().enumerate().skip(current.max(0) as usize) {
        let version = index as i64 + 1;
        conn.execute_batch(&format!("BEGIN; {sql}\n; PRAGMA user_version = {version}; COMMIT;"))
            .map_err(|e| {
                let _ = conn.execute_batch("ROLLBACK;");
                format!("migration {version}: {e}")
            })?;
    }
    Ok(())
}

// ---------------------------------------------------------------- projects

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub domain: Option<String>,
    pub spider_project: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

fn row_to_project(r: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        domain: r.get(2)?,
        spider_project: r.get(3)?,
        notes: r.get(4)?,
        created_at: r.get(5)?,
    })
}

const PROJECT_COLS: &str = "id, name, domain, spider_project, notes, created_at";

pub fn list_projects(conn: &Connection) -> Result<Vec<Project>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {PROJECT_COLS} FROM projects ORDER BY name COLLATE NOCASE"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_project).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_project(conn: &Connection, id: i64) -> Result<Option<Project>, String> {
    conn.query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"), [id], row_to_project)
        .optional()
        .map_err(|e| e.to_string())
}

/// Finds a project by numeric id or by name (case-insensitive). MCP tools accept either.
pub fn find_project(conn: &Connection, key: &str) -> Result<Option<Project>, String> {
    if let Ok(id) = key.trim().parse::<i64>() {
        if let Some(p) = get_project(conn, id)? {
            return Ok(Some(p));
        }
    }
    conn.query_row(
        &format!("SELECT {PROJECT_COLS} FROM projects WHERE name = ?1 COLLATE NOCASE"),
        [key.trim()],
        row_to_project,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn create_project(conn: &Connection, name: &str, domain: Option<&str>, spider_project: Option<&str>) -> Result<Project, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Project name is required".into());
    }
    let domain = domain.map(super::text::normalize_domain).filter(|d| !d.is_empty());
    conn.execute(
        "INSERT INTO projects (name, domain, spider_project, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, domain, spider_project.filter(|s| !s.trim().is_empty()), super::now_iso()],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            format!("A project named \"{name}\" already exists")
        } else {
            e.to_string()
        }
    })?;
    let id = conn.last_insert_rowid();
    log_activity(conn, Some(id), "project", &format!("Project created: {name}"))?;
    get_project(conn, id)?.ok_or_else(|| "project vanished after insert".into())
}

pub fn update_project(conn: &Connection, p: &Project) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET name = ?2, domain = ?3, spider_project = ?4, notes = ?5 WHERE id = ?1",
        params![p.id, p.name.trim(), p.domain, p.spider_project, p.notes],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Deletes the project row. Tool tables reference `projects(id)` with `ON DELETE CASCADE`.
pub fn delete_project(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM projects WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- activity log

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: i64,
    pub at: String,
    pub project_id: Option<i64>,
    pub kind: String,
    pub message: String,
}

pub fn log_activity(conn: &Connection, project_id: Option<i64>, kind: &str, message: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO activity (at, project_id, kind, message) VALUES (?1, ?2, ?3, ?4)",
        params![super::now_iso(), project_id, kind, message],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn recent_activity(conn: &Connection, project_id: Option<i64>, limit: i64) -> Result<Vec<Activity>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, at, project_id, kind, message FROM activity
             WHERE (?1 IS NULL OR project_id = ?1) ORDER BY id DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![project_id, limit], |r| {
            Ok(Activity { id: r.get(0)?, at: r.get(1)?, project_id: r.get(2)?, kind: r.get(3)?, message: r.get(4)? })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- key/value + settings

pub fn kv_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())
}

pub fn kv_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Tool settings as one JSON object (never put secrets here — use `hub::secrets`).
pub fn get_settings(conn: &Connection) -> Result<Value, String> {
    Ok(kv_get(conn, "settings")?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({})))
}

pub fn save_settings(conn: &Connection, settings: &Value) -> Result<(), String> {
    if !settings.is_object() {
        return Err("settings must be a JSON object".into());
    }
    kv_set(conn, "settings", &settings.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_crud_and_lookup() {
        let conn = open_memory().unwrap();
        let p = create_project(&conn, "Acme", Some("https://www.Acme.com/"), None).unwrap();
        assert_eq!(p.domain.as_deref(), Some("acme.com"));
        assert!(create_project(&conn, "acme", None, None).is_err(), "names are unique case-insensitively");
        assert_eq!(find_project(&conn, "ACME").unwrap().unwrap().id, p.id);
        assert_eq!(find_project(&conn, &p.id.to_string()).unwrap().unwrap().name, "Acme");
        assert_eq!(recent_activity(&conn, Some(p.id), 10).unwrap().len(), 1);
        delete_project(&conn, p.id).unwrap();
        assert!(list_projects(&conn).unwrap().is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let conn = open_memory().unwrap();
        assert_eq!(get_settings(&conn).unwrap(), serde_json::json!({}));
        save_settings(&conn, &serde_json::json!({"lang": "tr"})).unwrap();
        assert_eq!(get_settings(&conn).unwrap()["lang"], "tr");
        assert!(save_settings(&conn, &serde_json::json!([1])).is_err());
    }
}
