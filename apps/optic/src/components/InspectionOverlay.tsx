import { useEffect, useRef, useState, useCallback } from "react";
import type { InspectionMode } from "../types";
import "./InspectionOverlay.css";

interface InspectionOverlayProps {
  originalImageData: string | null;
  optimizedImageData: string | null;
  mode: InspectionMode;
  width: number;
  height: number;
  zoom: number;
  panX: number;
  panY: number;
  hasAlpha?: boolean;
}

export default function InspectionOverlay({
  originalImageData,
  optimizedImageData,
  mode,
  width,
  height,
  zoom,
  panX,
  panY,
  hasAlpha,
}: InspectionOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const loupeRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const workerRef = useRef<Worker | null>(null);
  const [mousePos, setMousePos] = useState<{ x: number; y: number } | null>(
    null
  );

  // Initialize worker
  useEffect(() => {
    workerRef.current = new Worker(
      new URL("../workers/inspectionWorker.ts", import.meta.url),
      { type: "module" }
    );

    return () => {
      workerRef.current?.terminate();
    };
  }, []);

  // Compute overlay when mode/data changes
  useEffect(() => {
    if (mode === "none" || mode === "magnifier") return;
    if (!originalImageData || !canvasRef.current || !workerRef.current) return;

    const canvas = canvasRef.current;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    canvas.width = width;
    canvas.height = height;

    const loadImageData = (
      base64: string,
      mimeType: string
    ): Promise<ImageData> => {
      return new Promise((resolve) => {
        const img = new Image();
        img.onload = () => {
          const offscreen = document.createElement("canvas");
          offscreen.width = width;
          offscreen.height = height;
          const offCtx = offscreen.getContext("2d")!;
          offCtx.drawImage(img, 0, 0, width, height);
          resolve(offCtx.getImageData(0, 0, width, height));
        };
        img.src = `data:${mimeType};base64,${base64}`;
      });
    };

    const compute = async () => {
      const origData = await loadImageData(originalImageData, "image/png");

      if (mode === "difference" && optimizedImageData) {
        const optData = await loadImageData(optimizedImageData, "image/webp");

        workerRef.current!.onmessage = (e) => {
          ctx.putImageData(e.data.result, 0, 0);
        };

        workerRef.current!.postMessage({
          type: "difference",
          originalData: origData,
          optimizedData: optData,
          width,
          height,
        });
      } else if (mode === "alpha-edge") {
        workerRef.current!.onmessage = (e) => {
          ctx.putImageData(e.data.result, 0, 0);
        };

        workerRef.current!.postMessage({
          type: "alpha-edge",
          originalData: origData,
          optimizedData: origData,
          width,
          height,
        });
      } else if (mode === "high-contrast") {
        workerRef.current!.onmessage = (e) => {
          ctx.putImageData(e.data.result, 0, 0);
        };

        workerRef.current!.postMessage({
          type: "high-contrast",
          originalData: origData,
          optimizedData: origData,
          width,
          height,
        });
      }
    };

    compute();
  }, [mode, originalImageData, optimizedImageData, width, height]);

  // Magnifier
  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (mode !== "magnifier" || !originalImageData) return;
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      setMousePos({ x: e.clientX - rect.left, y: e.clientY - rect.top });
    },
    [mode, originalImageData]
  );

  const handleMouseLeave = useCallback(() => {
    setMousePos(null);
  }, []);

  useEffect(() => {
    if (mode !== "magnifier" || !mousePos || !originalImageData) return;

    const loupeCanvas = loupeRef.current;
    if (!loupeCanvas) return;
    const ctx = loupeCanvas.getContext("2d");
    if (!ctx) return;

    const loupeSize = 160;
    const magnification = 16;
    loupeCanvas.width = loupeSize;
    loupeCanvas.height = loupeSize;

    const img = new Image();
    img.onload = () => {
      // Calculate what part of the image to magnify
      const imgX = (mousePos.x - panX) / zoom;
      const imgY = (mousePos.y - panY) / zoom;
      const srcSize = loupeSize / magnification;

      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, loupeSize, loupeSize);
      ctx.drawImage(
        img,
        imgX - srcSize / 2,
        imgY - srcSize / 2,
        srcSize,
        srcSize,
        0,
        0,
        loupeSize,
        loupeSize
      );

      // Draw grid
      ctx.strokeStyle = "rgba(255,255,255,0.15)";
      ctx.lineWidth = 0.5;
      const cellSize = loupeSize / (srcSize);
      for (let i = 0; i <= srcSize; i++) {
        ctx.beginPath();
        ctx.moveTo(i * cellSize, 0);
        ctx.lineTo(i * cellSize, loupeSize);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(0, i * cellSize);
        ctx.lineTo(loupeSize, i * cellSize);
        ctx.stroke();
      }
    };
    img.src = `data:image/png;base64,${originalImageData}`;
  }, [mode, mousePos, originalImageData, zoom, panX, panY]);

  if (mode === "none") return null;

  return (
    <div
      ref={containerRef}
      className="inspection-overlay"
      onMouseMove={handleMouseMove}
      onMouseLeave={handleMouseLeave}
    >
      {mode === "alpha-edge" && hasAlpha === false && (
        <div className="inspection-overlay__info">
          This image has no alpha channel — alpha edge detection has no effect on opaque PNGs.
        </div>
      )}
      {mode !== "magnifier" && (
        <canvas
          ref={canvasRef}
          className="inspection-overlay__canvas"
          style={{
            transform: `translate(${panX}px, ${panY}px) scale(${zoom})`,
            transformOrigin: "0 0",
          }}
        />
      )}
      {mode === "magnifier" && mousePos && (
        <canvas
          ref={loupeRef}
          className="inspection-overlay__loupe"
          style={{
            left: mousePos.x - 80,
            top: mousePos.y - 80,
          }}
        />
      )}
    </div>
  );
}
