import { useCallback, useState } from "react";

interface ZoomState {
  level: number;
  panX: number;
  panY: number;
}

const ZOOM_LEVELS = [0.25, 0.5, 1, 2, 4, 8];
export const ZOOM_MIN = 0.1;
export const ZOOM_MAX = 16;

const clampZoom = (z: number) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z));

export function useZoom() {
  const [state, setState] = useState<ZoomState>({
    level: 1,
    panX: 0,
    panY: 0,
  });

  const setZoom = useCallback((level: number) => {
    setState((prev) => ({ ...prev, level: clampZoom(level) }));
  }, []);

  const zoomIn = useCallback(() => {
    setState((prev) => {
      const next = ZOOM_LEVELS.find((l) => l > prev.level + 1e-6) ?? prev.level;
      return { ...prev, level: next };
    });
  }, []);

  const zoomOut = useCallback(() => {
    setState((prev) => {
      const lower = ZOOM_LEVELS.filter((l) => l < prev.level - 1e-6);
      const next = lower.length > 0 ? lower[lower.length - 1] : prev.level;
      return { ...prev, level: next };
    });
  }, []);

  /** Multiplies the zoom by `factor` keeping the point (cx, cy) of the viewport fixed. */
  const zoomAt = useCallback((factor: number, cx: number, cy: number) => {
    setState((prev) => {
      const next = clampZoom(prev.level * factor);
      const ratio = next / prev.level;
      return {
        level: next,
        panX: cx - (cx - prev.panX) * ratio,
        panY: cy - (cy - prev.panY) * ratio,
      };
    });
  }, []);

  const pan = useCallback((dx: number, dy: number) => {
    setState((prev) => ({
      ...prev,
      panX: prev.panX + dx,
      panY: prev.panY + dy,
    }));
  }, []);

  const resetPan = useCallback(() => {
    setState((prev) => ({ ...prev, panX: 0, panY: 0 }));
  }, []);

  return {
    zoom: state.level,
    panX: state.panX,
    panY: state.panY,
    setZoom,
    zoomIn,
    zoomOut,
    zoomAt,
    pan,
    resetPan,
    zoomLevels: ZOOM_LEVELS,
  };
}
