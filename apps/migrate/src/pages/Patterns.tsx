// Screen 4 — Pattern rules: ordered regex list with live preview of affected URLs.
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import type { PatternPreview, PatternWithCounts, PlanSummary } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { pathOf } from "../lib/migrate";
import { usePlan } from "../lib/plan";
import type { Navigate } from "../nav";
import PatternSuggestions from "../components/PatternSuggestions";
import PlanGate, { PlanSelect } from "../components/PlanGate";
import { Badge, Card, Empty, ErrorAlert, Field, Loading, Page } from "../components/ui";

export default function Patterns({ onNavigate }: { onNavigate: Navigate }) {
  const { t } = useI18n();
  return (
    <PlanGate title={t("nav.patterns")} onNavigate={onNavigate}>
      {(plan) => <PatternsInner plan={plan} />}
    </PlanGate>
  );
}

function PatternsInner({ plan }: { plan: PlanSummary }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { version, bump } = usePlan();
  const list = useAsync(() => migrate.listPatterns(plan.id), [plan.id, version]);
  const [regex, setRegex] = useState("");
  const [replacement, setReplacement] = useState("");
  const [note, setNote] = useState("");
  const [preview, setPreview] = useState<PatternPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!regex.trim() || !replacement.trim()) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    const id = window.setTimeout(() => {
      migrate
        .previewPattern(plan.id, regex, replacement)
        .then((p) => {
          setPreview(p);
          setPreviewError(null);
        })
        .catch((e) => {
          setPreview(null);
          setPreviewError(e instanceof Error ? e.message : String(e));
        });
    }, 350);
    return () => window.clearTimeout(id);
  }, [regex, replacement, plan.id]);

  const act = async (fn: () => Promise<unknown>, success?: string) => {
    setBusy(true);
    try {
      await fn();
      if (success) toast(success, "success");
      bump();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const add = () =>
    act(async () => {
      await migrate.addPattern(plan.id, regex, replacement, note);
      setRegex("");
      setReplacement("");
      setNote("");
    }, t("patterns.added"));

  const rematch = () =>
    act(async () => {
      const r = await migrate.autoMatch(plan.id, true);
      toast(t("patterns.applied", { count: formatNumber(r.processed, locale) }), "success");
    });

  const toggle = (p: PatternWithCounts) => act(() => migrate.updatePattern({ ...p, enabled: !p.enabled }));

  return (
    <Page
      title={t("nav.patterns")}
      subtitle={t("patterns.subtitle")}
      actions={
        <>
          <PlanSelect />
          <button type="button" className="btn btn--primary" disabled={busy || !list.data?.length} onClick={rematch}>{t("patterns.apply")}</button>
        </>
      }
    >
      <Card title={t("patterns.new")} hint={t("patterns.newHint")}>
        <div className="grid grid--3">
          <Field label={t("patterns.regex")}>
            <input className="input mono" value={regex} placeholder="^/blog/\d{4}/\d{2}/(.+)$" onChange={(e) => setRegex(e.target.value)} />
          </Field>
          <Field label={t("patterns.replacement")}>
            <input className="input mono" value={replacement} placeholder="/makale/$1" onChange={(e) => setReplacement(e.target.value)} />
          </Field>
          <Field label={t("patterns.note")}>
            <input className="input" value={note} onChange={(e) => setNote(e.target.value)} />
          </Field>
        </div>
        <ErrorAlert error={previewError} />
        {preview && (
          <div className="field">
            <div className="alert alert--info">{t("patterns.previewSummary", { matched: formatNumber(preview.matched, locale), found: formatNumber(preview.targetsFound, locale) })}</div>
            {preview.samples.length > 0 && (
              <div className="table-wrap" style={{ maxHeight: 240 }}>
                <table className="table">
                  <thead>
                    <tr>
                      <th>{t("mapping.oldUrl")}</th>
                      <th>{t("patterns.target")}</th>
                      <th>{t("patterns.inNewSet")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {preview.samples.map((s) => (
                      <tr key={s.oldUrl}>
                        <td className="url">{pathOf(s.oldUrl)}</td>
                        <td className="url">{pathOf(s.target)}</td>
                        <td>{s.exists ? <Badge tone="success">✓</Badge> : <Badge tone="warning">{t("patterns.missing")}</Badge>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        )}
        <div className="row">
          <button type="button" className="btn btn--primary" disabled={busy || !regex.trim() || !replacement.trim() || !!previewError} onClick={add}>{t("patterns.add")}</button>
        </div>
      </Card>
      <Card title={t("patterns.list")} hint={t("patterns.listHint")}>
        {list.loading && !list.data && <Loading />}
        <ErrorAlert error={list.error} />
        {list.data && list.data.length === 0 && <Empty icon="✱" title={t("patterns.emptyTitle")} text={t("patterns.emptyText")} />}
        {list.data && list.data.length > 0 && (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th className="num">#</th>
                  <th>{t("patterns.regex")}</th>
                  <th>{t("patterns.replacement")}</th>
                  <th className="num">{t("patterns.matched")}</th>
                  <th className="num">{t("patterns.found")}</th>
                  <th>{t("patterns.note")}</th>
                  <th>{t("patterns.enabled")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {list.data.map((p, i) => (
                  <tr key={p.id} style={{ opacity: p.enabled ? 1 : 0.55 }}>
                    <td className="num">{p.position}</td>
                    <td className="mono">{p.regex}</td>
                    <td className="mono">{p.replacement}</td>
                    <td className="num">{formatNumber(p.matched, locale)}</td>
                    <td className="num"><Badge tone={p.matched && p.targetsFound === p.matched ? "success" : p.targetsFound ? "warning" : undefined}>{formatNumber(p.targetsFound, locale)}</Badge></td>
                    <td className="muted">{p.note ?? ""}</td>
                    <td>
                      <input type="checkbox" checked={p.enabled} disabled={busy} onChange={() => toggle(p)} />
                    </td>
                    <td className="nowrap">
                      <button type="button" className="btn btn--sm btn--ghost" disabled={busy || i === 0} onClick={() => act(() => migrate.movePattern(p.id, -1))} title={t("patterns.up")}>↑</button>
                      <button type="button" className="btn btn--sm btn--ghost" disabled={busy || i === list.data!.length - 1} onClick={() => act(() => migrate.movePattern(p.id, 1))} title={t("patterns.down")}>↓</button>
                      <button type="button" className="btn btn--sm btn--danger" disabled={busy} onClick={() => window.confirm(t("common.confirmDelete", { name: p.regex })) && act(() => migrate.deletePattern(p.id), t("patterns.deleted"))}>{t("common.delete")}</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
      <PatternSuggestions planId={plan.id} version={version} onApplied={bump} />
    </Page>
  );
}
