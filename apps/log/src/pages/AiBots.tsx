import { useState } from "react";
import { LineChart } from "../components/charts";
import { CatBadge, CsvButton, LogGate, RangeBar, VerBadges, fmtTs, useLogData } from "../components/LogKit";
import { Badge, Card, DataTable, ErrorAlert, Loading, Page, Stat } from "../components/ui";
import { useI18n } from "../i18n";
import { useProjects } from "../lib/context";
import { formatNumber } from "../lib/format";
import { CATEGORY_COLORS, logApi } from "../lib/logapi";
import type { RobotsResult } from "../lib/logapi";

export default function AiBots() {
  const { t, locale } = useI18n();
  const { project } = useProjects();
  const { data, error, loading } = useLogData((pid, range) => logApi.aiActivity(pid, range));
  const [robots, setRobots] = useState<RobotsResult | null>(null);
  const [robotsBusy, setRobotsBusy] = useState(false);
  const [robotsError, setRobotsError] = useState<string | null>(null);

  const checkRobots = async () => {
    if (!project) return;
    setRobotsBusy(true);
    setRobotsError(null);
    try {
      setRobots(await logApi.robots(project.id));
    } catch (e) {
      setRobotsError(e instanceof Error ? e.message : String(e));
    } finally {
      setRobotsBusy(false);
    }
  };

  return (
    <Page title={t("nav.ai")} subtitle={t("ai.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <ErrorAlert error={error} />
        {loading && !data && <Loading />}
        {data && (
          <>
            <div className="grid grid--stats">
              <Stat label={t("cat.ai_training")} value={formatNumber(data.trainingTotal, locale)} />
              <Stat label={t("cat.ai_assistant")} value={formatNumber(data.assistantTotal, locale)} />
              <Stat label={t("ai.botCount")} value={formatNumber(data.bots.length, locale)} />
            </div>
            <Card title={t("ai.overTime")}>
              <LineChart
                height={240}
                series={[
                  { name: t("cat.ai_training"), color: CATEGORY_COLORS.ai_training, points: data.days.map((d, i) => ({ x: d, y: data.training[i] })) },
                  { name: t("cat.ai_assistant"), color: CATEGORY_COLORS.ai_assistant, points: data.days.map((d, i) => ({ x: d, y: data.assistant[i] })) },
                ]}
              />
            </Card>
            <Card title={t("ai.bots")}>
              <DataTable
                rows={data.bots}
                rowKey={(b) => b.botId}
                initialSort={{ key: "requests", desc: true }}
                toolbar={<CsvButton filename="ai-bots.csv" headers={["bot", "owner", "category", "requests", "unique_paths", "top_dirs", "verified", "unverifiable", "fake", "last_seen"]} rows={data.bots.map((b) => [b.name, b.owner, b.category, b.requests, b.uniquePaths, b.topDirs.map((d) => `${d.dir} (${d.hits})`).join(" "), b.verified, b.unverifiable, b.fake, b.lastSeen])} />}
                columns={[
                  { key: "name", label: t("col.bot"), render: (b) => <b>{b.name}</b> },
                  { key: "owner", label: t("col.owner") },
                  { key: "category", label: t("col.category"), render: (b) => <CatBadge category={b.category} /> },
                  { key: "requests", label: t("col.requests"), numeric: true },
                  { key: "uniquePaths", label: t("col.uniqueUrls"), numeric: true },
                  { key: "topDirs", label: t("ai.topDirs"), sortable: false, render: (b) => <span className="mono">{b.topDirs.slice(0, 4).map((d) => `${d.dir} ${formatNumber(d.hits, locale)}`).join(" · ")}</span> },
                  { key: "verification", label: t("col.verification"), sortable: false, render: (b) => <VerBadges verified={b.verified} fake={b.fake} unverifiable={b.unverifiable} /> },
                  { key: "lastSeen", label: t("col.lastSeen"), render: (b) => fmtTs(b.lastSeen, locale) },
                ]}
              />
            </Card>
            <div className="grid grid--2">
              <Card title={t("ai.topUrls")} actions={<CsvButton filename="ai-top-urls.csv" headers={["path", "hits", "bots"]} rows={data.topUrls.map((u) => [u.path, u.hits, u.bots])} />}>
                <DataTable
                  rows={data.topUrls}
                  rowKey={(u) => u.path}
                  searchable={false}
                  initialSort={{ key: "hits", desc: true }}
                  columns={[
                    { key: "path", label: t("col.url"), className: "url" },
                    { key: "hits", label: t("col.requests"), numeric: true },
                    { key: "bots", label: t("ai.bots"), numeric: true },
                  ]}
                />
              </Card>
              <Card
                title={t("ai.robots")}
                hint={t("ai.robotsHint")}
                actions={<button type="button" className="btn btn--sm" disabled={robotsBusy || !project?.domain} onClick={checkRobots}>{robotsBusy ? <span className="spinner" /> : "↻"} {t("ai.robotsCheck")}</button>}
              >
                {!project?.domain && <p className="muted">{t("ai.noDomain")}</p>}
                <ErrorAlert error={robotsError} />
                {robots && (
                  <>
                    <p className="muted" style={{ fontSize: 12 }}>{robots.finalUrl} · HTTP {robots.status} · {fmtTs(robots.fetchedAt, locale)}</p>
                    <DataTable
                      rows={robots.bots}
                      rowKey={(b) => b.token}
                      searchable={false}
                      columns={[
                        { key: "token", label: t("ai.token"), render: (b) => <code>{b.token}</code> },
                        { key: "category", label: t("col.category"), render: (b) => <CatBadge category={b.category} /> },
                        { key: "verdict", label: t("ai.verdict"), render: (b) => <Badge tone={b.verdict === "allowed" ? "success" : b.verdict === "blocked" ? "error" : "warning"}>{t(`ai.verdict.${b.verdict}`)}</Badge> },
                        { key: "matched", label: t("ai.matched"), render: (b) => <span className="muted">{t(`ai.matched.${b.matched}`)}</span> },
                        { key: "rules", label: t("ai.rules"), sortable: false, value: (b) => b.rules.join(" · "), className: "url" },
                      ]}
                    />
                  </>
                )}
              </Card>
            </div>
          </>
        )}
      </LogGate>
    </Page>
  );
}
