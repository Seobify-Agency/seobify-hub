// Screen 1 — Plans: list, create, edit and delete migration plans; matching options.
import { useState } from "react";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import type { MatchOptions, PlanSummary } from "../lib/api";
import { useProjects, useToast } from "../lib/context";
import { formatDate, formatNumber } from "../lib/format";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import { Badge, Card, DataTable, Empty, ErrorAlert, Field, Page } from "../components/ui";

const DEFAULT_OPTIONS: MatchOptions = { caseInsensitive: true, ignoreQuery: true, goneWithoutValue: false, fuzzyMin: 35 };

interface Form {
  name: string;
  oldBase: string;
  newBase: string;
  stagingBase: string;
  notes: string;
  options: MatchOptions;
}

const EMPTY: Form = { name: "", oldBase: "", newBase: "", stagingBase: "", notes: "", options: DEFAULT_OPTIONS };

export default function Plans({ onNavigate }: { onNavigate: Navigate }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { project, projects } = useProjects();
  const { plans, plan, setPlanId, bump, error } = usePlan();
  const [form, setForm] = useState<Form>(() => ({ ...EMPTY, oldBase: project?.domain ? `https://${project.domain}` : "" }));
  const [editing, setEditing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  if (!projects.length || !project) {
    return (
      <Page title={t("nav.plans")}>
        <Empty title={t("project.emptyTitle")} text={t("project.emptyText")} action={<button type="button" className="btn btn--primary" onClick={() => onNavigate("projects")}>{t("project.new")}</button>} />
      </Page>
    );
  }

  const set = <K extends keyof Form>(k: K, v: Form[K]) => setForm((f) => ({ ...f, [k]: v }));
  const setOpt = <K extends keyof MatchOptions>(k: K, v: MatchOptions[K]) => setForm((f) => ({ ...f, options: { ...f.options, [k]: v } }));

  const save = async () => {
    setBusy(true);
    setFormError(null);
    try {
      const input = { name: form.name, oldBase: form.oldBase, newBase: form.newBase, stagingBase: form.stagingBase || null, notes: form.notes || null, options: form.options };
      const saved = editing ? await migrate.updatePlan(editing, input) : await migrate.createPlan(project.id, input);
      setPlanId(saved.id);
      bump();
      toast(t(editing ? "plan.updated" : "plan.created", { name: saved.name }), "success");
      setEditing(null);
      setForm({ ...EMPTY, oldBase: project.domain ? `https://${project.domain}` : "" });
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const edit = (p: PlanSummary) => {
    setEditing(p.id);
    setForm({ name: p.name, oldBase: p.oldBase, newBase: p.newBase, stagingBase: p.stagingBase ?? "", notes: p.notes ?? "", options: { ...DEFAULT_OPTIONS, ...p.options } });
  };

  const remove = async (p: PlanSummary) => {
    if (!window.confirm(t("common.confirmDelete", { name: p.name }))) return;
    try {
      await migrate.deletePlan(p.id);
      if (plan?.id === p.id) setPlanId(null);
      bump();
      toast(t("plan.deleted"), "success");
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  return (
    <Page title={t("nav.plans")} subtitle={t("plan.subtitle")}>
      <ErrorAlert error={error} />
      <Card title={editing ? t("plan.edit") : t("plan.new")} hint={t("plan.hint")}>
        <div className="grid grid--3">
          <Field label={t("plan.name")}>
            <input className="input" value={form.name} placeholder={t("plan.namePlaceholder")} onChange={(e) => set("name", e.target.value)} />
          </Field>
          <Field label={t("plan.oldBase")}>
            <input className="input" value={form.oldBase} placeholder="https://eski-site.com" onChange={(e) => set("oldBase", e.target.value)} />
          </Field>
          <Field label={t("plan.newBase")}>
            <input className="input" value={form.newBase} placeholder="https://yeni-site.com" onChange={(e) => set("newBase", e.target.value)} />
          </Field>
          <Field label={t("plan.stagingBase")} hint={t("plan.stagingHint")}>
            <input className="input" value={form.stagingBase} placeholder="https://staging.yeni-site.com" onChange={(e) => set("stagingBase", e.target.value)} />
          </Field>
          <Field label={t("plan.notes")}>
            <input className="input" value={form.notes} onChange={(e) => set("notes", e.target.value)} />
          </Field>
          <Field label={t("plan.fuzzyMin")} hint={t("plan.fuzzyMinHint")}>
            <input className="input" type="number" min={10} max={89} value={form.options.fuzzyMin} onChange={(e) => setOpt("fuzzyMin", Math.max(10, Math.min(89, Number(e.target.value) || 35)))} />
          </Field>
        </div>
        <div className="row" style={{ gap: 18 }}>
          <label className="checkbox">
            <input type="checkbox" checked={form.options.caseInsensitive} onChange={(e) => setOpt("caseInsensitive", e.target.checked)} />
            {t("plan.optCase")}
          </label>
          <label className="checkbox">
            <input type="checkbox" checked={form.options.ignoreQuery} onChange={(e) => setOpt("ignoreQuery", e.target.checked)} />
            {t("plan.optQuery")}
          </label>
          <label className="checkbox">
            <input type="checkbox" checked={form.options.goneWithoutValue} onChange={(e) => setOpt("goneWithoutValue", e.target.checked)} />
            {t("plan.optGone")}
          </label>
        </div>
        <ErrorAlert error={formError} />
        <div className="row">
          <button type="button" className="btn btn--primary" disabled={busy || !form.name.trim() || !form.oldBase.trim() || !form.newBase.trim()} onClick={save}>
            {editing ? t("common.save") : t("common.create")}
          </button>
          {editing && (
            <button type="button" className="btn" onClick={() => { setEditing(null); setForm(EMPTY); }}>
              {t("common.cancel")}
            </button>
          )}
        </div>
      </Card>
      {plans.length === 0 ? (
        <Empty icon="▦" title={t("plan.emptyTitle")} text={t("plan.emptyText")} />
      ) : (
        <Card title={t("plan.list")}>
          <DataTable
            rows={plans}
            rowKey={(p) => p.id}
            onRowClick={(p) => setPlanId(p.id)}
            initialSort={{ key: "updatedAt", desc: true }}
            columns={[
              { key: "name", label: t("plan.name"), render: (p) => <b style={{ color: p.id === plan?.id ? "var(--accent-hover)" : undefined }}>{p.id === plan?.id ? "● " : ""}{p.name}</b> },
              { key: "oldBase", label: t("plan.bases"), render: (p) => <span className="mono">{p.oldBase} → {p.newBase}</span> },
              { key: "oldUrls", label: t("plan.oldUrls"), numeric: true },
              { key: "newUrls", label: t("plan.newUrls"), numeric: true },
              { key: "mapped", label: t("plan.mapped"), numeric: true },
              { key: "approved", label: t("plan.approved"), numeric: true, render: (p) => <Badge tone={p.approved && p.approved === p.mapped ? "success" : undefined}>{formatNumber(p.approved, locale)}</Badge> },
              { key: "patterns", label: t("nav.patterns"), numeric: true },
              { key: "updatedAt", label: t("plan.updatedAt"), render: (p) => <span className="muted">{formatDate(p.updatedAt, locale)}</span> },
              {
                key: "actions",
                label: "",
                sortable: false,
                render: (p) => (
                  <span className="row" style={{ gap: 6, flexWrap: "nowrap" }}>
                    <button type="button" className="btn btn--sm" onClick={(e) => { e.stopPropagation(); setPlanId(p.id); onNavigate("sources"); }}>{t("plan.open")}</button>
                    <button type="button" className="btn btn--sm btn--ghost" onClick={(e) => { e.stopPropagation(); edit(p); }}>{t("plan.editShort")}</button>
                    <button type="button" className="btn btn--sm btn--danger" onClick={(e) => { e.stopPropagation(); void remove(p); }}>{t("common.delete")}</button>
                  </span>
                ),
              },
            ]}
          />
        </Card>
      )}
    </Page>
  );
}
