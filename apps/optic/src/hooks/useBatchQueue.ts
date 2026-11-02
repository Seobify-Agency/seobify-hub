import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { loadImage } from "../services/imageService";
import type {
  BatchQueueItem,
  BatchItemStatus,
  EncodingConfig,
  SanitizeConfig,
  SourceImage,
  BatchProgressPayload,
  OutputDirResult,
} from "../types";
import { SUPPORTED_EXTENSIONS } from "../types";

export function isSupportedImagePath(path: string): boolean {
  const lower = path.toLowerCase();
  return SUPPORTED_EXTENSIONS.some((ext) => lower.endsWith(`.${ext}`));
}

const LOAD_CONCURRENCY = 3;

function baseName(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(idx + 1) : path;
}

let nextItemId = 0;

export function useBatchQueue() {
  const [queue, setQueue] = useState<BatchQueueItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isExporting, setIsExporting] = useState(false);

  /**
   * Adds files to the queue immediately as "loading" rows, then reads them with a small
   * concurrency pool so the first preview appears right away instead of after every
   * file has been transferred.
   */
  const addFiles = useCallback(async (filePaths: string[]) => {
    const placeholders: BatchQueueItem[] = filePaths
      .filter(isSupportedImagePath)
      .map((filePath) => ({
        id: `batch-${Date.now()}-${nextItemId++}`,
        filePath,
        fileName: baseName(filePath),
        sourceImage: null,
        position: 0,
        status: "loading" as const,
        optimizedImage: null,
        outputPath: null,
        error: null,
      }));
    if (placeholders.length === 0) return;

    setQueue((prev) =>
      [...prev, ...placeholders].map((item, i) => ({ ...item, position: i }))
    );
    setSelectedId((current) => current ?? placeholders[0].id);

    let cursor = 0;
    const worker = async () => {
      while (cursor < placeholders.length) {
        const item = placeholders[cursor++];
        try {
          const image = await loadImage(item.filePath);
          setQueue((prev) =>
            prev.map((it) =>
              it.id === item.id ? { ...it, sourceImage: image, status: "pending" } : it
            )
          );
        } catch (err) {
          setQueue((prev) =>
            prev.map((it) =>
              it.id === item.id ? { ...it, status: "failed", error: String(err) } : it
            )
          );
        }
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(LOAD_CONCURRENCY, placeholders.length) }, worker)
    );
  }, []);

  const clearQueue = useCallback(() => {
    setQueue([]);
    setSelectedId(null);
  }, []);

  const removeItem = useCallback(
    (id: string) => {
      setQueue((prev) => {
        const next = prev.filter((item) => item.id !== id);
        return next.map((item, i) => ({ ...item, position: i }));
      });
      if (selectedId === id) {
        setSelectedId(null);
      }
    },
    [selectedId]
  );

  const moveItem = useCallback((id: string, direction: "up" | "down") => {
    setQueue((prev) => {
      const idx = prev.findIndex((item) => item.id === id);
      if (idx < 0) return prev;
      const targetIdx = direction === "up" ? idx - 1 : idx + 1;
      if (targetIdx < 0 || targetIdx >= prev.length) return prev;
      const next = [...prev];
      [next[idx], next[targetIdx]] = [next[targetIdx], next[idx]];
      return next.map((item, i) => ({ ...item, position: i }));
    });
  }, []);

  const updateItemStatus = useCallback(
    (id: string, status: BatchItemStatus, error?: string) => {
      setQueue((prev) =>
        prev.map((item) =>
          item.id === id ? { ...item, status, error: error ?? null } : item
        )
      );
    },
    []
  );

  const selectedImage: SourceImage | null =
    queue.find((item) => item.id === selectedId)?.sourceImage ?? null;

  const exportAll = useCallback(
    async (config: EncodingConfig, sanitize?: SanitizeConfig) => {
      // Select output directory
      const dirResult = await invoke<OutputDirResult | null>(
        "select_output_directory"
      );
      if (!dirResult?.path) return;

      setIsExporting(true);

      // Mark all loaded items as processing
      setQueue((prev) =>
        prev.map((item) =>
          item.status === "pending" ? { ...item, status: "processing" } : item
        )
      );

      // Listen for progress events
      const unlisten = await listen<BatchProgressPayload>(
        "batch_progress",
        (event) => {
          const payload = event.payload;
          setQueue((prev) =>
            prev.map((item) =>
              item.id === payload.itemId
                ? {
                    ...item,
                    status: payload.status as BatchItemStatus,
                    outputPath: payload.outputPath ?? null,
                    error: payload.error ?? null,
                  }
                : item
            )
          );
        }
      );

      try {
        const items = queue
          .filter((item) => item.status !== "completed" && item.sourceImage !== null)
          .map((item) => ({
            id: item.id,
            filePath: item.filePath,
          }));

        await invoke("export_batch", {
          items,
          config,
          outputDir: dirResult.path,
          sanitize: sanitize ?? null,
        });
      } finally {
        unlisten();
        setIsExporting(false);
      }
    },
    [queue]
  );

  return {
    queue,
    selectedId,
    selectedImage,
    isExporting,
    setSelectedId,
    addFiles,
    removeItem,
    moveItem,
    updateItemStatus,
    exportAll,
    clearQueue,
  };
}
