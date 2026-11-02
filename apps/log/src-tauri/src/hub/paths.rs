//! Data locations. Everything a hub tool stores lives under `%USERPROFILE%\Seobify\<Product>\`
//! (`~/Seobify/<Product>` elsewhere; override with `SEOBIFY_DATA_DIR`, used by tests and
//! portable installs).
//!
//! Not under AppData: Windows redirects new AppData files written by packaged apps (the
//! Claude desktop app and the MCP servers it starts) into a private per-app store, so the
//! desktop app and Claude would each see their own data. Data from versions up to 0.1.x in
//! `%APPDATA%\Seobify\<Product>\` is moved here on first use.

use std::path::{Path, PathBuf};
use std::sync::Once;

pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("SEOBIFY_DATA_DIR") {
        Some(custom) => PathBuf::from(custom),
        None => {
            let dir = default_dir();
            static MOVE: Once = Once::new();
            MOVE.call_once(|| {
                if let Err(e) = move_legacy(&legacy_dir(), &dir, &format!("{}.db", crate::TOOL_KEY)) {
                    eprintln!("{}: could not move data from {}: {e}", crate::PRODUCT_NAME, legacy_dir().display());
                }
            });
            dir
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn default_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("Seobify").join(crate::PRODUCT_DIR)
}

/// Where versions up to 0.1.x kept their data (`%APPDATA%\Seobify\<Product>\`).
pub fn legacy_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Seobify").join(crate::PRODUCT_DIR)
}

/// Copies an old data folder into `to` once: the database as a consistent snapshot
/// (`VACUUM INTO`, safe while other processes have it open), every other file and folder
/// as is. Does nothing when `to` already has a database or `from` has none. The old folder
/// is kept, with a `MOVED.txt` pointing to the new place.
pub fn move_legacy(from: &Path, to: &Path, db_file: &str) -> Result<bool, String> {
    let (src_db, dst_db) = (from.join(db_file), to.join(db_file));
    if dst_db.exists() || !src_db.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let tmp = to.join(format!("{db_file}.moving"));
    let _ = std::fs::remove_file(&tmp);
    let src = rusqlite::Connection::open_with_flags(&src_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("{}: {e}", src_db.display()))?;
    src.busy_timeout(std::time::Duration::from_secs(8)).map_err(|e| e.to_string())?;
    if let Err(e) = src.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("snapshot of {}: {e}", src_db.display()));
    }
    drop(src);
    let skip = [db_file.to_string(), format!("{db_file}-wal"), format!("{db_file}-shm"), "MOVED.txt".into()];
    copy_tree(from, to, &skip)?;
    std::fs::rename(&tmp, &dst_db).map_err(|e| format!("{}: {e}", dst_db.display()))?;
    let note = format!("This folder is no longer used. Data moved on {} to:\n{}\n", super::now_iso(), to.display());
    let _ = std::fs::write(from.join("MOVED.txt"), note);
    Ok(true)
}

fn copy_tree(from: &Path, to: &Path, skip: &[String]) -> Result<(), String> {
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip.contains(&name) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            std::fs::create_dir_all(&dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            copy_tree(&src, &dst, &[])?;
        } else if !dst.exists() {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

pub fn db_path() -> PathBuf {
    data_dir().join(format!("{}.db", crate::TOOL_KEY))
}

pub fn reports_dir() -> PathBuf {
    let dir = data_dir().join("reports");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Seobify Spider keeps one DuckDB file per project here.
pub fn spider_home() -> PathBuf {
    std::env::var_os("SEOBIFY_SPIDER_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".seobify-spider"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_legacy_data_once_with_open_wal_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        std::fs::create_dir_all(old.join("reports").join("sub")).unwrap();
        std::fs::write(old.join("reports").join("sub").join("r.html"), "report").unwrap();
        let writer = rusqlite::Connection::open(old.join("t.db")).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE p (name TEXT); INSERT INTO p VALUES ('Seobify');").unwrap();
        // The writer stays open: the snapshot must include rows still in the WAL.
        assert!(move_legacy(&old, &new, "t.db").unwrap());
        let copied = rusqlite::Connection::open(new.join("t.db")).unwrap();
        let name: String = copied.query_row("SELECT name FROM p", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Seobify");
        assert_eq!(std::fs::read_to_string(new.join("reports").join("sub").join("r.html")).unwrap(), "report");
        assert!(!new.join("t.db-wal").exists() && !new.join("t.db.moving").exists());
        assert!(std::fs::read_to_string(old.join("MOVED.txt")).unwrap().contains("new"));
        writer.execute("INSERT INTO p VALUES ('later')", []).unwrap();
        assert!(!move_legacy(&old, &new, "t.db").unwrap(), "never overwrites the new database");
        let n: i64 = copied.query_row("SELECT COUNT(*) FROM p", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn nothing_to_move() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!move_legacy(&tmp.path().join("missing"), &tmp.path().join("new"), "t.db").unwrap());
        assert!(!tmp.path().join("new").exists());
    }
}
