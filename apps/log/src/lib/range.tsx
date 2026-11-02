// Shared date range (per app session) plus the active project's data bounds.
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { isTauri } from "./api";
import { useProjects } from "./context";
import { logApi, presetRange } from "./logapi";
import type { Bounds, RangeArgs } from "./logapi";

interface RangeState {
  preset: string;
  range: RangeArgs;
  bounds: Bounds | null;
  boundsError: string | null;
  /** Bumped after imports/deletes so pages reload. */
  version: number;
  setPreset: (preset: string) => void;
  setCustom: (from: string | null, to: string | null) => void;
  refresh: () => void;
}

const RangeContext = createContext<RangeState>({
  preset: "all",
  range: {},
  bounds: null,
  boundsError: null,
  version: 0,
  setPreset: () => {},
  setCustom: () => {},
  refresh: () => {},
});

export function RangeProvider({ children }: { children: ReactNode }) {
  const { project } = useProjects();
  const [preset, setPresetState] = useState("all");
  const [custom, setCustomState] = useState<RangeArgs>({});
  const [bounds, setBounds] = useState<Bounds | null>(null);
  const [boundsError, setBoundsError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    if (!isTauri || !project) {
      setBounds(null);
      return;
    }
    let alive = true;
    logApi
      .bounds(project.id)
      .then((b) => alive && (setBounds(b), setBoundsError(null)))
      .catch((e) => alive && setBoundsError(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
    };
  }, [project, version]);

  const range = useMemo<RangeArgs>(() => {
    if (preset === "custom") return custom;
    return bounds ? presetRange(preset, bounds) : {};
  }, [preset, custom, bounds]);

  const setPreset = useCallback((p: string) => setPresetState(p), []);
  const setCustom = useCallback((from: string | null, to: string | null) => {
    setCustomState({ from, to });
    setPresetState("custom");
  }, []);
  const refresh = useCallback(() => setVersion((v) => v + 1), []);
  const value = useMemo(() => ({ preset, range, bounds, boundsError, version, setPreset, setCustom, refresh }), [preset, range, bounds, boundsError, version, setPreset, setCustom, refresh]);
  return <RangeContext.Provider value={value}>{children}</RangeContext.Provider>;
}

export function useRange() {
  return useContext(RangeContext);
}
