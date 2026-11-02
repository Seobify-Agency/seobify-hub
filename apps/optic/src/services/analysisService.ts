import { encodeImage } from "./encodeService";
import type { BandingRisk, EncodingConfig, SourceImage, SsimResult } from "../types";

let workerInstance: Worker | null = null;

function getWorker(): Worker {
  if (!workerInstance) {
    workerInstance = new Worker(
      new URL("../workers/inspectionWorker.ts", import.meta.url),
      { type: "module" }
    );
  }
  return workerInstance;
}

export function loadImageData(
  base64: string,
  mimeType: string,
  width: number,
  height: number
): Promise<ImageData> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext("2d");
      if (!ctx) {
        reject(new Error("Canvas context not available"));
        return;
      }
      ctx.drawImage(img, 0, 0, width, height);
      resolve(ctx.getImageData(0, 0, width, height));
    };
    img.onerror = () => reject(new Error("Failed to load image for analysis"));
    img.src = `data:${mimeType};base64,${base64}`;
  });
}

/**
 * Runs SSIM and Banding analysis asynchronously via Web Worker
 */
export async function runImageAnalysis(
  originalBase64: string,
  optimizedBase64: string,
  width: number,
  height: number,
  originalMimeType = "image/png",
  optimizedMimeType = "image/webp"
): Promise<{ ssim: SsimResult; banding: BandingRisk }> {
  const [origData, optData] = await Promise.all([
    loadImageData(originalBase64, originalMimeType, width, height),
    loadImageData(optimizedBase64, optimizedMimeType, width, height),
  ]);

  return new Promise((resolve) => {
    const worker = getWorker();

    const handleMessage = (e: MessageEvent) => {
      if (e.data.type === "full-analysis") {
        worker.removeEventListener("message", handleMessage);
        resolve({
          ssim: e.data.ssim,
          banding: e.data.risk,
        });
      }
    };

    worker.addEventListener("message", handleMessage);
    worker.postMessage({
      type: "full-analysis",
      originalData: origData,
      optimizedData: optData,
      width,
      height,
    });
  });
}

/**
 * Auto-calibrates quality using binary search to find the lowest -q reaching target SSIM
 */
export async function autoCalibrateQuality(
  sourceImage: SourceImage,
  baseConfig: EncodingConfig,
  targetSsim = 0.992,
  onStep?: (testedQuality: number, ssimPercent: number) => void
): Promise<{ optimalQuality: number; finalSsim: SsimResult }> {
  let low = 50;
  let high = 98;
  let bestQuality = baseConfig.quality;
  let bestSsim: SsimResult = { ssim: 1, ssimPercent: 100, rating: "imperceptible" };

  // Load original image data once
  const origData = await loadImageData(
    sourceImage.imageData,
    sourceImage.mimeType,
    sourceImage.width,
    sourceImage.height
  );

  // Maximum 4 binary search steps for rapid calibration (< 600ms total)
  for (let step = 0; step < 4; step++) {
    const mid = Math.round((low + high) / 2);
    const testConfig: EncodingConfig = {
      ...baseConfig,
      quality: mid,
      lossless: false,
    };

    const optResult = await encodeImage(sourceImage.filePath, testConfig);
    const optData = await loadImageData(
      optResult.imageData,
      optResult.mimeType,
      sourceImage.width,
      sourceImage.height
    );

    // Synchronous SSIM run in worker or direct
    const ssimRes: SsimResult = await new Promise((resolve) => {
      const worker = getWorker();
      const handler = (e: MessageEvent) => {
        if (e.data.type === "ssim") {
          worker.removeEventListener("message", handler);
          resolve({
            ssim: e.data.ssim,
            ssimPercent: e.data.ssimPercent,
            rating: e.data.rating,
          });
        }
      };
      worker.addEventListener("message", handler);
      worker.postMessage({
        type: "ssim",
        originalData: origData,
        optimizedData: optData,
        width: sourceImage.width,
        height: sourceImage.height,
      });
    });

    onStep?.(mid, ssimRes.ssimPercent);

    if (ssimRes.ssim >= targetSsim) {
      bestQuality = mid;
      bestSsim = ssimRes;
      high = mid - 1; // Try lower quality to save more bytes
    } else {
      low = mid + 1; // Needs higher quality to meet threshold
      if (bestQuality < mid) {
        bestQuality = mid;
        bestSsim = ssimRes;
      }
    }
  }

  return { optimalQuality: bestQuality, finalSsim: bestSsim };
}
