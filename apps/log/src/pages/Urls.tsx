import { useEffect, useState } from "react";
import { CsvButton, LogGate, RangeBar, StatusMix, fmtTs, useLogData } from "../components/LogKit";
import { Card, DataTable, ErrorAlert, Field, Loading, Page } from "../components/ui";
import { useI18n } from "../i18n";
import { formatNumber } from "../lib/format";
import { CATEGORIES, logApi } from "../lib/logapi";
import type { UrlFilter } from "../lib/logapi";

export default function Urls() {
  const { t, locale } = useI18n();
  const [bot, setBot] = useState("all");
  const [status, setStatus] = useState("all");
  const [dir, setDir] = useState("all");
  const [params, setParams] = useState("all");
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [bots, setBots] = useState<{ id: string; name: string }[]>([]);
  useEffect(() => {
    const h = window.setTimeout(() => setQuery(search), 350);
    return () => window.clearTimeout(h);
  }, [search]);
  const filter: UrlFilter = {
    bot: bot === "all" ? null : bot,
    status: status === "all" ? null : status,
    dir: dir === "all" ? null : dir,
    params: params === "all" ? null : params,
    search: query || null,
    limit: 2000,
  };
  const { data, error, loading } = useLogData((pid, range) => logApi.urls(pid, range, filter), [bot, status, dir, params, query]);
  const botList = useLogData((pid, range) => logApi.botStats(pid, range, null));
  useEffect(() => {
    if (botList.data) setBots(botList.data.bots.map((b) => ({ id: b.botId, name: b.name })));
  }, [botList.data]);
  const rows = data?.urls ?? [];
  return (
    <Page title={t("nav.urls")} subtitle={t("urls.subtitle")} actions={<RangeBar />}>
      <LogGate>
        <Card>
          <div className="grid grid--3" style={{ gridTemplateColumns: "repeat(5, minmax(0, 1fr))" }}>
            <Field label={t("urls.bot")}>
              <select className="select" value={bot} onChange={(e) => setBot(e.target.value)}>
                <option value="all">{t("urls.allBots")}</option>
                <option value="googlebot">{t("urls.googlebot")}</option>
                <optgroup label={t("col.category")}>
                  {CATEGORIES.map((c) => <option key={c} value={c}>{t(`cat.${c}`)}</option>)}
                </optgroup>
                <optgroup label={t("col.bot")}>
                  {bots.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
                </optgroup>
              </select>
            </Field>
            <Field label={t("urls.status")}>
              <select className="select" value={status} onChange={(e) => setStatus(e.target.value)}>
                <option value="all">{t("common.all")}</option>
                {["2xx", "3xx", "4xx", "5xx"].map((s) => <option key={s} value={s}>{s}</option>)}
                <option value="error">4xx + 5xx</option>
              </select>
            </Field>
            <Field label={t("urls.dir")}>
              <select className="select" value={dir} onChange={(e) => setDir(e.target.value)}>
                <option value="all">{t("common.all")}</option>
                {(data?.dirs ?? []).map((d) => <option key={d.dir} value={d.dir}>{d.dir} ({formatNumber(d.hits, locale)})</option>)}
              </select>
            </Field>
            <Field label={t("urls.params")}>
              <select className="select" value={params} onChange={(e) => setParams(e.target.value)}>
                <option value="all">{t("common.all")}</option>
                <option value="with">{t("urls.withParams")}</option>
                <option value="without">{t("urls.withoutParams")}</option>
              </select>
            </Field>
            <Field label={t("urls.search")}>
              <input className="input" value={search} placeholder="/blog/" onChange={(e) => setSearch(e.target.value)} />
            </Field>
          </div>
        </Card>
        <ErrorAlert error={error} />
        <Card title={data ? t("urls.total", { total: formatNumber(data.total, locale) }) : undefined}>
          {loading && !data ? (
            <Loading />
          ) : (
            <DataTable
              rows={rows}
              rowKey={(u) => u.url}
              searchable={false}
              limit={2000}
              initialSort={{ key: "hits", desc: true }}
              toolbar={
                <CsvButton
                  filename="urls.csv"
                  headers={["url", "bot_hits", "googlebot_hits", "last_googlebot", "first_seen", "last_seen", "2xx", "3xx", "4xx", "5xx", "hits_per_day"]}
                  rows={rows.map((u) => [u.url, u.hits, u.googlebotHits, u.lastGooglebot, u.firstSeen, u.lastSeen, u.s2xx, u.s3xx, u.s4xx, u.s5xx, u.perDay.toFixed(2)])}
                />
              }
              columns={[
                { key: "url", label: t("col.url"), className: "url" },
                { key: "hits", label: t("col.botHits"), numeric: true },
                { key: "googlebotHits", label: t("col.googlebot"), numeric: true },
                { key: "lastGooglebot", label: t("col.lastGooglebot"), render: (u) => fmtTs(u.lastGooglebot, locale) },
                { key: "firstSeen", label: t("col.firstSeen"), render: (u) => fmtTs(u.firstSeen, locale) },
                { key: "status", label: "2xx · 3xx · 4xx · 5xx", sortable: false, render: (u) => <StatusMix {...u} /> },
                { key: "perDay", label: t("col.perDay"), numeric: true, render: (u) => formatNumber(u.perDay, locale, 2) },
              ]}
            />
          )}
        </Card>
      </LogGate>
    </Page>
  );
}
