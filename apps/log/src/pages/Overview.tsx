import { useState } from "react";
import { BarList, LineChart, ShareBar } from "../components/charts";
import { CsvButton, LogGate, RangeBar, ReportButton, useLogData } from "../components/LogKit";
import { Card, ErrorAlert, Loading, Page, Stat } from "../components/ui";
import { useI18n } from "../i18n";
import { useProjects } from "../lib/context";
import { formatNumber, formatPercent } from "../lib/format";
import { CATEGORIES, CATEGORY_COLORS, STATUS_COLORS, logApi } from "../lib/logapi";

export default function Overview() {
  const { t, locale } = useI18n();
  const { project } = useProjects();
  const [showHuman, setShowHuman] = useState(false);
  const { data, error, loading } = useLogData((pid, range) => logApi.overview(pid, range));

  const series = data
    ? [...CATEGORIES, "fake", ...(showHuman ? ["human"] : [])]
        .filter((k) => (data.daily.series[k] ?? []).some((v) => v > 0))
        .map((k) => ({ name: t(`cat.${k}`), color: CATEGORY_COLORS[k], points: data.daily.days.map((d, i) => ({ x: d, y: data.daily.series[k]?.[i] ?? 0 })) }))
    : [];

  return (
    <Page title={t("nav.dashboard")} subtitle={project ? `${project.name}${project.domain ? ` · ${project.domain}` : ""} — ${t("ov.subtitle")}` : t("ov.subtitle")} actions={<><RangeBar /><ReportButton /></>}>
      <LogGate>
        <ErrorAlert error={error} />
        {loading && !data && <Loading />}
        {data && (
          <>
            <div className="grid grid--stats">
              <Stat label={t("ov.totalBot")} value={formatNumber(data.totalBotRequests, locale)} sub={`${data.range.from} → ${data.range.to}`} />
              <Stat label={t("ov.googlebot")} value={formatNumber(data.googlebot.requests, locale)} sub={t("ov.smartphoneSub", { smart: formatPercent(data.googlebot.smartphoneShare, locale), desk: formatNumber(data.googlebot.desktop, locale) })} />
              <Stat label={t("ov.uniqueUrls")} value={formatNumber(data.uniqueUrls, locale)} />
              <Stat label={t("ov.verifiedShare")} value={formatPercent(data.verification.verifiedShare, locale)} sub={t("ov.fakeSub", { fake: formatNumber(data.verification.fake, locale), share: formatPercent(data.verification.fakeShare, locale) })} />
              <Stat label={t("ov.human")} value={formatNumber(data.humanRequests, locale)} sub={t("ov.humanHint")} />
            </div>
            <Card
              title={t("ov.daily")}
              actions={
                <label className="checkbox">
                  <input type="checkbox" checked={showHuman} onChange={(e) => setShowHuman(e.target.checked)} /> {t("ov.showHuman")}
                </label>
              }
            >
              <LineChart series={series} height={260} />
            </Card>
            <div className="grid grid--2">
              <Card title={t("ov.status")}>
                <ShareBar
                  parts={[
                    { label: "2xx", value: data.status.s2xx, color: STATUS_COLORS.s2xx },
                    { label: "3xx", value: data.status.s3xx, color: STATUS_COLORS.s3xx },
                    { label: "4xx", value: data.status.s4xx, color: STATUS_COLORS.s4xx },
                    { label: "5xx", value: data.status.s5xx, color: STATUS_COLORS.s5xx },
                  ]}
                />
                <div className="field__label" style={{ marginTop: 8 }}>{t("ov.verification")}</div>
                <ShareBar
                  parts={[
                    { label: t("ver.verified"), value: data.verification.verified, color: "var(--success)" },
                    { label: t("ver.unverifiable"), value: data.verification.unverifiable, color: "var(--text-muted)" },
                    { label: t("ver.fake"), value: data.verification.fake, color: "var(--error)" },
                  ]}
                />
                <div className="field__label" style={{ marginTop: 8 }}>{t("ov.googlebotSplit")}</div>
                <ShareBar
                  parts={[
                    { label: t("ov.smartphone"), value: data.googlebot.smartphone, color: "var(--series-1)" },
                    { label: t("ov.desktop"), value: data.googlebot.desktop, color: "var(--series-2)" },
                    { label: t("ov.otherGoogle"), value: Math.max(0, data.googlebot.requests - data.googlebot.smartphone - data.googlebot.desktop), color: "var(--series-3)" },
                  ]}
                />
              </Card>
              <Card title={t("ov.categories")}>
                <BarList items={data.byCategory.map((c) => ({ label: t(`cat.${c.category}`), value: c.requests }))} />
                {data.humanStatus.length > 0 && (
                  <>
                    <div className="field__label" style={{ marginTop: 8 }}>{t("ov.humanStatus")}</div>
                    <ShareBar parts={data.humanStatus.map((h) => ({ label: h.class, value: h.requests, color: STATUS_COLORS[`s${h.class}` as keyof typeof STATUS_COLORS] ?? "var(--text-muted)" }))} />
                    <ShareBar parts={data.humanDevices.map((d, i) => ({ label: t(`device.${d.device}`), value: d.requests, color: `var(--series-${(i % 6) + 1})` }))} />
                  </>
                )}
              </Card>
            </div>
            <Card title={t("ov.topUrls")} actions={<CsvButton filename="top-urls.csv" headers={["url", "bot_hits", "googlebot_hits", "errors"]} rows={data.topUrls.map((u) => [u.url, u.hits, u.googlebotHits, u.errors])} />}>
              <BarList items={data.topUrls.map((u) => ({ label: u.url, value: u.hits, hint: `${u.url} · Googlebot ${u.googlebotHits}` }))} />
            </Card>
          </>
        )}
      </LogGate>
    </Page>
  );
}
