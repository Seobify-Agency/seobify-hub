// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// `--mcp` still works in release: MCP clients spawn the exe with piped stdio handles.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  if std::env::args().skip(1).any(|a| a == "--mcp") {
    app_lib::mcp_server::run();
    return;
  }
  app_lib::run();
}
