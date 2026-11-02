import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import type { Project } from "../lib/api";
import { useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { CF_TOKENS_URL, logApi } from "../lib/logapi";
import type { CfProgress, CfStatus, CfSyncResult } from "../lib/logapi";
import { fmtTs } from "./LogKit";
import { Badge, Card, ErrorAlert, Field } from "./ui";

/** Cloudflare connection for sites served entirely by Cloudflare (no server log). */
export default function CloudflareCard({ project, onImported }: { project: Project; onImported: () => void }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const [status, setStatus] = useState<CfStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [token, setToken] = useState("");
  const [zone, setZone] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<CfProgress | null>(null);
  const [result, setResult] = useState<CfSyncResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    try {
      setStatus(await logApi.cfStatus());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    const p = listen<CfProgress>("log-cf-progress", (e) => setProgress(e.payload));
    const d = listen("log-cf-done", () => {
      setProgress(null);
      void load();
      onImported();
    });
    return () => {
      void p.then((f) => f());
      void d.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const link = status?.links.find((l) => l.projectId === project.id);
  const zones = status?.zones ?? [];
  useEffect(() => {
    const match = zones.find((z) => project.domain && z.name.toLowerCase() === project.domain.toLowerCase());
    setZone(match?.id ?? zones[0]?.id ?? "");
  }, [project.id, project.domain, status?.zones]); // eslint-disable-line react-hooks/exhaustive-deps

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  const doSync = async () => {
    const r = await logApi.cfSync(project.id);
    setResult(r);
    if (r.error) setError(r.error);
    toast(t("cf.synced", { days: r.daysFetched, bots: formatNumber(r.botRequests, locale) }), r.error ? "error" : "success");
    onImported();
    await load();
  };

  const pct = progress && progress.total > 0 ? ((progress.index + 1) / progress.total) * 100 : 0;

  return (
    <Card title={t("cf.title")} hint={t("cf.hint")} actions={status?.connected ? <Badge tone="success">{t("cf.connected", { masked: status.masked ?? "" })}</Badge> : null}>
      {loading && <span className="spinner" />}
      {!loading && status && !status.connected && (
        <>
          <ol style={{ margin: 0, paddingLeft: 20, display: "flex", flexDirection: "column", gap: 6, lineHeight: 1.6 }}>
            <li>
              {t("cf.step1")} <button type="button" className="btn btn--sm" onClick={() => openUrl(CF_TOKENS_URL)}>{t("cf.openTokens")} ↗</button>
            </li>
            <li>{t("cf.step2")}</li>
            <li>{t("cf.step3")}</li>
            <li>{t("cf.step4")}</li>
            <li>{t("cf.step5")}</li>
          </ol>
          <div className="row">
            <input className="input" style={{ maxWidth: 420 }} type="password" autoComplete="off" placeholder={t("cf.tokenPlaceholder")} value={token} disabled={busy} onChange={(e) => setToken(e.target.value)} />
            <button type="button" className="btn btn--primary" disabled={busy || token.trim().length < 20} onClick={() => run(async () => {
              setStatus(await logApi.cfSetToken(token));
              setToken("");
              toast(t("cf.tokenSaved"), "success");
            })}>{busy ? <span className="spinner" /> : t("cf.connect")}</button>
          </div>
          <p className="muted" style={{ fontSize: 12.5 }}>{t("cf.tokenNote")}</p>
        </>
      )}
      {!loading && status?.connected && (
        <>
          <ErrorAlert error={status.error ?? null} />
          {link ? (
            <div className="row">
              <span>
                <b>{link.zoneName}</b> · {link.days ? t("cf.range", { from: fmtTs(link.firstDay, locale), to: fmtTs(link.lastDay, locale), days: link.days }) : t("cf.noDays")} · {t("cf.lastSync", { at: link.lastSyncAt ? fmtTs(link.lastSyncAt, locale) : "—" })}
              </span>
              <span className="spacer" />
              <button type="button" className="btn btn--primary" disabled={busy} onClick={() => run(doSync)}>{busy ? <span className="spinner" /> : "⟳"} {t("cf.syncNow")}</button>
            </div>
          ) : (
            <div className="row" style={{ alignItems: "flex-end" }}>
              <Field label={t("cf.zone")}>
                <select className="select" value={zone} disabled={busy || !zones.length} onChange={(e) => setZone(e.target.value)}>
                  {zones.map((z) => <option key={z.id} value={z.id}>{z.name}{z.plan ? ` (${z.plan})` : ""}</option>)}
                </select>
              </Field>
              <button type="button" className="btn btn--primary" disabled={busy || !zone} onClick={() => run(async () => {
                await logApi.cfLink(project.id, zone);
                await load();
                await doSync();
              })}>{busy ? <span className="spinner" /> : t("cf.linkAndSync")}</button>
            </div>
          )}
          {link?.lastError && !busy && <ErrorAlert error={link.lastError} />}
          {busy && progress && (
            <div className="field">
              <div className="progress"><div className="progress__bar" style={{ width: `${pct}%` }} /></div>
              <span className="muted" style={{ fontSize: 12.5 }}>{t("cf.progress", { n: progress.index + 1, total: progress.total, day: progress.day })}</span>
            </div>
          )}
          {result && !busy && !result.error && (
            <div className="alert alert--success">{t("cf.result", { days: result.daysFetched, requests: formatNumber(result.requests, locale), bots: formatNumber(result.botRequests, locale) })}</div>
          )}
          <p className="muted" style={{ fontSize: 12.5 }}>{t("cf.dataNote")}</p>
          <div className="row">
            <span className="spacer" />
            <button type="button" className="btn btn--ghost btn--sm" disabled={busy} onClick={() => {
              if (!window.confirm(t("cf.disconnectConfirm"))) return;
              void run(async () => setStatus(await logApi.cfDeleteToken()));
            }}>{t("cf.disconnect")}</button>
          </div>
        </>
      )}
      <ErrorAlert error={error} />
    </Card>
  );
}
