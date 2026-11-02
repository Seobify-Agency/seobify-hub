//! Synthetic combined-log generator for benchmarks and demo data.
//!
//! cargo run --example synth_log -- --out <file> [--lines 1000000] [--days 28]
//!     [--start 2026-10-01] [--seed 42] [--gz] [--timing] [--no-dns-bots]
//!     [--spider default --crawl 158]   # use the pages of a Seobify Spider crawl
//!
//! With `--spider`, site pages come from the crawl, noindex pages become "hot" (Googlebot
//! keeps fetching them) and ~35 % of pages stay cold (never crawled by Googlebot).

use app_lib::logs::synth::{self, SynthConfig};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let has = |k: &str| args.iter().any(|a| a == k);
    let Some(out) = get("--out") else {
        eprintln!("usage: synth_log --out <file> [--lines N] [--days D] [--start YYYY-MM-DD] [--seed N] [--gz] [--timing] [--no-dns-bots] [--spider PROJECT --crawl ID]");
        std::process::exit(2);
    };
    let mut cfg = SynthConfig {
        lines: get("--lines").and_then(|v| v.parse().ok()).unwrap_or(1_000_000),
        days: get("--days").and_then(|v| v.parse().ok()).unwrap_or(28),
        start_day: get("--start").unwrap_or_else(|| "2026-10-01".into()),
        seed: get("--seed").and_then(|v| v.parse().ok()).unwrap_or(42),
        dns_bots: !has("--no-dns-bots"),
        timing: has("--timing"),
        ..Default::default()
    };
    if let Some(project) = get("--spider") {
        let crawl = get("--crawl").and_then(|v| v.parse::<i64>().ok());
        let rt = tokio::runtime::Runtime::new().expect("tokio");
        let (urls, id, source) = rt.block_on(app_lib::hub::spider::urls(&project, crawl)).unwrap_or_else(|e| {
            eprintln!("Spider: {e}");
            std::process::exit(1);
        });
        let mut site = Vec::new();
        let mut hot = Vec::new();
        for u in &urls {
            let html = u.content_type.as_deref().map_or(true, |c| c.contains("html"));
            let Ok(parsed) = url::Url::parse(&u.url) else { continue };
            if !html {
                continue;
            }
            let path = match parsed.query() {
                Some(q) => format!("{}?{q}", parsed.path()),
                None => parsed.path().to_string(),
            };
            if u.indexability.as_deref() == Some("non-indexable") {
                hot.push(path.clone());
            }
            site.push(path);
        }
        eprintln!("Spider {project}#{id} via {source}: {} pages, {} noindex (hot)", site.len(), hot.len());
        cfg.site_paths = site;
        cfg.hot_paths = hot;
    }
    let file = std::fs::File::create(&out).unwrap_or_else(|e| {
        eprintln!("{out}: {e}");
        std::process::exit(1);
    });
    let started = std::time::Instant::now();
    let stats = if has("--gz") {
        let mut w = flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::fast());
        let s = synth::generate(&cfg, &mut w).expect("write");
        w.finish().expect("gzip").flush().expect("flush");
        s
    } else {
        let mut w = std::io::BufWriter::new(file);
        let s = synth::generate(&cfg, &mut w).expect("write");
        w.flush().expect("flush");
        s
    };
    println!(
        "{}",
        serde_json::json!({ "out": out, "lines": stats.lines, "coldPaths": stats.cold_paths, "seconds": started.elapsed().as_secs_f64(), "byActor": stats.by_actor })
    );
}
