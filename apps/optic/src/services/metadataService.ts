import { invoke } from "@tauri-apps/api/core";
import type { InspectReport } from "../types";

export async function inspectMetadata(filePath: string): Promise<InspectReport> {
  return invoke<InspectReport>("inspect_metadata", { filePath });
}
