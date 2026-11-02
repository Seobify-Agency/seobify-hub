import { useState } from "react";
import logoGold from "../assets/seobify-logo-gold.svg";
import AboutDialog from "./AboutDialog";
import { useTranslation } from "../i18n";
import type { Lang } from "../i18n";
import "./Header.css";

type AppMode = "single" | "batch" | "documents";

interface HeaderProps {
  appMode: AppMode;
  onAppModeChange: (mode: AppMode) => void;
  hasContent: boolean;
  onAddImages: () => void;
  onClose: () => void;
}

export default function Header({
  appMode,
  onAppModeChange,
  hasContent,
  onAddImages,
  onClose,
}: HeaderProps) {
  const { t, lang, setLang } = useTranslation();
  const [aboutOpen, setAboutOpen] = useState(false);

  return (
    <header className="hub-header">
      <div className="hub-header__brand">
        <img src={logoGold} alt="Seobify" className="hub-header__logo" draggable={false} />
        <span className="hub-header__divider" />
        <span className="hub-header__product">
          <span className="hub-header__product-name">OPTIC</span>
          <span className="hub-header__product-sub">{t("header.tagline")}</span>
        </span>
      </div>

      <div className="hub-header__controls">
        <button
          type="button"
          className="hub-header__btn hub-header__btn--primary"
          onClick={onAddImages}
          title={appMode === "documents" ? t("header.addDocumentsHint") : t("header.addImagesHint")}
        >
          ＋ {appMode === "documents" ? t("header.addDocuments") : t("header.addImages")}
        </button>
        {hasContent && (
          <button type="button" className="hub-header__btn" onClick={onClose} title={appMode === "documents" ? t("header.closeDocsHint") : t("header.closeHint")}>
            ✕ {t("header.close")}
          </button>
        )}

        <div className="hub-header__segment" role="group" aria-label={t("header.workMode")}>
          <button
            type="button"
            className={`hub-header__segment-btn ${appMode === "single" ? "hub-header__segment-btn--active" : ""}`}
            onClick={() => onAppModeChange("single")}
          >
            {t("app.single")}
          </button>
          <button
            type="button"
            className={`hub-header__segment-btn ${appMode === "batch" ? "hub-header__segment-btn--active" : ""}`}
            onClick={() => onAppModeChange("batch")}
          >
            {t("app.batch")}
          </button>
          <button
            type="button"
            className={`hub-header__segment-btn ${appMode === "documents" ? "hub-header__segment-btn--active" : ""}`}
            onClick={() => onAppModeChange("documents")}
          >
            {t("app.documents")}
          </button>
        </div>

        <div className="hub-header__segment" role="group" aria-label={t("lang.label")}>
          {(["tr", "en"] as Lang[]).map((l) => (
            <button
              key={l}
              type="button"
              className={`hub-header__segment-btn ${lang === l ? "hub-header__segment-btn--active" : ""}`}
              onClick={() => setLang(l)}
            >
              {l.toUpperCase()}
            </button>
          ))}
        </div>
        <button type="button" className="hub-header__btn" onClick={() => setAboutOpen(true)} title={t("about.title")} aria-label={t("about.title")}>
          ⓘ
        </button>
      </div>
      <AboutDialog isOpen={aboutOpen} onClose={() => setAboutOpen(false)} />
    </header>
  );
}
