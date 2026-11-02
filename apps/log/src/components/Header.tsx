import logoGold from "../assets/seobify-logo-gold.svg";
import { useI18n } from "../i18n";
import type { Lang } from "../i18n";
import { useProjects } from "../lib/context";
import { TOOL_NAME } from "../nav";

export default function Header() {
  const { t, lang, setLang } = useI18n();
  const { projects, project, setProjectId } = useProjects();

  return (
    <header className="hub-header">
      <div className="hub-header__brand">
        <img src={logoGold} alt="Seobify" className="hub-header__logo" draggable={false} />
        <span className="hub-header__divider" />
        <span className="hub-header__product">
          <span className="hub-header__product-name">{TOOL_NAME}</span>
          <span className="hub-header__product-sub">{t("app.tagline")}</span>
        </span>
      </div>
      <div className="hub-header__controls">
        {projects.length > 0 && (
          <label className="hub-header__project" title={t("project.label")}>
            <span className="muted">{t("project.label")}</span>
            <select className="select" value={project?.id ?? ""} onChange={(e) => setProjectId(Number(e.target.value))}>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                  {p.domain ? ` · ${p.domain}` : ""}
                </option>
              ))}
            </select>
          </label>
        )}
        <div className="segment" role="group" aria-label={t("settings.language")}>
          {(["tr", "en"] as Lang[]).map((l) => (
            <button key={l} type="button" className={`segment__btn ${lang === l ? "segment__btn--active" : ""}`} onClick={() => setLang(l)}>
              {l.toUpperCase()}
            </button>
          ))}
        </div>
      </div>
    </header>
  );
}
