// Active migration plan of the selected project, shared by every Migrate screen.
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { isTauri, migrate } from "./api";
import type { PlanSummary } from "./api";
import { useProjects } from "./context";

interface PlanState {
  plans: PlanSummary[];
  plan: PlanSummary | null;
  setPlanId: (id: number | null) => void;
  reload: () => Promise<PlanSummary[]>;
  /** Increments after any data change so screens can refresh. */
  version: number;
  bump: () => void;
  loading: boolean;
  error: string | null;
}

const PlanContext = createContext<PlanState>({
  plans: [],
  plan: null,
  setPlanId: () => {},
  reload: async () => [],
  version: 0,
  bump: () => {},
  loading: false,
  error: null,
});

const key = (projectId: number) => `seobify-migrate-plan-${projectId}`;

export function PlanProvider({ children }: { children: ReactNode }) {
  const { project } = useProjects();
  const [plans, setPlans] = useState<PlanSummary[]>([]);
  const [planId, setPlanIdState] = useState<number | null>(null);
  const [version, setVersion] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (!isTauri || !project) {
      setPlans([]);
      return [];
    }
    setLoading(true);
    try {
      const list = await migrate.listPlans(project.id);
      setPlans(list);
      setError(null);
      return list;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return [];
    } finally {
      setLoading(false);
    }
  }, [project]);

  useEffect(() => {
    if (!project) return;
    try {
      const stored = localStorage.getItem(key(project.id));
      setPlanIdState(stored ? Number(stored) : null);
    } catch {
      setPlanIdState(null);
    }
    void reload();
  }, [project, reload]);

  const setPlanId = useCallback(
    (id: number | null) => {
      setPlanIdState(id);
      if (!project) return;
      try {
        if (id === null) localStorage.removeItem(key(project.id));
        else localStorage.setItem(key(project.id), String(id));
      } catch {
        /* storage unavailable */
      }
    },
    [project],
  );

  const bump = useCallback(() => {
    setVersion((v) => v + 1);
    void reload();
  }, [reload]);

  const plan = useMemo(() => plans.find((p) => p.id === planId) ?? plans[0] ?? null, [plans, planId]);
  const value = useMemo(() => ({ plans, plan, setPlanId, reload, version, bump, loading, error }), [plans, plan, setPlanId, reload, version, bump, loading, error]);
  return <PlanContext.Provider value={value}>{children}</PlanContext.Provider>;
}

export function usePlan() {
  return useContext(PlanContext);
}
