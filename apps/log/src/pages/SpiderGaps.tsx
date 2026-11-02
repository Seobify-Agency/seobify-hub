import { useEffect, useState } from "react";
import { CsvButton, LogGate, StatusBadge, fmtTs } from "../components/LogKit";
import { Badge, Card, DataTable, Empty, ErrorAlert, Field, Loading, Page, Stat } from "../components/ui";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import type { SpiderCrawl, SpiderProject } from "../lib/api";
import { useProjects, useToast } from "../lib/context";
import { formatNumber, formatPercent } from "../lib/format";
import { logApi } from "../lib/logapi";
import type { GapsResult } from "../lib/logapi";
import { useRange } from "../lib/range";

export default function SpiderGaps() {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { project, reload } = useProjects();
  const { version } = useRange();
  const [projects, setProjects] = useState<SpiderProject[]>([]);
  const [source, setSource] = useState<string | null>(null);
  const [spider, setSpider] = useState("");
  const [crawls, setCrawls] = useState<SpiderCrawl[]>([]);
  const [crawlId, setCrawlId] = useState<number | null>(null);
  const [days, setDays] = useState(30);
  const [data, setData] = useState<GapsResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.spiderProjects()
      .then((r) => {
        setProjects(r.projects);
        setSource(r.source);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  useEffect(() => {
    setSpider(project?.spiderProject ?? (projects.some((p) => p.name === "default") ? "default" : projects[0]?.name ?? ""));
    setData(null);
  }, [project?.id, project?.spiderProject, projects]);
  useEffect(() => {
    if (!spider) return;
    setCrawls([]);
    setCrawlId(null);
    api.spiderCrawls(spider).then((r) => setCrawls(r.crawls)).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [spider]);

  const run = async () => {
    if (!project || !spider) return;
    setBusy(true);
    setError(null);
    try {
      setData(await logApi.spiderGaps(project.id, spider, crawlId, days));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    if (project?.spiderProject && spider === project.spiderProject && !data && !busy) void run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [spider, project?.spiderProject, version]);

  const link = async () => {
    if (!project) return;
    try {
      await logApi.updateProject(project.id, project.domain, spider);
      await reload();
      toast(t("spider.linked", { spider }), "success");
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const s = data?.summary;
  return (
    <Page title={t("nav.spider")} subtitle={t("spider.subtitle")}>
      <LogGate>
        <Card hint={source === "files" ? t("import.spiderOffline") : source === "api" ? t("import.spiderOnline") : undefined}>
          <div className="grid grid--3" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))", alignItems: "end" }}>
            <Field label={t("spider.project")}>
              <select className="select" value={spider} onChange={(e) => setSpider(e.target.value)}>
                {projects.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
              </select>
            </Field>
            <Field label={t("spider.crawl")}>
              <select className="select" value={crawlId ?? ""} onChange={(e) => setCrawlId(e.target.value ? Number(e.target.value) : null)}>
                <option value="">{t("spider.auto")}</option>
                {crawls.filter((c) => c.status === "completed").map((c) => (
                  <option key={c.id} value={c.id}>#{c.id} · {c.startUrl ?? c.name ?? "?"} · {formatNumber(c.urlsDone ?? 0, locale)} URL</option>
                ))}
              </select>
            </Field>
            <Field label={t("spider.days")}>
              <select className="select" value={days} onChange={(e) => setDays(Number(e.target.value))}>
                {[7, 14, 30, 60, 90, 365].map((d) => <option key={d} value={d}>{t("spider.daysN", { n: d })}</option>)}
              </select>
            </Field>
            <div className="row">
              <button type="button" className="btn btn--primary" disabled={busy || !spider} onClick={run}>{busy ? <span className="spinner" /> : "⌘"} {t("spider.run")}</button>
              {project && spider && project.spiderProject !== spider && <button type="button" className="btn btn--sm" onClick={link}>{t("spider.link")}</button>}
            </div>
          </div>
        </Card>
        <ErrorAlert error={error} />
        {busy && !data && <Loading label={t("spider.loading")} />}
        {!data && !busy && !error && <Empty icon="⌘" title={t("spider.emptyTitle")} text={t("spider.emptyText")} />}
        {data && s && (
          <>
            <p className="secondary" style={{ fontSize: 13 }}>
              {t("spider.source", { project: data.spider.spiderProject, crawl: data.spider.crawlId, url: data.spider.startUrl ?? "—", count: formatNumber(data.spider.comparedPages, locale), via: data.spider.source })} · {t("spider.window", { from: data.window.from, to: data.window.to })}
            </p>
            <div className="grid grid--stats">
              <Stat label={t("spider.stat.pages")} value={formatNumber(s.spiderPages, locale)} sub={t("spider.stat.indexableSub", { n: formatNumber(s.indexable, locale) })} />
              <Stat label={t("spider.stat.uncrawled")} value={formatNumber(s.indexableUncrawled, locale)} sub={formatPercent(s.uncrawledShare, locale)} />
              <Stat label={t("spider.stat.orphans")} value={formatNumber(s.orphanCandidates, locale)} sub={t("spider.stat.orphansSub", { n: formatNumber(s.orphanCandidates2xx, locale), hits: formatNumber(s.orphanHits, locale) })} />
              <Stat label={t("spider.stat.problems")} value={formatNumber(s.problemPagesCrawled, locale)} sub={t("spider.stat.problemsSub", { hits: formatNumber(s.problemHits, locale) })} />
            </div>
            <Card title={t("spider.orphans")} hint={t("spider.orphansHint")}>
              <DataTable
                rows={data.orphans}
                rowKey={(o) => o.url}
                initialSort={{ key: "googlebotHits", desc: true }}
                toolbar={<CsvButton filename="spider-orphans.csv" headers={["url", "googlebot_hits", "status", "last_googlebot"]} rows={data.orphans.map((o) => [o.url, o.googlebotHits, o.status, o.lastGooglebot])} />}
                columns={[
                  { key: "url", label: t("col.url"), className: "url" },
                  { key: "googlebotHits", label: t("col.googlebot"), numeric: true },
                  { key: "status", label: t("col.status"), render: (o) => <StatusBadge status={o.status} /> },
                  { key: "lastGooglebot", label: t("col.lastGooglebot"), render: (o) => fmtTs(o.lastGooglebot, locale) },
                ]}
              />
            </Card>
            <Card title={t("spider.uncrawled")} hint={t("spider.uncrawledHint", { days: data.window.days })}>
              <DataTable
                rows={data.uncrawled}
                rowKey={(u) => u.url}
                toolbar={<CsvButton filename="spider-uncrawled.csv" headers={["url", "title", "depth", "last_googlebot_any_time"]} rows={data.uncrawled.map((u) => [u.url, u.title, u.depth, u.lastGooglebot])} />}
                columns={[
                  { key: "path", label: t("col.url"), className: "url" },
                  { key: "title", label: t("spider.titleCol"), className: "url" },
                  { key: "depth", label: t("spider.depth"), numeric: true },
                  { key: "lastGooglebot", label: t("spider.lastEver"), render: (u) => (u.lastGooglebot ? fmtTs(u.lastGooglebot, locale) : <Badge tone="warning">{t("spider.never")}</Badge>) },
                ]}
              />
            </Card>
            <Card title={t("spider.problems")} hint={t("spider.problemsHint")}>
              <DataTable
                rows={data.problems}
                rowKey={(p) => p.url}
                initialSort={{ key: "googlebotHits", desc: true }}
                toolbar={<CsvButton filename="spider-problem-pages.csv" headers={["url", "status", "indexability", "reason", "googlebot_hits", "last_googlebot"]} rows={data.problems.map((p) => [p.url, p.status, p.indexability, p.reason, p.googlebotHits, p.lastGooglebot])} />}
                columns={[
                  { key: "path", label: t("col.url"), className: "url" },
                  { key: "status", label: t("col.status"), render: (p) => <StatusBadge status={p.status} /> },
                  { key: "reason", label: t("spider.reason"), render: (p) => <Badge tone="medium">{p.reason ?? p.indexability ?? "—"}</Badge> },
                  { key: "googlebotHits", label: t("col.googlebot"), numeric: true },
                  { key: "lastGooglebot", label: t("col.lastGooglebot"), render: (p) => fmtTs(p.lastGooglebot, locale) },
                ]}
              />
            </Card>
          </>
        )}
      </LogGate>
    </Page>
  );
}
