import { useState } from "react";
import { BarList } from "../components/charts";
import { CsvButton, LogGate, RangeBar, useLogData, useNav } from "../components/LogKit";
import { Card, DataTable, ErrorAlert, Loading, Page, Segment, Stat } from "../components/ui";
import { useI18n } from "../i18n";
import { formatNumber, formatPercent } from "../lib/format";
import { logApi } from "../lib/logapi";

export default function Budget() {
  const { t, locale } = useI18n();
  const nav = useNav();
  const [scope, setScope] = useState("googlebot");
  const { data, error, loading } = useLogData((pid, range) => logApi.crawlWaste(pid, range, scope), [scope]);
  return (
    <Page title={t("nav.budget")} subtitle={t("budget.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <Segment value={scope} onChange={setScope} options={["googlebot", "search", "all"].map((s) => ({ value: s, label: t(`budget.scope.${s}`) }))} />
        <ErrorAlert error={error} />
        {loading && !data && <Loading />}
        {data && (
          <>
            <div className="grid grid--stats">
              <Stat label={t("budget.waste")} value={formatPercent(data.wasteShare, locale)} sub={t("budget.wasteSub", { waste: formatNumber(data.wasteHits, locale), total: formatNumber(data.totalHits, locale) })} />
              {data.categories.map((c) => (
                <Stat key={c.key} label={t(`budget.cat.${c.key}`)} value={formatPercent(c.share, locale)} sub={`${formatNumber(c.hits, locale)} · ${t("budget.urlCount", { count: formatNumber(c.urls, locale) })}`} />
              ))}
            </div>
            {!data.spiderLinked && (
              <div className="alert alert--info">
                {t("budget.spiderOff")}{" "}
                <button type="button" className="btn btn--sm" onClick={() => nav("spider")}>{t("nav.spider")}</button>
              </div>
            )}
            <div className="grid grid--2">
              {data.categories.map((c) => (
                <Card key={c.key} title={`${t(`budget.cat.${c.key}`)} · ${formatPercent(c.share, locale)}`} hint={t(`budget.rec.${c.key}`)}
                  actions={<CsvButton filename={`waste-${c.key}.csv`} headers={["url", "hits"]} rows={c.examples.map((e) => [e.url, e.hits])} />}>
                  {c.examples.length ? <BarList items={c.examples.map((e) => ({ label: e.url, value: e.hits }))} /> : <p className="muted">{t("common.none")}</p>}
                </Card>
              ))}
            </div>
            <Card title={t("budget.params")} hint={t("budget.paramsHint")}>
              <DataTable
                rows={data.parameters}
                rowKey={(p) => p.param}
                initialSort={{ key: "hits", desc: true }}
                toolbar={<CsvButton filename="parameters.csv" headers={["param", "hits", "share", "urls", "examples"]} rows={data.parameters.map((p) => [p.param, p.hits, p.share.toFixed(4), p.urls, p.examples.join(" ")])} />}
                columns={[
                  { key: "param", label: t("budget.param"), render: (p) => <code>{p.param}</code> },
                  { key: "hits", label: t("col.requests"), numeric: true },
                  { key: "share", label: t("col.share"), numeric: true, render: (p) => formatPercent(p.share, locale) },
                  { key: "urls", label: t("budget.urls"), numeric: true },
                  { key: "examples", label: t("col.examples"), className: "url", value: (p) => p.examples.join("  ") },
                ]}
              />
            </Card>
          </>
        )}
      </LogGate>
    </Page>
  );
}
