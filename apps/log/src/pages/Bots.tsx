import { useState } from "react";
import { LineChart } from "../components/charts";
import { CatBadge, CsvButton, LogGate, RangeBar, StatusMix, VerBadges, fmtTs, useLogData } from "../components/LogKit";
import { Card, DataTable, ErrorAlert, Loading, Modal, Page, Segment } from "../components/ui";
import { useI18n } from "../i18n";
import { formatNumber } from "../lib/format";
import { CATEGORIES, logApi } from "../lib/logapi";
import type { BotRow } from "../lib/logapi";

function BotDetailModal({ bot, onClose }: { bot: BotRow; onClose: () => void }) {
  const { t, locale } = useI18n();
  const { data, error, loading } = useLogData((pid, range) => logApi.botDetail(pid, bot.botId, range), [bot.botId]);
  return (
    <Modal title={t("bots.detail", { name: bot.name })} onClose={onClose} wide>
      <ErrorAlert error={error} />
      {loading && !data && <Loading />}
      {data && (
        <>
          <LineChart
            height={200}
            series={[
              { name: t("col.requests"), color: "var(--series-1)", points: data.days.map((d, i) => ({ x: d, y: data.requests[i] })) },
              { name: t("bots.errors"), color: "var(--warning)", points: data.days.map((d, i) => ({ x: d, y: data.errors[i] })) },
              ...(data.fake.some((v) => v > 0) ? [{ name: t("ver.fake"), color: "var(--error)", points: data.days.map((d, i) => ({ x: d, y: data.fake[i] })) }] : []),
            ]}
          />
          <DataTable
            rows={data.urls}
            rowKey={(u) => u.url}
            initialSort={{ key: "hits", desc: true }}
            toolbar={<CsvButton filename={`${bot.botId}-urls.csv`} headers={["url", "hits", "2xx", "3xx", "errors", "fake", "last_seen"]} rows={data.urls.map((u) => [u.url, u.hits, u.s2xx, u.s3xx, u.errors, u.fake, u.lastSeen])} />}
            columns={[
              { key: "url", label: t("col.url"), className: "url" },
              { key: "hits", label: t("col.requests"), numeric: true },
              { key: "s2xx", label: "2xx", numeric: true },
              { key: "s3xx", label: "3xx", numeric: true },
              { key: "errors", label: "4xx+5xx", numeric: true },
              { key: "lastSeen", label: t("col.lastSeen"), render: (u) => fmtTs(u.lastSeen, locale) },
            ]}
          />
          <p className="muted" style={{ fontSize: 12 }}>{t("common.rows", { count: formatNumber(data.urls.length, locale) })}</p>
        </>
      )}
    </Modal>
  );
}

export default function Bots() {
  const { t, locale } = useI18n();
  const [category, setCategory] = useState("all");
  const [selected, setSelected] = useState<BotRow | null>(null);
  const { data, error, loading } = useLogData((pid, range) => logApi.botStats(pid, range, category === "all" ? null : category), [category]);
  const rows = data?.bots ?? [];
  return (
    <Page title={t("nav.bots")} subtitle={t("bots.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <Segment value={category} onChange={setCategory} options={[{ value: "all", label: t("common.all") }, ...CATEGORIES.map((c) => ({ value: c, label: t(`cat.${c}`) }))]} />
        <ErrorAlert error={error} />
        <Card hint={t("bots.clickHint")}>
          {loading && !data ? (
            <Loading />
          ) : (
            <DataTable
              rows={rows}
              rowKey={(b) => b.botId}
              onRowClick={setSelected}
              initialSort={{ key: "requests", desc: true }}
              toolbar={
                <CsvButton
                  filename="bots.csv"
                  headers={["bot", "category", "owner", "requests", "unique_urls", "2xx", "3xx", "4xx", "5xx", "avg_response_ms", "first_seen", "last_seen", "verified", "unverifiable", "fake"]}
                  rows={rows.map((b) => [b.name, b.category, b.owner, b.requests, b.uniqueUrls, b.s2xx, b.s3xx, b.s4xx, b.s5xx, b.avgResponseMs === null ? "" : Math.round(b.avgResponseMs), b.firstSeen, b.lastSeen, b.verified, b.unverifiable, b.fake])}
                />
              }
              columns={[
                { key: "name", label: t("col.bot"), render: (b) => <b>{b.name}</b> },
                { key: "category", label: t("col.category"), render: (b) => <CatBadge category={b.category} /> },
                { key: "requests", label: t("col.requests"), numeric: true },
                { key: "uniqueUrls", label: t("col.uniqueUrls"), numeric: true },
                { key: "status", label: "2xx · 3xx · 4xx · 5xx", sortable: false, render: (b) => <StatusMix {...b} /> },
                { key: "avgResponseMs", label: t("col.avgMs"), numeric: true, render: (b) => (b.avgResponseMs === null ? "—" : formatNumber(b.avgResponseMs, locale)) },
                { key: "lastSeen", label: t("col.lastSeen"), render: (b) => fmtTs(b.lastSeen, locale) },
                { key: "verification", label: t("col.verification"), sortable: false, render: (b) => <VerBadges verified={b.verified} fake={b.fake} unverifiable={b.unverifiable} /> },
              ]}
            />
          )}
        </Card>
        {selected && <BotDetailModal bot={selected} onClose={() => setSelected(null)} />}
      </LogGate>
    </Page>
  );
}
