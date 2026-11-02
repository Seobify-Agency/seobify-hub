import { LineChart } from "../components/charts";
import { CsvButton, LogGate, RangeBar, StatusBadge, fmtTs, useLogData } from "../components/LogKit";
import { Badge, Card, DataTable, ErrorAlert, Loading, Page, Stat } from "../components/ui";
import { useI18n } from "../i18n";
import { formatNumber } from "../lib/format";
import { logApi } from "../lib/logapi";
import type { Spike } from "../lib/logapi";

export default function Status() {
  const { t, locale } = useI18n();
  const { data, error, loading } = useLogData((pid, range) => logApi.status(pid, range));
  const bySeries = (map: Record<string, number[]>, days: string[]) =>
    Object.entries(map).map(([bot, values]) => ({ name: bot, points: days.map((d, i) => ({ x: d, y: values[i] ?? 0 })) }));
  const spikes: (Spike & { kind: string })[] = data ? [...data.spikes5xx.map((s) => ({ ...s, kind: "5xx" })), ...data.spikes4xx.map((s) => ({ ...s, kind: "4xx" }))] : [];
  return (
    <Page title={t("nav.status")} subtitle={t("status.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <ErrorAlert error={error} />
        {loading && !data && <Loading />}
        {data && (
          <>
            <div className="grid grid--stats">
              <Stat label={t("status.total4xx")} value={formatNumber(data.total4xx, locale)} />
              <Stat label={t("status.total5xx")} value={formatNumber(data.total5xx, locale)} />
              <Stat label={t("status.spikeDays")} value={formatNumber(spikes.length, locale)} sub={t("status.spikeRule")} />
            </div>
            <Card title={t("status.spikes")}>
              {spikes.length === 0 ? (
                <p className="muted">{t("status.noSpikes")}</p>
              ) : (
                spikes.map((s) => (
                  <div key={`${s.kind}-${s.day}`} className="row" style={{ fontSize: 13 }}>
                    <Badge tone={s.kind === "5xx" ? "error" : "warning"}>{s.kind}</Badge>
                    <b>{fmtTs(s.day, locale)}</b>
                    <span>{t("status.spikeRow", { count: formatNumber(s.count, locale), mean: formatNumber(s.mean, locale, 1), threshold: formatNumber(s.threshold, locale, 1) })}</span>
                  </div>
                ))
              )}
            </Card>
            <div className="grid grid--2">
              <Card title={t("status.timeline5xx")}>{Object.keys(data.byBot5xx).length ? <LineChart series={bySeries(data.byBot5xx, data.days)} height={220} /> : <p className="muted">{t("common.none")}</p>}</Card>
              <Card title={t("status.timeline4xx")}>{Object.keys(data.byBot4xx).length ? <LineChart series={bySeries(data.byBot4xx, data.days)} height={220} /> : <p className="muted">{t("common.none")}</p>}</Card>
            </div>
            <Card title={t("status.errorUrls")} hint={t("status.errorHint")}>
              <DataTable
                rows={data.errorUrls}
                rowKey={(u) => `${u.url}-${u.status}`}
                initialSort={{ key: "hits", desc: true }}
                toolbar={<CsvButton filename="error-urls.csv" headers={["url", "status", "hits", "bots", "last_seen"]} rows={data.errorUrls.map((u) => [u.url, u.status, u.hits, u.bots, u.lastSeen])} />}
                columns={[
                  { key: "url", label: t("col.url"), className: "url" },
                  { key: "status", label: t("col.status"), render: (u) => <StatusBadge status={u.status} /> },
                  { key: "hits", label: t("col.requests"), numeric: true },
                  { key: "bots", label: t("col.bot"), className: "url", render: (u) => u.bots.split(",").join(", ") },
                  { key: "lastSeen", label: t("col.lastSeen"), render: (u) => fmtTs(u.lastSeen, locale) },
                ]}
              />
            </Card>
            <Card title={t("status.codes")}>
              <div className="row">
                {data.codes.map((c) => (
                  <span key={c.status} className="row" style={{ gap: 6 }}>
                    <StatusBadge status={c.status} /> {formatNumber(c.hits, locale)}
                  </span>
                ))}
              </div>
            </Card>
          </>
        )}
      </LogGate>
    </Page>
  );
}
