import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import type { ReactNode } from "react";
import { useI18n } from "../i18n";
import type { Lang } from "../i18n";
import { api } from "../lib/api";
import { useAsync, useToast } from "../lib/context";
import { Card, ErrorAlert, Field, Loading, Page, Segment } from "../components/ui";

const REPO_URL = "https://github.com/Seobify-Agency/seobify-hub";
/** Expert-support link; it carries a source tag only, no personal data. */
const supportUrl = (lang: Lang, product = "Seobify Hub") =>
  `${lang === "en" ? "https://seobify.com/en/contact/" : "https://seobify.com/iletisim/"}?utm_source=${product.toLowerCase().replace(/\s+/g, "-")}&utm_medium=app&utm_campaign=seobify-hub`;

/** Tool pages can pass extra cards (API keys, schedules…) as children. */
export default function Settings({ children }: { children?: ReactNode }) {
  const { t, lang, setLang } = useI18n();
  const toast = useToast();
  const info = useAsync(() => api.appInfo(), []);
  const prefs = useAsync(() => api.getSettings(), []);
  const supportLine = prefs.data?.reportSupportLine !== false;
  const setSupportLine = async (on: boolean) => {
    const next = { ...(prefs.data ?? {}), reportSupportLine: on };
    await api.saveSettings(next);
    prefs.setData(next);
  };

  const copy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    toast(t("common.copied"), "success");
  };

  const mcpCli = info.data ? `claude mcp add ${info.data.mcp.name} --scope user -- "${info.data.mcp.command}" --mcp` : "";
  const mcpJson = info.data
    ? JSON.stringify({ mcpServers: { [info.data.mcp.name]: { command: info.data.mcp.command, args: info.data.mcp.args } } }, null, 2)
    : "";

  return (
    <Page title={t("settings.title")}>
      <Card title={t("settings.language")}>
        <Segment<Lang> value={lang} onChange={setLang} options={[{ value: "tr", label: "Türkçe" }, { value: "en", label: "English" }]} />
      </Card>
      {children}
      <Card title={t("settings.mcp")} hint={t("settings.mcpHint")}>
        {info.loading && <Loading />}
        <ErrorAlert error={info.error} />
        {info.data && (
          <>
            <Field label={t("settings.mcpCli")}>
              <pre className="code">{mcpCli}</pre>
            </Field>
            <div className="row"><button type="button" className="btn btn--sm" onClick={() => copy(mcpCli)}>{t("common.copy")}</button></div>
            <Field label={t("settings.mcpJson")}>
              <pre className="code">{mcpJson}</pre>
            </Field>
            <div className="row"><button type="button" className="btn btn--sm" onClick={() => copy(mcpJson)}>{t("common.copy")}</button></div>
          </>
        )}
      </Card>
      <Card title={t("settings.data")} hint={t("settings.dataHint")}>
        {info.data && (
          <>
            <pre className="code">{`${info.data.database}\n${info.data.reportsDir}`}</pre>
            <div className="row">
              <button type="button" className="btn btn--sm" onClick={() => revealItemInDir(info.data!.database)}>{t("common.openFolder")}</button>
            </div>
          </>
        )}
      </Card>
      <Card title={t("settings.about")}>
        {info.data && <p className="secondary">{info.data.product} · {t("settings.version", { version: info.data.version })} · Seobify Hub</p>}
        <p className="secondary">{t("settings.openSource")} {t("settings.support")}</p>
        <div className="row">
          <button type="button" className="btn btn--sm" onClick={() => void openUrl(REPO_URL)}>{t("settings.sourceCode")}</button>
          <button type="button" className="btn btn--sm" onClick={() => void openUrl(supportUrl(lang, info.data?.product))}>{t("settings.supportLink")}</button>
        </div>
        <label className="checkbox">
          <input type="checkbox" checked={supportLine} disabled={prefs.loading} onChange={(e) => void setSupportLine(e.target.checked)} /> {t("settings.reportSupportLine")}
        </label>
      </Card>
    </Page>
  );
}
