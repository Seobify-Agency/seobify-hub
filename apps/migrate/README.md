# Seobify Migrate

<img src="brand/logo.jpg" alt="logo" width="160">

**Migration & Redirect Mapper** · Taşıma ve Yönlendirme

Maps old URLs to new ones, generates redirect rules and verifies them after launch. Part of **Seobify Hub**: local-first desktop tools for SEO agencies. Data never leaves the machine; every tool is also an MCP server for Claude.

## What it does

- **Plans** with old, new and optional staging bases; old/new URL sets from a Seobify Spider crawl, CSV, sitemap or list; priority values (clicks, referring domains) and optional content fetch (title/H1/simhash).
- **Matching engine**: regex patterns → same path → same (Turkish-folded) slug → same title/H1 → fuzzy score (Jaro-Winkler, Jaccard, TF-IDF cosine, simhash) → one-to-one preference → parent / homepage / 410 fallbacks. Statuses `auto`, `review`, `weak`, `manual`, `gone`, `keep` with confidence and reason.
- **Review** screen with filters, candidates and score breakdown, bulk approval, pattern suggestions and a validation panel (chains, loops, missing or non-200 targets, self and redundant rules) with one-click chain collapsing.
- **Exports**: Apache `.htaccess`, Nginx (`map` + `rewrite`), IIS `web.config`, Cloudflare Bulk Redirects CSV, Netlify `_redirects`, Vercel `vercel.json`, Next.js `redirects()`, WordPress Redirection CSV, plain CSV.
- **Verification** after launch (one 301/308 hop to the expected target answering 200) and before launch on staging, with run comparison, branded HTML report and CSV.

MCP tools: `list_plans`, `create_plan`, `import_urls`, `auto_match`, `list_mappings`, `set_mapping`, `add_pattern`, `validate_plan`, `export_rules`, `verify`, `generate_report` (plus the shared hub tools).

## Install

Download the newest `Seobify.Migrate_x.y.z_x64-setup.exe` from the [Seobify Hub releases](https://github.com/Seobify-Agency/seobify-hub/releases) and run it (per-user install, no admin rights).

## Use with Claude (MCP)

The exact `claude mcp add` command for your machine is shown under **Settings → Claude / MCP** in the app. The installed exe runs as an MCP server with the `--mcp` argument.

## Develop

```bash
npm install
npm run tauri dev
```

Checks: `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`. Installer: `npm run release`.


Licensed under AGPL-3.0, see [LICENSE](../../LICENSE). Seobify name and logos: see [TRADEMARKS.md](../../TRADEMARKS.md).
