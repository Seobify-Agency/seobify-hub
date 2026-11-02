import { CsvButton, LogGate, RangeBar, fmtTs, useLogData } from "../components/LogKit";
import { Card, DataTable, Empty, ErrorAlert, Loading, Page, Stat } from "../components/ui";
import { BarList } from "../components/charts";
import { useI18n } from "../i18n";
import { formatNumber, formatPercent } from "../lib/format";
import { logApi } from "../lib/logapi";

export default function FakeBots() {
  const { t, locale } = useI18n();
  const { data, error, loading } = useLogData((pid, range) => logApi.fakeBots(pid, range));
  return (
    <Page title={t("nav.fake")} subtitle={t("fake.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <ErrorAlert error={error} />
        {loading && !data && <Loading />}
        {data && (
          <>
            <div className="grid grid--stats">
              <Stat label={t("fake.requests")} value={formatNumber(data.fakeRequests, locale)} />
              <Stat label={t("fake.sources")} value={formatNumber(data.sources, locale)} sub={t("fake.privacy")} />
              <Stat label={t("fake.unverifiable")} value={formatNumber(data.unverifiableRequests, locale)} sub={t("fake.unverifiableHint")} />
            </div>
            {data.fakeRequests === 0 ? (
              <Empty icon="✓" title={t("fake.none")} text={t("fake.noneText")} />
            ) : (
              <>
                <Card title={t("fake.byBot")}>
                  <BarList items={data.byBot.map((b) => ({ label: `${b.name} · ${t("fake.shareOfClaims", { share: formatPercent(b.fakeShare, locale) })}`, value: b.fake }))} />
                </Card>
                <Card title={t("fake.groups")}>
                  <DataTable
                    rows={data.groups}
                    rowKey={(g) => `${g.ip}-${g.botId}`}
                    initialSort={{ key: "requests", desc: true }}
                    toolbar={<CsvButton filename="fake-bots.csv" headers={["ip_masked_or_hashed", "claims", "requests", "unique_paths", "sample_urls", "first_seen", "last_seen"]} rows={data.groups.map((g) => [g.ip, g.claims, g.requests, g.uniquePaths, g.sampleUrls.join(" "), g.firstSeen, g.lastSeen])} />}
                    columns={[
                      { key: "ip", label: t("col.ip"), render: (g) => <code>{g.ip}</code> },
                      { key: "claims", label: t("col.claims") },
                      { key: "requests", label: t("col.requests"), numeric: true },
                      { key: "uniquePaths", label: t("col.uniqueUrls"), numeric: true },
                      { key: "sampleUrls", label: t("col.examples"), className: "url", value: (g) => g.sampleUrls.join("  ") },
                      { key: "firstSeen", label: t("col.firstSeen"), render: (g) => fmtTs(g.firstSeen, locale) },
                      { key: "lastSeen", label: t("col.lastSeen"), render: (g) => fmtTs(g.lastSeen, locale) },
                    ]}
                  />
                </Card>
              </>
            )}
          </>
        )}
      </LogGate>
    </Page>
  );
}
