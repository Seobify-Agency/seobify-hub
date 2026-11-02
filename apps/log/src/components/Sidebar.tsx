import { useI18n } from "../i18n";
import { NAV } from "../nav";
import type { PageId } from "../nav";

interface Props {
  page: PageId;
  onNavigate: (page: PageId) => void;
  version?: string;
}

export default function Sidebar({ page, onNavigate, version }: Props) {
  const { t } = useI18n();
  return (
    <nav className="sidebar" aria-label="Navigation">
      {NAV.map((item) =>
        "section" in item ? (
          <div key={item.section} className="sidebar__section">
            {t(item.section)}
          </div>
        ) : (
          <button
            key={item.id}
            type="button"
            className={`sidebar__item ${page === item.id ? "sidebar__item--active" : ""}`}
            onClick={() => onNavigate(item.id)}
          >
            <span className="sidebar__icon" aria-hidden>
              {item.icon}
            </span>
            {t(item.label)}
          </button>
        ),
      )}
      <div className="sidebar__foot">
        {t("app.hub")}
        {version ? ` · v${version}` : ""}
      </div>
    </nav>
  );
}
