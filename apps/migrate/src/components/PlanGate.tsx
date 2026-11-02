// Plan selector and the "pick a project / create a plan first" gate used by Migrate screens.
import type { ReactNode } from "react";
import { useI18n } from "../i18n";
import type { PlanSummary } from "../lib/api";
import { useProjects } from "../lib/context";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import { Empty, Loading, Page } from "./ui";

export function PlanSelect() {
  const { t } = useI18n();
  const { plans, plan, setPlanId } = usePlan();
  if (!plans.length) return null;
  return (
    <label className="row" style={{ gap: 8, fontSize: 12 }} title={plan ? `${plan.oldBase} → ${plan.newBase}` : undefined}>
      <span className="muted">{t("plan.label")}</span>
      <select className="select" style={{ minWidth: 220 }} value={plan?.id ?? ""} onChange={(e) => setPlanId(Number(e.target.value))}>
        {plans.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
    </label>
  );
}

export default function PlanGate({ title, subtitle, onNavigate, children }: { title: string; subtitle?: ReactNode; onNavigate: Navigate; children: (plan: PlanSummary) => ReactNode }) {
  const { t } = useI18n();
  const { projects, loading: projectsLoading } = useProjects();
  const { plan, loading } = usePlan();
  if (!projects.length) {
    return (
      <Page title={title}>
        {projectsLoading ? (
          <Loading />
        ) : (
          <Empty title={t("project.emptyTitle")} text={t("project.emptyText")} action={<button type="button" className="btn btn--primary" onClick={() => onNavigate("projects")}>{t("project.new")}</button>} />
        )}
      </Page>
    );
  }
  if (!plan) {
    return (
      <Page title={title} subtitle={subtitle}>
        {loading ? (
          <Loading />
        ) : (
          <Empty icon="▦" title={t("plan.emptyTitle")} text={t("plan.emptyText")} action={<button type="button" className="btn btn--primary" onClick={() => onNavigate("plans")}>{t("plan.new")}</button>} />
        )}
      </Page>
    );
  }
  return <>{children(plan)}</>;
}
