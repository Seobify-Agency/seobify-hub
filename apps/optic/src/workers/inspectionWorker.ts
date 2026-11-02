// Inspection Web Worker for non-blocking computation
// Handles: difference view, alpha edge analysis, Sobel high-contrast edge detection,
// SSIM calculation, and Glow/Shadow Banding & Halo detection.

export interface BandingRisk {
  detected: boolean;
  severity: "none" | "low" | "medium" | "high";
  message: string;
  recommendation?: {
    quality?: number;
    alphaQuality?: number;
    method?: number;
  };
}

export interface SsimResult {
  ssim: number;
  ssimPercent: number;
  rating: "imperceptible" | "excellent" | "good" | "fair" | "poor";
}

interface WorkerMessage {
  type:
    | "difference"
    | "alpha-edge"
    | "high-contrast"
    | "ssim"
    | "banding"
    | "full-analysis";
  originalData: ImageData;
  optimizedData?: ImageData;
  width: number;
  height: number;
}

if (typeof self !== "undefined") {
  self.onmessage = (e: MessageEvent<WorkerMessage>) => {
    const { type, originalData, optimizedData, width, height } = e.data;

    if (type === "ssim") {
      if (!optimizedData) return;
      const ssim = computeSSIM(originalData, optimizedData, width, height);
      self.postMessage({ type: "ssim", ...ssim });
      return;
    }

    if (type === "banding") {
      if (!optimizedData) return;
      const risk = detectBandingAndHalo(originalData, optimizedData, width, height);
      self.postMessage({ type: "banding", risk });
      return;
    }

    if (type === "full-analysis") {
      if (!optimizedData) return;
      const ssim = computeSSIM(originalData, optimizedData, width, height);
      const risk = detectBandingAndHalo(originalData, optimizedData, width, height);
      self.postMessage({ type: "full-analysis", ssim, risk });
      return;
    }

    let result: ImageData;

    switch (type) {
      case "difference":
        if (!optimizedData) return;
        result = computeDifference(originalData, optimizedData, width, height);
        break;
      case "alpha-edge":
        result = computeAlphaEdges(originalData, width, height);
        break;
      case "high-contrast":
        result = computeSobelEdges(originalData, width, height);
        break;
      default:
        return;
    }

    self.postMessage({ type, result }, { transfer: [result.data.buffer] });
  };
}


// ---------------------------------------------------------------------------
// Difference View
// ---------------------------------------------------------------------------
function computeDifference(
  orig: ImageData,
  opt: ImageData,
  width: number,
  height: number
): ImageData {
  const output = new ImageData(width, height);
  const d = output.data;
  const threshold = 3;

  for (let i = 0; i < d.length; i += 4) {
    const dr = Math.abs(orig.data[i] - opt.data[i]);
    const dg = Math.abs(orig.data[i + 1] - opt.data[i + 1]);
    const db = Math.abs(orig.data[i + 2] - opt.data[i + 2]);
    const da = Math.abs(orig.data[i + 3] - opt.data[i + 3]);
    const diff = Math.max(dr, dg, db, da);

    if (diff > threshold) {
      const intensity = Math.min(255, diff * 4);
      d[i] = intensity; // R
      d[i + 1] = 0; // G
      d[i + 2] = 0; // B
      d[i + 3] = 200; // A
    } else {
      d[i] = 0;
      d[i + 1] = 0;
      d[i + 2] = 0;
      d[i + 3] = 0;
    }
  }

  return output;
}

// ---------------------------------------------------------------------------
// Alpha Edges
// ---------------------------------------------------------------------------
function computeAlphaEdges(
  data: ImageData,
  width: number,
  height: number
): ImageData {
  const output = new ImageData(width, height);
  const d = output.data;
  const src = data.data;

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      const alpha = src[i + 3];

      if (alpha > 0 && alpha < 255) {
        d[i] = 0; // R
        d[i + 1] = 255; // G (cyan edge)
        d[i + 2] = 200; // B
        d[i + 3] = 180; // A
      } else {
        d[i] = 0;
        d[i + 1] = 0;
        d[i + 2] = 0;
        d[i + 3] = 0;
      }
    }
  }

  return output;
}

