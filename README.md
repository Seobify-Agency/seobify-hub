# Seobify Hub

[Türkçe](README.tr.md)

Free, open-source SEO desktop tools built by [Seobify](https://seobify.com/en/). Everything runs on your computer: no accounts, no telemetry, no Seobify server in between. Every tool also works inside Claude as an MCP server, so you can ask Claude to analyse crawler logs, plan redirects or clean files with your own data.

| | Tool | What it does |
| --- | --- | --- |
| <img src="brand/seobify-log.jpg" width="40"> | **Seobify Log** | Reads server logs or Cloudflare analytics and shows how Googlebot, Bingbot and AI crawlers really crawl a site: verified vs fake bots, crawl waste, status code problems. |
| <img src="brand/seobify-migrate.jpg" width="40"> | **Seobify Migrate** | Maps old URLs to new ones for a site migration, writes redirect rules for 9 servers and platforms, and verifies them before and after launch. |
| <img src="brand/seobify-optic.png" width="40"> | **Seobify Optic** | Converts images to WebP and removes hidden metadata (EXIF/GPS, XMP, C2PA, AI generator data) from images and from DOCX, XLSX, PPTX, ODF and PDF files. |

## Install

**Windows 10/11:** download the installer of the tool you need from the [latest release](https://github.com/Seobify-Agency/seobify-hub/releases/latest) and run it. It installs for your user only, no administrator rights needed. The installers are not code-signed yet: if Windows shows "Windows protected your PC", choose More info → Run anyway. Signed builds will follow.

**macOS:** not available yet.

**Claude Code:** after installing the tools, add the plugin. It registers every installed tool as an MCP server and adds a site migration workflow.

```text
/plugin marketplace add Seobify-Agency/seobify-hub
/plugin install seobify-hub@seobify
```

## Privacy

- Data stays in `~/Seobify/<Tool>` (Windows: `C:\Users\<you>\Seobify\<Tool>`). Uninstalling a tool does not delete it.
- Tokens (for example a Cloudflare API token) are stored in the Windows Credential Manager and are only ever shown masked.
- The tools contact only the sites you point them at and the services you connect. There is no analytics or tracking code.
- Reports end with one line pointing to Seobify. You can turn it off under Settings → About.

## Build from source

Each tool is a [Tauri 2](https://tauri.app) app (Rust + React + TypeScript) in `apps/<tool>`:

```text
cd apps/log
npm install
npm run tauri dev
```

Checks: `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`. Installer: `npm run release`. Some features connect to Seobify Spider, our crawler, which is not public; the tools work without it.

## Feedback and security

Bug reports and ideas are welcome in [Issues](https://github.com/Seobify-Agency/seobify-hub/issues). This is community support without a response-time promise. We do not accept pull requests at the moment, see [CONTRIBUTING.md](CONTRIBUTING.md). Please report security problems privately as described in [SECURITY.md](SECURITY.md).

## License

The code is licensed under the [GNU Affero General Public License v3.0](LICENSE). The Seobify name and logos are not covered by the license, see [TRADEMARKS.md](TRADEMARKS.md).

Planning a site migration or want an expert to act on what the tools find? [Talk to Seobify](https://seobify.com/en/contact/?utm_source=github&utm_medium=readme&utm_campaign=seobify-hub).
