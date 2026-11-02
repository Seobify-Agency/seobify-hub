// Screen 7 — Overview: plan progress (match rate, approval rate, last verification), status
// mix, matching stages, validation summary, next steps and recent activity.
import { openPath } from "@tauri-apps/plugin-opener";
import { useI18n } from "../i18n";
import { api, migrate } from "../lib/api";
import type { Overview, PlanSummary } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatDate, formatNumber, formatPercent } from "../lib/format";
import { ISSUE_KINDS, STATUSES, issueTone, statusColor } from "../lib/migrate";
import { usePlan } from "../lib/plan";
import type { Navigate, PageId } from "../nav";
import { BarList, ShareBar } from "../components/charts";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import { Badge, Card, ErrorAlert, Loading, Page, Stat } from "../components/ui";

export default function Dashboard({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("dashboard.welcome")} subtitle={t("app.tagline")} onNavigate={onNavigate}>
      {(plan) => <OverviewInner plan={plan} onNavigate={onNavigate} />}
    </PlanGate>
  );
}

function steps(ov: Overview): { key: string; done: boolean; page: PageId }[] {
  const s = ov.stats.byStatus;
  const open = (s.weak ?? 0) + (s.review ?? 0);
  return [
    { key: "import", done: ov.old.total > 0 && ov.new.total > 0, page: "sources" },
    { key: "match", done: ov.stats.total > 0 && ov.stats.unmapped === 0, page: "mapping" },
    { key: "review", done: ov.stats.total > 0 && (open === 0 || ov.approvalRate >= 0.95), page: "mapping" },
    { key: "validate", done: ov.stats.total > 0 && ov.blockingIssues === 0, page: "mapping" },
    { key: "staging", done: !!ov.lastStaging, page: "verify" },
    { key: "verify", done: !!ov.lastPost && ov.lastPost.failed === 0, page: "verify" },
  ];
}

function OverviewInner({ plan, onNavigate }: { plan: PlanSummary; onNavigate: Navigate }) {
  const { t, lang, locale } = useI18n();
  const toast = useToast();
  const { version } = usePlan();
  const ov = useAsync(() => migrate.overview(plan.id), [plan.id, version]);
  const activity = useAsync(() => api.recentActivity(plan.projectId, 12), [plan.projectId, version]);

  const report = async () => {
    try {
      const r = await migrate.report(plan.id, lang);
      toast(t("common.reportSaved", { path: r.html }), "success");
      void openPath(r.html).catch(() => {});
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const o = ov.data;
  const last = o?.lastPost ?? null;
  return (
    <Page
      title={plan.name}
      subtitle={`${plan.oldBase} → ${plan.newBase}${plan.stagingBase ? ` · staging: ${plan.stagingBase}` : ""}`}
      actions={
        <>
          <PlanSelect />
          <button type="button" className="btn" onClick={report} disabled={!o?.stats.total}>{t("common.exportReport")}</button>
        </>
      }
    >
      <ErrorAlert error={ov.error} />
      {!o && ov.loading && <Loading />}
      {o && (
        <>
          <div className="grid grid--stats">
            <Stat label={t("overview.oldUrls")} value={formatNumber(o.old.total, locale)} sub={t("overview.withTitle", { count: formatNumber(o.old.withTitle, locale) })} />
            <Stat label={t("overview.newUrls")} value={formatNumber(o.new.total, locale)} sub={t("overview.withTitle", { count: formatNumber(o.new.withTitle, locale) })} />
            <Stat label={t("overview.matchRate")} value={o.old.total ? formatPercent(o.matchRate, locale) : "—"} sub={t("overview.matchRateHint")} />
            <Stat label={t("overview.approvalRate")} value={o.stats.total ? formatPercent(o.approvalRate, locale) : "—"} sub={t("overview.approvedN", { count: formatNumber(o.stats.approved, locale) })} />
            <Stat label={t("overview.patterns")} value={formatNumber(o.patterns, locale)} />
            <Stat
              label={t("overview.lastVerify")}
              value={last ? formatPercent(last.total ? last.ok / last.total : 0, locale) : "—"}
              sub={last ? `${formatNumber(last.ok, locale)} / ${formatNumber(last.total, locale)} · ${formatDate(last.finishedAt, locale)}` : t("overview.notVerified")}
            />
          </div>
          <div className="grid grid--2">
            <Card title={t("overview.statusMix")}>
              {o.stats.total ? (
                <ShareBar parts={STATUSES.map((s) => ({ label: t(`status.${s}`), value: o.stats.byStatus[s] ?? 0, color: statusColor(s) }))} />
              ) : (
                <p className="muted">{t("overview.noMappings")}</p>
              )}
              {o.stats.unmapped > 0 && <p className="muted" style={{ fontSize: 12.5 }}>{t("overview.unmapped", { count: formatNumber(o.stats.unmapped, locale) })}</p>}
              <h3 className="card__title" style={{ marginTop: 6 }}>{t("overview.stages")}</h3>
              <BarList
                items={Object.entries(o.stats.byReason)
                  .sort((a, b) => b[1] - a[1])
                  .map(([k, v]) => ({ label: t(`reason.${k}`), value: v }))}
              />
            </Card>
            <Card title={t("overview.nextSteps")}>
              <div className="steps">
                {steps(o).map((s, i) => (
                  <button key={s.key} type="button" className={`step ${s.done ? "step--done" : ""}`} onClick={() => onNavigate(s.page)}>
                    <span className="step__n">{s.done ? "✓" : i + 1}</span>
                    <span>
                      <b>{t(`step.${s.key}`)}</b>
                      <span className="muted step__hint">{t(`step.${s.key}Hint`)}</span>
                    </span>
                  </button>
                ))}
              </div>
              <h3 className="card__title" style={{ marginTop: 6 }}>{t("validation.title")}</h3>
              <div className="row" style={{ gap: 6 }}>
                {ISSUE_KINDS.map((k) => (
                  <span key={k} className="chip" style={{ cursor: "default" }}>
                    <Badge tone={o.validation[k] ? issueTone(k) : undefined}>{formatNumber(o.validation[k] ?? 0, locale)}</Badge> {t(`issue.${k}`)}
                  </span>
                ))}
              </div>
            </Card>
          </div>
        </>
      )}
      <Card title={t("dashboard.recent")}>
        <ErrorAlert error={activity.error} />
        {activity.data && activity.data.length === 0 && <p className="muted">{t("dashboard.noActivity")}</p>}
        {activity.data?.map((a) => (
          <div key={a.id} className="row" style={{ fontSize: 13 }}>
            <span className="muted nowrap">{formatDate(a.at, locale)}</span>
            <span>{a.message}</span>
          </div>
        ))}
      </Card>
    </Page>
  );
}