// ---------------------------------------------------------------------------
// Sobel High-Contrast Edges
// ---------------------------------------------------------------------------
function computeSobelEdges(
  data: ImageData,
  width: number,
  height: number
): ImageData {
  const output = new ImageData(width, height);
  const d = output.data;
  const src = data.data;

  const gray = new Float32Array(width * height);
  for (let i = 0; i < gray.length; i++) {
    const idx = i * 4;
    gray[i] = 0.299 * src[idx] + 0.587 * src[idx + 1] + 0.114 * src[idx + 2];
  }

  for (let y = 1; y < height - 1; y++) {
    for (let x = 1; x < width - 1; x++) {
      const idx = y * width + x;

      const gx =
        -gray[(y - 1) * width + (x - 1)] +
        gray[(y - 1) * width + (x + 1)] +
        -2 * gray[y * width + (x - 1)] +
        2 * gray[y * width + (x + 1)] +
        -gray[(y + 1) * width + (x - 1)] +
        gray[(y + 1) * width + (x + 1)];

      const gy =
        -gray[(y - 1) * width + (x - 1)] +
        -2 * gray[(y - 1) * width + x] +
        -gray[(y - 1) * width + (x + 1)] +
        gray[(y + 1) * width + (x - 1)] +
        2 * gray[(y + 1) * width + x] +
        gray[(y + 1) * width + (x + 1)];

      const magnitude = Math.min(255, Math.sqrt(gx * gx + gy * gy));

      const i = idx * 4;
      d[i] = magnitude;
      d[i + 1] = magnitude;
      d[i + 2] = magnitude;
      d[i + 3] = magnitude > 20 ? 255 : 0;
    }
  }

  return output;
}

// ---------------------------------------------------------------------------
// Structural Similarity Index (SSIM)
// ---------------------------------------------------------------------------
export function computeSSIM(
  orig: ImageData,
  opt: ImageData,
  width: number,
  height: number
): SsimResult {
  const c1 = 6.5025; // (0.01 * 255)^2
  const c2 = 58.5225; // (0.03 * 255)^2

  const src1 = orig.data;
  const src2 = opt.data;

  const len = width * height;
  const luma1 = new Float32Array(len);
  const luma2 = new Float32Array(len);

  for (let i = 0; i < len; i++) {
    const idx = i * 4;
    const a1 = src1[idx + 3] / 255;
    const a2 = src2[idx + 3] / 255;

    luma1[i] =
      (0.299 * src1[idx] + 0.587 * src1[idx + 1] + 0.114 * src1[idx + 2]) * a1;
    luma2[i] =
      (0.299 * src2[idx] + 0.587 * src2[idx + 1] + 0.114 * src2[idx + 2]) * a2;
  }

  const blockSize = 8;
  const step = Math.max(4, Math.floor(Math.min(width, height) / 80));
  let totalSsim = 0;
  let blockCount = 0;

  for (let y = 0; y <= height - blockSize; y += step) {
    for (let x = 0; x <= width - blockSize; x += step) {
      let sumX = 0;
      let sumY = 0;

      for (let by = 0; by < blockSize; by++) {
        const rowOffset = (y + by) * width + x;
        for (let bx = 0; bx < blockSize; bx++) {
          sumX += luma1[rowOffset + bx];
          sumY += luma2[rowOffset + bx];
        }
      }

      const n = blockSize * blockSize;
      const muX = sumX / n;
      const muY = sumY / n;

      let varX = 0;
      let varY = 0;
      let covXY = 0;

      for (let by = 0; by < blockSize; by++) {
        const rowOffset = (y + by) * width + x;
        for (let bx = 0; bx < blockSize; bx++) {
          const dx = luma1[rowOffset + bx] - muX;
          const dy = luma2[rowOffset + bx] - muY;
          varX += dx * dx;
          varY += dy * dy;
          covXY += dx * dy;
        }
      }

      varX /= n - 1;
      varY /= n - 1;
      covXY /= n - 1;

      const num = (2 * muX * muY + c1) * (2 * covXY + c2);
      const den = (muX * muX + muY * muY + c1) * (varX + varY + c2);

      totalSsim += num / den;
      blockCount++;
    }
  }

  const meanSsim = blockCount > 0 ? Math.max(0, Math.min(1, totalSsim / blockCount)) : 1;
  const ssimPercent = Number((meanSsim * 100).toFixed(2));

  let rating: SsimResult["rating"] = "imperceptible";
  if (meanSsim >= 0.99) {
    rating = "imperceptible";
  } else if (meanSsim >= 0.97) {
    rating = "excellent";
  } else if (meanSsim >= 0.93) {
    rating = "good";
  } else if (meanSsim >= 0.85) {
    rating = "fair";
  } else {
    rating = "poor";
  }

  return { ssim: meanSsim, ssimPercent, rating };
}

