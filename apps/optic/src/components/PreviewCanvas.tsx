import { useEffect, useRef } from "react";
import type { BackgroundMode } from "../types";
import "./PreviewCanvas.css";

interface PreviewCanvasProps {
  imageData: string | null;
  mimeType: string;
  backgroundMode: BackgroundMode;
  label?: string;
  isLoading?: boolean;
}

export default function PreviewCanvas({
  imageData,
  mimeType,
  backgroundMode,
  label,
  isLoading,
}: PreviewCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !imageData) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const img = new Image();
    img.onload = () => {
      canvas.width = img.width;
      canvas.height = img.height;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(img, 0, 0);
    };
    img.src = `data:${mimeType};base64,${imageData}`;
  }, [imageData, mimeType]);

  const bgClass =
    backgroundMode === "checkerboard"
      ? "bg-checkerboard"
      : backgroundMode === "dark"
        ? "bg-dark-matte"
        : "bg-light-matte";

  return (
    <div ref={containerRef} className={`preview-canvas ${bgClass}`}>
      {label && <div className="preview-canvas__label">{label}</div>}
      {isLoading && (
        <div className="preview-canvas__loading">
          <div className="spinner" />
        </div>
      )}
      {imageData ? (
        <canvas ref={canvasRef} className="preview-canvas__canvas" />
      ) : (
        !isLoading && (
          <div className="preview-canvas__empty">No image</div>
        )
      )}
    </div>
  );
}
