import { useEffect, useState } from "react";
import Header from "./components/Header";
import Sidebar from "./components/Sidebar";
import { api, isTauri } from "./lib/api";
import { PlanProvider } from "./lib/plan";
import { NAV } from "./nav";
import type { PageId } from "./nav";
import Dashboard from "./pages/Dashboard";
import Export from "./pages/Export";
import Mapping from "./pages/Mapping";
import Patterns from "./pages/Patterns";
import Plans from "./pages/Plans";
import Projects from "./pages/Projects";
import Settings from "./pages/Settings";
import Sources from "./pages/Sources";
import Verify from "./pages/Verify";
import "./components/shell.css";
import "./styles/migrate.css";

const PAGE_KEY = "seobify-hub-page";
const PAGES = new Set(NAV.flatMap((n) => ("id" in n ? [n.id] : [])));

export default function App() {
  const [page, setPage] = useState<PageId>(() => {
    try {
      const stored = localStorage.getItem(PAGE_KEY) as PageId | null;
      return stored && PAGES.has(stored) ? stored : "dashboard";
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
    <PlanProvider>
      <div className="app">
        <Header />
        <div className="app__body">
          <Sidebar page={page} onNavigate={navigate} version={version} />
          <main className="app__main">
            {page === "dashboard" && <Dashboard onNavigate={navigate} />}
            {page === "plans" && <Plans onNavigate={navigate} />}
            {page === "sources" && <Sources onNavigate={navigate} />}
            {page === "mapping" && <Mapping onNavigate={navigate} />}
            {page === "patterns" && <Patterns onNavigate={navigate} />}
            {page === "export" && <Export onNavigate={navigate} />}
            {page === "verify" && <Verify onNavigate={navigate} />}
            {page === "projects" && <Projects />}
            {page === "settings" && <Settings />}
          </main>
        </div>
      </div>
    </PlanProvider>
  );
}
