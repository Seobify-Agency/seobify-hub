// Navigation for this tool. Pages are rendered in App.tsx.

/** Uppercase product word shown in the header next to the Seobify logo. */
export const TOOL_NAME = "MIGRATE";

export type PageId = "dashboard" | "plans" | "sources" | "mapping" | "patterns" | "export" | "verify" | "projects" | "settings";

export type NavItem = { id: PageId; label: string; icon: string } | { section: string };

export const NAV: NavItem[] = [
  { id: "dashboard", label: "nav.dashboard", icon: "◈" },
  { section: "nav.migration" },
  { id: "plans", label: "nav.plans", icon: "▦" },
  { id: "sources", label: "nav.sources", icon: "⇅" },
  { id: "mapping", label: "nav.mapping", icon: "⇄" },
  { id: "patterns", label: "nav.patterns", icon: "✱" },
  { id: "export", label: "nav.export", icon: "⤓" },
  { id: "verify", label: "nav.verify", icon: "✓" },
  { section: "app.hub" },
  { id: "projects", label: "nav.projects", icon: "▤" },
  { id: "settings", label: "nav.settings", icon: "⚙" },
];

export type Navigate = (page: PageId) => void;
