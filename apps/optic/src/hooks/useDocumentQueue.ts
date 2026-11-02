import { useCallback, useMemo, useRef, useState } from "react";
import type { DocCleanOptions, DocItem } from "../types";
import { cleanDocument, cleanFileName, inspectDocument, isSupportedDocumentPath } from "../services/documentService";

let nextId = 0;

function normDir(path: string): string {
  return path.replace(/[\\/]+$/, "").replace(/\//g, "\\").toLowerCase();
}

/**
 * Output inside `outputDir`: the original name, or `<stem>.clean.<ext>` when the folder is
 * the source folder (the backend applies the same rule); names repeated within one run get
 * `-2`, `-3` so documents with equal names from different folders do not overwrite each other.
 */
export function batchTarget(filePath: string, fileName: string, outputDir: string, used: Set<string>): { outputDir?: string; outputPath?: string } {
  const sourceDir = filePath.slice(0, Math.max(filePath.lastIndexOf("\\"), filePath.lastIndexOf("/")));
  const name = normDir(sourceDir) === normDir(outputDir) ? cleanFileName(fileName) : fileName;
  const dot = name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  let candidate = name;
  for (let n = 2; used.has(candidate.toLowerCase()); n += 1) candidate = `${stem}-${n}${ext}`;
  used.add(candidate.toLowerCase());
  if (candidate === name) return { outputDir };
  const sep = outputDir.includes("\\") ? "\\" : "/";
  return { outputPath: `${outputDir.replace(/[\\/]+$/, "")}${sep}${candidate}` };
}

/** Documents opened in the Documents mode: inspection on add, clean one or all. */
export function useDocumentQueue() {
  const [items, setItems] = useState<DocItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isCleaningAll, setIsCleaningAll] = useState(false);
  const itemsRef = useRef<DocItem[]>([]);
  itemsRef.current = items;

  const patch = useCallback((id: string, partial: Partial<DocItem>) => {
    setItems((prev) => prev.map((item) => (item.id === id ? { ...item, ...partial } : item)));
  }, []);

  /** Adds supported paths (duplicates ignored) and inspects them; returns how many were skipped. */
  const addFiles = useCallback(
    (paths: string[]): number => {
      const known = new Set(itemsRef.current.map((i) => i.filePath.toLowerCase()));
      const fresh: DocItem[] = [];
      for (const filePath of paths) {
        if (!isSupportedDocumentPath(filePath) || known.has(filePath.toLowerCase())) continue;
        known.add(filePath.toLowerCase());
        fresh.push({
          id: `doc-${++nextId}`,
          filePath,
          fileName: filePath.split(/[\\/]/).pop() ?? filePath,
          status: "inspecting",
          report: null,
          outcome: null,
          error: null,
        });
      }
      if (fresh.length > 0) {
        setItems((prev) => [...prev, ...fresh]);
        setSelectedId((current) => current ?? fresh[0].id);
        for (const item of fresh) {
          inspectDocument(item.filePath)
            .then((report) => patch(item.id, { status: "ready", report }))
            .catch((err) => patch(item.id, { status: "failed", error: String(err) }));
        }
      }
      return paths.filter((p) => !isSupportedDocumentPath(p)).length;
    },
    [patch]
  );

  const removeItem = useCallback((id: string) => {
    setItems((prev) => prev.filter((i) => i.id !== id));
    setSelectedId((current) => {
      if (current !== id) return current;
      const rest = itemsRef.current.filter((i) => i.id !== id);
      return rest[0]?.id ?? null;
    });
  }, []);

  const clear = useCallback(() => {
    setItems([]);
    setSelectedId(null);
  }, []);

  const cleanOne = useCallback(
    async (id: string, options: DocCleanOptions, outputPath?: string) => {
      const item = itemsRef.current.find((i) => i.id === id);
      if (!item) return;
      patch(id, { status: "cleaning", error: null });
      try {
        const outcome = await cleanDocument(item.filePath, options, { outputPath });
        patch(id, { status: "done", outcome, report: outcome.before });
      } catch (err) {
        patch(id, { status: "failed", error: String(err) });
      }
    },
    [patch]
  );

  /** Cleans every inspected document into `outputDir` (one after another). */
  const cleanAll = useCallback(
    async (options: DocCleanOptions, outputDir: string) => {
      setIsCleaningAll(true);
      const used = new Set<string>();
      try {
        for (const item of itemsRef.current) {
          if (item.status === "inspecting" || (item.status === "failed" && !item.report)) continue;
          patch(item.id, { status: "cleaning", error: null });
          try {
            const target = batchTarget(item.filePath, item.fileName, outputDir, used);
            const outcome = await cleanDocument(item.filePath, options, target);
            patch(item.id, { status: "done", outcome, report: outcome.before });
          } catch (err) {
            patch(item.id, { status: "failed", error: String(err) });
          }
        }
      } finally {
        setIsCleaningAll(false);
      }
    },
    [patch]
  );

  const selected = useMemo(() => items.find((i) => i.id === selectedId) ?? null, [items, selectedId]);

  return { items, selected, selectedId, setSelectedId, addFiles, removeItem, clear, cleanOne, cleanAll, isCleaningAll };
}

export type DocumentQueue = ReturnType<typeof useDocumentQueue>;
