import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "../i18n";
import "./SeoDialog.css";

const REPO_URL = "https://github.com/Seobify-Agency/seobify-hub";

/** About: version, license, source code and the expert-support link (source tag only). */
export default function AboutDialog({ isOpen, onClose }: { isOpen: boolean; onClose: () => void }) {
  const { t, lang } = useTranslation();
  const [version, setVersion] = useState("");
  useEffect(() => {
    if (isOpen) getVersion().then(setVersion).catch(() => setVersion(""));
  }, [isOpen]);
  if (!isOpen) return null;
  const support = `${lang === "en" ? "https://seobify.com/en/contact/" : "https://seobify.com/iletisim/"}?utm_source=seobify-optic&utm_medium=app&utm_campaign=seobify-hub`;
  return (
    <div className="seo-dialog-backdrop" onClick={onClose}>
      <div className="seo-dialog" style={{ maxWidth: 480 }} onClick={(e) => e.stopPropagation()}>
        <div className="seo-dialog__header">
          <div className="seo-dialog__title-box">
            <h3 className="seo-dialog__title">{t("about.title")}</h3>
          </div>
          <button className="seo-dialog__close" onClick={onClose} aria-label={t("header.close")}>
            ✕
          </button>
        </div>
        <p>Seobify Optic{version ? ` v${version}` : ""} · Seobify Hub</p>
        <p>
          {t("about.openSource")} {t("about.support")}
        </p>
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <button type="button" className="hub-header__btn" onClick={() => void openUrl(REPO_URL)}>
            {t("about.sourceCode")}
          </button>
          <button type="button" className="hub-header__btn" onClick={() => void openUrl(support)}>
            {t("about.supportLink")}
          </button>
        </div>
      </div>
    </div>
  );
}
