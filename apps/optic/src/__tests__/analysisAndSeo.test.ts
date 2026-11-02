import { describe, it, expect, beforeAll } from "vitest";
import { computeSSIM, detectBandingAndHalo } from "../workers/inspectionWorker";
import {
  generateDefaultVariants,
  generatePictureSnippet,
  calculateWebVitalsImpact,
} from "../services/seoService";
import type { SeoPictureConfig } from "../types";

// Polyfill ImageData for node test environment if needed
beforeAll(() => {
  if (typeof globalThis.ImageData === "undefined") {
    class MockImageData {
      width: number;
      height: number;
      data: Uint8ClampedArray;
      constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
        this.data = new Uint8ClampedArray(width * height * 4);
      }
    }
    (globalThis as unknown as { ImageData: typeof MockImageData }).ImageData =
      MockImageData;
  }
});

describe("SSIM Calculation", () => {
  it("should return 1.0 (100%) and imperceptible for identical images", () => {
    const width = 16;
    const height = 16;
    const img1 = new ImageData(width, height);
    const img2 = new ImageData(width, height);

    // Fill with pattern
    for (let i = 0; i < img1.data.length; i += 4) {
      img1.data[i] = 120;
      img1.data[i + 1] = 140;
      img1.data[i + 2] = 160;
      img1.data[i + 3] = 255;

      img2.data[i] = 120;
      img2.data[i + 1] = 140;
      img2.data[i + 2] = 160;
      img2.data[i + 3] = 255;
    }

    const result = computeSSIM(img1, img2, width, height);
    expect(result.ssim).toBeCloseTo(1.0, 2);
    expect(result.ssimPercent).toBeGreaterThanOrEqual(99.9);
    expect(result.rating).toBe("imperceptible");
  });

  it("should detect lower SSIM for altered images", () => {
    const width = 16;
    const height = 16;
    const img1 = new ImageData(width, height);
    const img2 = new ImageData(width, height);

    for (let i = 0; i < img1.data.length; i += 4) {
      img1.data[i] = 200;
      img1.data[i + 1] = 200;
      img1.data[i + 2] = 200;
      img1.data[i + 3] = 255;

      img2.data[i] = 20;
      img2.data[i + 1] = 20;
      img2.data[i + 2] = 20;
      img2.data[i + 3] = 255;
    }

    const result = computeSSIM(img1, img2, width, height);
    expect(result.ssim).toBeLessThan(0.7);
    expect(result.rating).toBe("poor");
  });
});

describe("Banding and Halo Radar", () => {
  it("should report no risk on uniform/clean images", () => {
    const width = 32;
    const height = 32;
    const orig = new ImageData(width, height);
    const opt = new ImageData(width, height);

    const risk = detectBandingAndHalo(orig, opt, width, height);
    expect(risk.detected).toBe(false);
    expect(risk.severity).toBe("none");
  });
});

describe("SEO & Responsive Picture Generator", () => {
  it("generates correct responsive breakpoints based on image dimensions", () => {
    const variants = generateDefaultVariants(1200, 800);
    expect(variants.length).toBe(4);
    expect(variants[0].label).toContain("Mobile");
    expect(variants[0].width).toBe(480);
    expect(variants[1].width).toBe(768);
    expect(variants[2].width).toBe(1200);
  });

  it("generates W3C compliant <picture> tag with LCP optimization", () => {
    const variants = generateDefaultVariants(1200, 800);
    const config: SeoPictureConfig = {
      altText: "Hero Banner",
      isLcp: true,
      loading: "eager",
      decoding: "async",
      fetchPriority: "high",
      variants,
    };

    const { pictureHtml, preloadTag } = generatePictureSnippet(
      "hero.png",
      1200,
      800,
      config
    );

    expect(pictureHtml).toContain("<picture>");
    expect(pictureHtml).toContain('type="image/webp"');
    expect(pictureHtml).toContain('fetchpriority="high"');
    expect(pictureHtml).toContain('alt="Hero Banner"');
    expect(preloadTag).toContain('rel="preload"');
    expect(preloadTag).toContain('href="hero.webp"');
  });

  it("calculates Core Web Vitals speed gains accurately", () => {
    const originalBytes = 1_500_000; // ~1.5 MB
    const optimizedBytes = 150_000; // ~150 KB (90% savings)

    const impact = calculateWebVitalsImpact(originalBytes, optimizedBytes);
    expect(impact.savedPercent).toBe(90);
    expect(impact.savedKb).toBeGreaterThan(1300);
    expect(impact.lcpRating).toBe("excellent");
    expect(impact.savedTime4gMs).toBeGreaterThan(0);
    expect(impact.savedTime3gMs).toBeGreaterThan(impact.savedTime4gMs);
  });
});
