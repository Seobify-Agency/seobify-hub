import { describe, it, expect } from "vitest";
import { toSeoSlug, SLUG_FALLBACK, generatePictureSnippet } from "../services/seoService";
import { PRESETS } from "../services/presetService";

describe("toSeoSlug", () => {
  it("folds Turkish characters and kebab-cases", () => {
    expect(toSeoSlug("Kurumsal SEO Danışmanlığı.webp")).toBe("kurumsal-seo-danismanligi");
    expect(toSeoSlug("  --Özel__Çağrı--  ")).toBe("ozel-cagri");
    expect(toSeoSlug("iPhone 15 Pro Max kılıf.jpg")).toBe("iphone-15-pro-max-kilif");
  });

  it("drops generator/camera prefixes, dates, times and counters", () => {
    expect(toSeoSlug("ChatGPT Image 23 Eyl 2026 23_36_44 (1).png")).toBe(SLUG_FALLBACK);
    expect(toSeoSlug("IMG_4821.JPG")).toBe(SLUG_FALLBACK);
    expect(toSeoSlug("Gemini_Generated_Image_abc123.jpeg")).toBe("abc123");
    expect(toSeoSlug("Seobify Blog Görseli (2).png")).toBe("seobify-blog-gorseli");
    expect(toSeoSlug("Adsız tasarım (3).png")).toBe(SLUG_FALLBACK);
    expect(toSeoSlug("2026-10-29 rapor.png")).toBe("rapor");
    expect(toSeoSlug("")).toBe(SLUG_FALLBACK);
  });

  it("caps the length without a dangling dash", () => {
    const slug = toSeoSlug("kelime ".repeat(30));
    expect(slug.length).toBeLessThanOrEqual(80);
    expect(slug.endsWith("-")).toBe(false);
  });

  it("feeds the <picture> snippet with the slug", () => {
    const { pictureHtml } = generatePictureSnippet("kurumsal-seo-danismanligi", 1200, 800, {
      altText: "Kurumsal SEO danışmanlığı",
      isLcp: true,
      loading: "eager",
      decoding: "async",
      fetchPriority: "high",
      variants: [{ label: "Desktop", suffix: "", width: 1200, height: 800, scale: 1, enabled: true }],
    });
    expect(pictureHtml).toContain('src="kurumsal-seo-danismanligi.webp"');
  });
});

describe("presets", () => {
  it("start with the recommended balanced preset and keep the six core presets", () => {
    expect(PRESETS.map((p) => p.id)).toEqual([
      "balanced",
      "premium-hero",
      "aggressive",
      "transparency-safe",
      "glow-safe",
      "near-lossless",
    ]);
    expect(PRESETS[0].config).toMatchObject({ quality: 80, method: 4, metadata: "none" });
    expect(PRESETS.every((p) => p.config.metadata === "none")).toBe(true);
  });
});
