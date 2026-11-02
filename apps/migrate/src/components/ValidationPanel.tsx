// Validation panel: chains, loops, missing / non-200 targets, self and redundant rules.
import { useState } from "react";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { ISSUE_KINDS, issueTone, pathOf } from "../lib/migrate";
import { Badge, Card, ErrorAlert, Loading } from "./ui";

export default function ValidationPanel({ planId, version, onSelect, onChanged }: { planId: number; version: number; onSelect?: (mappingId: number) => void; onChanged: () => void }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const v = useAsync(() => migrate.validate(planId), [planId, version]);
  const [kind, setKind] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const collapse = async () => {
    setBusy(true);
    try {
      const n = await migrate.collapseChains(planId);
      toast(t("validation.collapsed", { count: formatNumber(n, locale) }), "success");
      onChanged();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const issues = (v.data?.issues ?? []).filter((i) => !kind || i.kind === kind);
  return (
    <Card
      title={t("validation.title")}
      hint={v.data ? (v.data.blocking ? t("validation.blocking", { count: formatNumber(v.data.blocking, locale) }) : t("validation.clean")) : undefined}
      actions={
        <button type="button" className="btn btn--sm" disabled={busy || !v.data?.counts.chain} onClick={collapse}>
          {t("validation.collapse")}
        </button>
      }
    >
      {v.loading && !v.data && <Loading />}
      <ErrorAlert error={v.error} />
      {v.data && (
        <>
          <div className="row" style={{ gap: 6 }}>
            {ISSUE_KINDS.map((k) => (
              <button key={k} type="button" className={`chip ${kind === k ? "chip--active" : ""}`} onClick={() => setKind(kind === k ? null : k)} disabled={!v.data!.counts[k]}>
                <Badge tone={v.data!.counts[k] ? issueTone(k) : undefined}>{formatNumber(v.data!.counts[k] ?? 0, locale)}</Badge> {t(`issue.${k}`)}
              </button>
            ))}
          </div>
          {issues.length > 0 && (
            <div className="issue-list">
              {issues.slice(0, 80).map((i, idx) => (
                <button key={`${i.kind}-${i.mappingId}-${idx}`} type="button" className="issue" onClick={() => onSelect?.(i.mappingId)}>
                  <Badge tone={issueTone(i.kind)}>{t(`issue.${i.kind}`)}</Badge>
                  <span className="mono truncate">{pathOf(i.source)} → {i.target ? pathOf(i.target) : "410"}</span>
                  {i.detail && <span className="muted truncate" style={{ fontSize: 11.5 }}>{i.kind === "multi_source" ? t("validation.sources", { count: i.detail }) : i.detail}</span>}
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </Card>
  );
}
