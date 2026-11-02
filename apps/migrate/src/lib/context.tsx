// App-wide state: active project and toasts.
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { api, isTauri } from "./api";
import type { Project } from "./api";

// ---------------------------------------------------------------- projects

interface ProjectState {
  projects: Project[];
  project: Project | null;
  setProjectId: (id: number | null) => void;
  reload: () => Promise<Project[]>;
  loading: boolean;
  error: string | null;
}

const ProjectContext = createContext<ProjectState>({
  projects: [],
  project: null,
  setProjectId: () => {},
  reload: async () => [],
  loading: false,
  error: null,
});

const PROJECT_KEY = "seobify-hub-project";

export function ProjectProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectIdState] = useState<number | null>(() => {
    try {
      const v = localStorage.getItem(PROJECT_KEY);
      return v ? Number(v) : null;
    } catch {
      return null;
    }
  });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (!isTauri) {
      setLoading(false);
      return [];
    }
    setLoading(true);
    try {
      const list = await api.listProjects();
      setProjects(list);
      setError(null);
      return list;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return [];
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const setProjectId = useCallback((id: number | null) => {
    setProjectIdState(id);
    try {
      if (id === null) localStorage.removeItem(PROJECT_KEY);
      else localStorage.setItem(PROJECT_KEY, String(id));
    } catch {
      /* storage unavailable */
    }
  }, []);

  const project = useMemo(() => projects.find((p) => p.id === projectId) ?? projects[0] ?? null, [projects, projectId]);
  const value = useMemo(() => ({ projects, project, setProjectId, reload, loading, error }), [projects, project, setProjectId, reload, loading, error]);
  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProjects() {
  return useContext(ProjectContext);
}

// ---------------------------------------------------------------- toasts

type ToastKind = "info" | "success" | "error";
interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

const ToastContext = createContext<(text: string, kind?: ToastKind) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const push = useCallback((text: string, kind: ToastKind = "info") => {
    const id = Date.now() + Math.random();
    setToasts((list) => [...list.slice(-3), { id, kind, text }]);
    window.setTimeout(() => setToasts((list) => list.filter((t) => t.id !== id)), kind === "error" ? 9000 : 4500);
  }, []);
  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast--${t.kind}`} onClick={() => setToasts((l) => l.filter((x) => x.id !== t.id))}>
            {t.text}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  return useContext(ToastContext);
}

// ---------------------------------------------------------------- async helper

/** Runs an async loader on mount / when deps change; exposes data, error, loading, reload. */
export function useAsync<T>(loader: () => Promise<T>, deps: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let alive = true;
    setLoading(true);
    loader()
      .then((d) => alive && (setData(d), setError(null)))
      .catch((e) => alive && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);
  const reload = useCallback(() => setTick((n) => n + 1), []);
  return { data, error, loading, reload, setData };
}
