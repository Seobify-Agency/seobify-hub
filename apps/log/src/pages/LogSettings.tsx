// Seobify Log settings cards (rendered inside the hub Settings page).
import { useEffect, useState } from "react";
import { fmtTs } from "../components/LogKit";
import { Badge, Card, DataTable, ErrorAlert, Field, Segment } from "../components/ui";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { logApi } from "../lib/logapi";

export default function LogSettings() {
  const { t, locale } = useI18n();
  const toast = useToast();
  const [settings, setSettings] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ranges = useAsync(() => logApi.rangeStatus(), []);

  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const update = async (patch: Record<string, unknown>) => {
    const next = { ...(settings ?? {}), ...patch };
    setSettings(next);
    try {
      await api.saveSettings(next);
      toast(t("common.saved"), "success");
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const ipMode = settings?.ipMode === "hash" ? "hash" : "mask";
  const verifyDns = settings?.verifyDns !== false;
  const rawErrors = settings?.storeRawErrors === true;

  return (
    <>
      <Card title={t("set.privacy")} hint={t("set.privacyHint")}>
        <ErrorAlert error={error} />
        <Field label={t("set.ipMode")} hint={t("set.ipModeHint")}>
          <Segment value={ipMode} onChange={(v) => update({ ipMode: v })} options={[{ value: "mask", label: t("set.ipMask") }, { value: "hash", label: t("set.ipHash") }]} />
        </Field>
        <label className="checkbox">
          <input type="checkbox" checked={rawErrors} onChange={(e) => update({ storeRawErrors: e.target.checked })} /> {t("set.rawErrors")}
        </label>
      </Card>
      <Card
        title={t("set.verification")}
        hint={t("set.verificationHint")}
        actions={
          <>
            <button type="button" className="btn btn--sm" disabled={busy} onClick={async () => {
              setBusy(true);
              try {
                await logApi.refreshRanges();
                ranges.reload();
                toast(t("set.refreshed"), "success");
              } catch (e) {
                toast(e instanceof Error ? e.message : String(e), "error");
              } finally {
                setBusy(false);
              }
            }}>{busy ? <span className="spinner" /> : "↻"} {t("set.refresh")}</button>
            <button type="button" className="btn btn--sm" onClick={async () => {
              const r = await logApi.clearIpCache();
              ranges.reload();
              toast(t("set.cacheCleared", { count: r.deleted }), "success");
            }}>{t("set.clearCache")}</button>
          </>
        }
      >
        <label className="checkbox">
          <input type="checkbox" checked={verifyDns} onChange={(e) => update({ verifyDns: e.target.checked })} /> {t("set.verifyDns")}
        </label>
        <ErrorAlert error={ranges.error} />
        {ranges.data && (
          <>
            <span className="muted" style={{ fontSize: 12.5 }}>{t("set.cacheEntries", { count: formatNumber(ranges.data.dnsCacheEntries, locale) })}</span>
            <DataTable
              rows={ranges.data.sources}
              rowKey={(s) => s.source}
              searchable={false}
              columns={[
                { key: "owner", label: t("col.owner") },
                { key: "url", label: t("col.source"), className: "url" },
                { key: "prefixes", label: t("col.prefixes"), numeric: true },
                { key: "fetchedAt", label: t("col.fetched"), render: (s) => fmtTs(s.fetchedAt, locale) },
                { key: "error", label: t("col.state"), render: (s) => (s.error ? <span title={s.error}><Badge tone="warning">{t("set.stale")}</Badge></span> : s.fetchedAt ? <Badge tone="success">OK</Badge> : <Badge>{t("set.notYet")}</Badge>) },
              ]}
            />
          </>
        )}
      </Card>
    </>
  );
}
