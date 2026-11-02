// "These N URLs follow /urunler/ → /magaza/" suggestions with a one-click pattern rule.
import { useState } from "react";
import { useI18n } from "../i18n";
import { migrate } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { formatNumber } from "../lib/format";
import { describeMove, pathOf } from "../lib/migrate";
import { Card, ErrorAlert, Loading } from "./ui";

export default function PatternSuggestions({ planId, version, onApplied }: { planId: number; version: number; onApplied: () => void }) {
  const { t, locale } = useI18n();
  const toast = useToast();
  const list = useAsync(() => migrate.suggestPatterns(planId), [planId, version]);
  const [busy, setBusy] = useState<string | null>(null);

  const add = async (regex: string, replacement: string) => {
    setBusy(regex);
    try {
      await migrate.addPattern(planId, regex, replacement, t("patterns.suggested"));
      const r = await migrate.autoMatch(planId, true);
      toast(t("patterns.addedAndMatched", { count: formatNumber(r.processed, locale) }), "success");
      onApplied();
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card title={t("patterns.suggestTitle")} hint={t("patterns.suggestHint")}>
      {list.loading && <Loading />}
      <ErrorAlert error={list.error} />
      {list.data && list.data.length === 0 && <p className="muted" style={{ fontSize: 13 }}>{t("patterns.noSuggestions")}</p>}
      <div className="stack">
        {list.data?.map((s) => {
          const move = describeMove(s.regex, s.replacement);
          return (
            <div key={s.regex + s.replacement} className="suggestion">
              <div>
                {t("patterns.suggestion", { count: formatNumber(s.affected, locale) })} <code>{move.from}</code> → <code>{move.to}</code> {t("patterns.suggestionEnd", { support: formatNumber(s.support, locale) })}
              </div>
              <div className="mono muted" style={{ fontSize: 11.5 }}>
                {s.regex} → {s.replacement}
              </div>
              {s.examples[0] && (
                <div className="muted" style={{ fontSize: 11.5 }}>
                  {t("patterns.example")}: {pathOf(s.examples[0][0])} → {pathOf(s.examples[0][1])}
                </div>
              )}
              <div className="row">
                <button type="button" className="btn btn--sm btn--primary" disabled={busy !== null} onClick={() => add(s.regex, s.replacement)}>
                  {busy === s.regex ? t("common.loading") : t("patterns.addRule")}
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}
