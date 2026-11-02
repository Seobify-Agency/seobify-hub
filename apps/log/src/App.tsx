import { useEffect, useState } from "react";
import Header from "./components/Header";
import { NavContext } from "./components/LogKit";
import Sidebar from "./components/Sidebar";
import { api, isTauri } from "./lib/api";
import { NAV } from "./nav";
import type { PageId } from "./nav";
import AiBots from "./pages/AiBots";
import Bots from "./pages/Bots";
import Budget from "./pages/Budget";
import FakeBots from "./pages/FakeBots";
import Imports from "./pages/Imports";
import LogSettings from "./pages/LogSettings";
import Overview from "./pages/Overview";
import Projects from "./pages/Projects";
import Settings from "./pages/Settings";
import SpiderGaps from "./pages/SpiderGaps";
import Status from "./pages/Status";
import Urls from "./pages/Urls";
import "./components/shell.css";

const PAGE_KEY = "seobify-hub-page";
const PAGES = NAV.flatMap((n) => ("id" in n ? [n.id] : []));

export default function App() {
  const [page, setPage] = useState<PageId>(() => {
    try {
      const saved = localStorage.getItem(PAGE_KEY) as PageId | null;
      return saved && PAGES.includes(saved) ? saved : "dashboard";
    } catch {
      return "dashboard";
    }
  });
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    if (isTauri) api.appInfo().then((i) => setVersion(i.version)).catch(() => {});
  }, []);

  const navigate = (next: PageId) => {
    setPage(next);
    try {
      localStorage.setItem(PAGE_KEY, next);
    } catch {
      /* storage unavailable */
    }
  };

  return (
    <NavContext.Provider value={navigate}>
      <div className="app">
        <Header />
        <div className="app__body">
          <Sidebar page={page} onNavigate={navigate} version={version} />
          <main className="app__main">
            {page === "dashboard" && <Overview />}
            {page === "bots" && <Bots />}
            {page === "urls" && <Urls />}
            {page === "status" && <Status />}
            {page === "budget" && <Budget />}
            {page === "spider" && <SpiderGaps />}
            {page === "ai" && <AiBots />}
            {page === "fake" && <FakeBots />}
            {page === "imports" && <Imports />}
            {page === "projects" && <Projects />}
            {page === "settings" && (
              <Settings>
                <LogSettings />
              </Settings>
            )}
          </main>
        </div>
      </div>
    </NavContext.Provider>
  );
}
