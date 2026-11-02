import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { SourceImage } from "../types";
import { SUPPORTED_EXTENSIONS } from "../types";

/** Opens the native file picker and returns the chosen image paths (empty when cancelled). */
export async function pickImageFiles(): Promise<string[]> {
  const selected = await open({
    multiple: true,
    filters: [{ name: "Images", extensions: [...SUPPORTED_EXTENSIONS] }],
  });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export async function loadImage(filePath: string): Promise<SourceImage> {
  return invoke<SourceImage>("load_image", { filePath });
}
