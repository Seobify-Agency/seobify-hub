# Seobify Log

<img src="brand/logo.jpg" alt="logo" width="160">

**Server Log Analyzer** · Sunucu Log Analizi

Local server log analyzer for search engine and AI crawler activity. Part of **Seobify Hub**: local-first desktop tools for SEO agencies. Data never leaves the machine; every tool is also an MCP server for Claude.

## What it does

- Imports access logs (Apache/Nginx combined and common, Nginx/Caddy JSON, Cloudflare Logpush, IIS W3C, AWS CloudFront, AWS ALB; `.gz` streamed; format detected per file; duplicate files refused).
- Classifies search, AI training, AI search/assistant, SEO, social and other bots, and verifies them with the published IP range lists (Google, Bing, OpenAI, Perplexity, Apple) and reverse DNS with forward confirmation.
- GDPR: human visitors are kept only as IP-free daily counters; unverified bot addresses are masked or stored as salted hashes.
- Screens: Overview, Bots, URLs, Status & Errors (3σ spikes), Crawl Budget, Spider Comparison (orphans, uncrawled indexable pages, crawled problem pages), AI Bots (with a robots.txt summary), Fake Bots, Imports. CSV on every table, branded HTML report with findings.
- Cloudflare connection for sites served entirely by Cloudflare (no server log): paste a read-only API token (Zone · Analytics · Read and Zone · Zone · Read) under **Imports**, link the project to its zone, and Log imports the last 30 days from Cloudflare's GraphQL analytics, then new days whenever the app opens. Works on the Free plan; counts may be sampled on busy sites. Crawler IPs come from Cloudflare and are verified like server logs (plus Cloudflare's own verified-bot category); human traffic is fetched without IPs.
- Origin logs behind Cloudflare: crawler hits from Cloudflare edge addresses are counted as unverifiable instead of fake, with a note to log `CF-Connecting-IP`.
- MCP tools: `cloudflare_status`, `cloudflare_sync`, `import_logs`, `log_overview`, `bot_stats`, `top_urls`, `crawl_waste`, `spider_gaps`, `ai_bot_activity`, `fake_bots`, `status_anomalies`, `generate_report`.

## Install

Download the newest `Seobify.Log_x.y.z_x64-setup.exe` from the [Seobify Hub releases](https://github.com/Seobify-Agency/seobify-hub/releases) and run it (per-user install, no admin rights).

## Use with Claude (MCP)

The exact `claude mcp add` command for your machine is shown under **Settings → Claude / MCP** in the app. The installed exe runs as an MCP server with the `--mcp` argument.

## Develop

```bash
npm install
npm run tauri dev
```

Checks: `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`. Installer: `npm run release`.


Licensed under AGPL-3.0, see [LICENSE](../../LICENSE). Seobify name and logos: see [TRADEMARKS.md](../../TRADEMARKS.md).
