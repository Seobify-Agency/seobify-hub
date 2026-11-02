// Navigation for Seobify Log. Pages are rendered in App.tsx.

/** Uppercase product word shown in the header next to the Seobify logo. */
export const TOOL_NAME = "LOG";

export type PageId = "dashboard" | "bots" | "urls" | "status" | "budget" | "spider" | "ai" | "fake" | "imports" | "projects" | "settings";

export type NavItem = { id: PageId; label: string; icon: string } | { section: string };

export const NAV: NavItem[] = [
  { id: "dashboard", label: "nav.dashboard", icon: "◈" },
  { id: "bots", label: "nav.bots", icon: "⚇" },
  { id: "urls", label: "nav.urls", icon: "⌁" },
  { id: "status", label: "nav.status", icon: "⚠" },
  { id: "budget", label: "nav.budget", icon: "◔" },
  { id: "spider", label: "nav.spider", icon: "⌘" },
  { id: "ai", label: "nav.ai", icon: "✦" },
  { id: "fake", label: "nav.fake", icon: "⊘" },
  { id: "imports", label: "nav.imports", icon: "⇪" },
  { section: "app.hub" },
  { id: "projects", label: "nav.projects", icon: "▤" },
  { id: "settings", label: "nav.settings", icon: "⚙" },
];
