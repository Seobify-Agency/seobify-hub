import { useState } from "react";
import type { InspectionMode } from "../types";

export function useInspection() {
  const [mode, setMode] = useState<InspectionMode>("none");

  return { inspectionMode: mode, setInspectionMode: setMode };
}
