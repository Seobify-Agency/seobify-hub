import { invoke } from "@tauri-apps/api/core";
import type {
  CwebpStatus,
  EncodingConfig,
  OptimizedImage,
  ExportResult,
  SanitizeConfig,
  SavePathResult,
} from "../types";

export async function detectCwebp(): Promise<CwebpStatus> {
  return invoke<CwebpStatus>("detect_cwebp");
}

export async function encodeImage(
  filePath: string,
  config: EncodingConfig,
  sanitize?: SanitizeConfig
): Promise<OptimizedImage> {
  return invoke<OptimizedImage>("encode_image", {
    filePath,
    config,
    sanitize: sanitize ?? null,
  });
}

export async function exportImage(
  tempFilePath: string,
  outputPath: string
): Promise<ExportResult> {
  return invoke<ExportResult>("export_image", {
    tempFilePath,
    outputPath,
  });
}

export async function selectSavePath(
  defaultName: string,
  extension = "webp"
): Promise<SavePathResult | null> {
  return invoke<SavePathResult | null>("select_save_path", {
    defaultName,
    extension,
  });
}