// ---------------------------------------------------------------------------
// Glow & Shadow Banding / Halo Radar
// ---------------------------------------------------------------------------
export function detectBandingAndHalo(
  orig: ImageData,
  opt: ImageData,
  width: number,
  height: number
): BandingRisk {
  const src1 = orig.data;
  const src2 = opt.data;

  let gradientPixelCount = 0;
  let bandingArtifactCount = 0;
  let haloArtifactCount = 0;

  const step = Math.max(1, Math.floor(Math.min(width, height) / 300));

  for (let y = 1; y < height - 1; y += step) {
    for (let x = 1; x < width - 2; x += step) {
      const idx0 = (y * width + x) * 4;
      const idx1 = (y * width + (x + 1)) * 4;
      const idx2 = (y * width + (x + 2)) * 4;

      const a0_orig = src1[idx0 + 3];
      const a1_orig = src1[idx1 + 3];
      const a2_orig = src1[idx2 + 3];

      const a0_opt = src2[idx0 + 3];
      const a1_opt = src2[idx1 + 3];
      const a2_opt = src2[idx2 + 3];

      const isAlphaGradient =
        (a0_orig > 5 && a0_orig < 250) ||
        (a1_orig > 5 && a1_orig < 250) ||
        (a2_orig > 5 && a2_orig < 250);

      if (isAlphaGradient) {
        gradientPixelCount++;

        const dOrigA1 = Math.abs(a1_orig - a0_orig);
        const dOptA1 = Math.abs(a1_opt - a0_opt);
        const dOrigA2 = Math.abs(a2_orig - a1_orig);
        const dOptA2 = Math.abs(a2_opt - a1_opt);

        if (
          (dOrigA1 >= 1 && dOrigA1 <= 4 && (dOptA1 === 0 || dOptA1 >= 8)) ||
          (dOrigA2 >= 1 && dOrigA2 <= 4 && (dOptA2 === 0 || dOptA2 >= 8))
        ) {
          bandingArtifactCount++;
        }

        const dr = Math.abs(src1[idx1] - src2[idx1]);
        const dg = Math.abs(src1[idx1 + 1] - src2[idx1 + 1]);
        const db = Math.abs(src1[idx1 + 2] - src2[idx1 + 2]);
        if (Math.max(dr, dg, db) > 25 && Math.abs(a1_orig - a1_opt) > 12) {
          haloArtifactCount++;
        }
      } else {
        const lum0_orig = 0.299 * src1[idx0] + 0.587 * src1[idx0 + 1] + 0.114 * src1[idx0 + 2];
        const lum1_orig = 0.299 * src1[idx1] + 0.587 * src1[idx1 + 1] + 0.114 * src1[idx1 + 2];
        const lum2_orig = 0.299 * src1[idx2] + 0.587 * src1[idx2 + 1] + 0.114 * src1[idx2 + 2];

        if (lum1_orig < 100) {
          const dLumOrig1 = Math.abs(lum1_orig - lum0_orig);
          const dLumOrig2 = Math.abs(lum2_orig - lum1_orig);

          if (dLumOrig1 >= 1 && dLumOrig1 <= 3 && dLumOrig2 >= 1 && dLumOrig2 <= 3) {
            gradientPixelCount++;

            const lum0_opt = 0.299 * src2[idx0] + 0.587 * src2[idx0 + 1] + 0.114 * src2[idx0 + 2];
            const lum1_opt = 0.299 * src2[idx1] + 0.587 * src2[idx1 + 1] + 0.114 * src2[idx1 + 2];
            const lum2_opt = 0.299 * src2[idx2] + 0.587 * src2[idx2 + 1] + 0.114 * src2[idx2 + 2];

            const dLumOpt1 = Math.abs(lum1_opt - lum0_opt);
            const dLumOpt2 = Math.abs(lum2_opt - lum1_opt);

            if (
              (dLumOpt1 === 0 && dLumOpt2 >= 6) ||
              (dLumOpt1 >= 6 && dLumOpt2 === 0)
            ) {
              bandingArtifactCount++;
            }
          }
        }
      }
    }
  }

  if (gradientPixelCount === 0) {
    return {
      detected: false,
      severity: "none",
      message: "Görselde kritik gradyan veya alfa geçişi yok.",
    };
  }

  const bandingRatio = (bandingArtifactCount + haloArtifactCount) / gradientPixelCount;

  if (bandingRatio > 0.04 || haloArtifactCount > 40) {
    return {
      detected: true,
      severity: "high",
      message: "Yüksek renk/alfa basamaklanması (banding) tespit edildi.",
      recommendation: {
        alphaQuality: 100,
        method: 6,
        quality: 85,
      },
    };
  }

  if (bandingRatio > 0.015 || haloArtifactCount > 15) {
    return {
      detected: true,
      severity: "medium",
      message: "Geçiş alanlarında orta seviye basamaklanma riski tespit edildi.",
      recommendation: {
        alphaQuality: 100,
        method: 5,
      },
    };
  }

  if (bandingRatio > 0.005) {
    return {
      detected: true,
      severity: "low",
      message: "Hafif renk geçiş kırılması riski mevcut.",
      recommendation: {
        alphaQuality: 95,
      },
    };
  }

  return {
    detected: false,
    severity: "none",
    message: "Renk ve saydamlık geçişleri pürüzsüz.",
  };
}
