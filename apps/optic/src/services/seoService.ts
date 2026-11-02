import type { ResponsiveVariant, SeoPictureConfig } from "../types";

export const SLUG_FALLBACK = "seobify-gorsel";

/** Tokens that never carry SEO meaning (generator/camera prefixes); months follow. */
const SLUG_STOP_TOKENS = [
  "chatgpt", "gemini", "generated", "untitled", "adsiz", "image", "img", "dsc", "dscn", "dcim", "pxl",
  "screenshot", "ekran", "goruntusu", "whatsapp", "photo", "picture", "copy", "kopya", "final", "edited",
];
const SLUG_MONTHS = [
  "ocak", "subat", "mart", "nisan", "mayis", "haziran", "temmuz", "agustos", "eylul", "eyl", "ekim", "kasim",
  "aralik", "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
];
const FOLD: Record<string, string> = {
  ş: "s", Ş: "s", ı: "i", I: "i", İ: "i", î: "i", Î: "i", ï: "i", í: "i", ì: "i",
  ğ: "g", Ğ: "g", ü: "u", Ü: "u", û: "u", Û: "u", ú: "u", ù: "u",
  ö: "o", Ö: "o", ô: "o", ó: "o", ò: "o", ø: "o", ç: "c", Ç: "c",
  â: "a", Â: "a", á: "a", à: "a", ä: "a", ã: "a", å: "a",
  é: "e", è: "e", ê: "e", ë: "e", É: "e", ñ: "n", ß: "ss",
};

/**
 * Turns a human/camera/AI file name into an SEO-friendly slug: Turkish and Latin
 * diacritics are folded, generator and camera prefixes (ChatGPT Image, Gemini Generated,
 * IMG_, Untitled…), dates/times and parenthesised counters are dropped, and the rest is
 * kebab-cased. Mirrors `to_seo_slug` in src-tauri/src/services/processor.rs.
 */
export function toSeoSlug(input: string): string {
  let name = input.trim();
  const extMatch = name.match(/^(.+)\.([A-Za-z0-9]{2,5})$/);
  if (extMatch) name = extMatch[1];
  name = name.replace(/\(\s*\d+\s*\)/g, "");
  let folded = "";
  for (const ch of name) folded += FOLD[ch] ?? ch.toLowerCase();
  const raw = folded.split(/[^a-z0-9]+/).filter((t) => t.length > 0);
  const isNum = (t: string) => /^\d+$/.test(t);
  const isMonth = (t: string) => SLUG_MONTHS.includes(t);
  const keep: string[] = [];
  raw.forEach((tok, i) => {
    if (SLUG_STOP_TOKENS.includes(tok) || SLUG_MONTHS.includes(tok)) return;
    // Canva/Figma defaults: "Untitled design" / "Adsız tasarım".
    if ((tok === "design" || tok === "tasarim") && i > 0 && (raw[i - 1] === "untitled" || raw[i - 1] === "adsiz")) return;
    if (isNum(tok)) {
      const long = tok.length >= 4;
      const prevNumeric = i > 0 && (isNum(raw[i - 1]) || isMonth(raw[i - 1]));
      const nextNumeric = i + 1 < raw.length && (isNum(raw[i + 1]) || isMonth(raw[i + 1]));
      if (long || prevNumeric || nextNumeric) return;
    }
    keep.push(tok);
  });
  let slug = keep.join("-");
  if (slug.length > 80) {
    const cut = slug.slice(0, 80).lastIndexOf("-");
    slug = slug.slice(0, cut > 0 ? cut : 80);
  }
  slug = slug.replace(/^-+|-+$/g, "");
  return slug.length > 0 ? slug : SLUG_FALLBACK;
}

export function generateDefaultVariants(
  width: number,
  height: number
): ResponsiveVariant[] {
  const aspectRatio = height / width;

  const standardWidths = [
    { label: "Mobile (480w)", suffix: "-mobile", targetW: 480 },
    { label: "Tablet (768w)", suffix: "-tablet", targetW: 768 },
    { label: "Desktop (1x)", suffix: "", targetW: Math.min(width, 1200) },
    { label: "Retina (2x)", suffix: "@2x", targetW: Math.min(width * 2, 2400) },
  ];

  return standardWidths.map(({ label, suffix, targetW }) => {
    const finalW = Math.round(targetW);
    const finalH = Math.round(finalW * aspectRatio);
    return {
      label,
      suffix,
      width: finalW,
      height: finalH,
      scale: Number((finalW / width).toFixed(2)),
      enabled: finalW <= width * 2,
    };
  });
}

export function generatePictureSnippet(
  baseFileName: string,
  width: number,
  height: number,
  config: SeoPictureConfig
): { pictureHtml: string; preloadTag: string } {
  const cleanName = baseFileName.replace(/\.(png|jpg|jpeg|webp)$/i, "");
  const activeVariants = config.variants.filter((v) => v.enabled);

  const mainVariant =
    activeVariants.find((v) => v.suffix === "") ||
    activeVariants[activeVariants.length - 1] || {
      width,
      height,
      suffix: "",
    };

  const mainSrc = `${cleanName}${mainVariant.suffix}.webp`;

  // Build srcset
  const srcsetEntries = activeVariants
    .map((v) => `${cleanName}${v.suffix}.webp ${v.width}w`)
    .join(",\n    ");

  const sizesAttr =
    activeVariants.length > 1
      ? `sizes="(max-width: 768px) 100vw, ${mainVariant.width}px"`
      : `sizes="${mainVariant.width}px"`;

  const lcpAttrs = config.isLcp
    ? `loading="eager"\n    fetchpriority="high"`
    : `loading="${config.loading}"\n    decoding="${config.decoding}"`;

  const pictureHtml = `<!-- Optimized WebP Picture - Core Web Vitals Ready -->
<picture>
  <source
    type="image/webp"
    srcset="${srcsetEntries}"
    ${sizesAttr}
  />
  <img
    src="${mainSrc}"
    alt="${config.altText || cleanName}"
    width="${mainVariant.width}"
    height="${mainVariant.height}"
    ${lcpAttrs}
  />
</picture>`;

  const preloadTag = config.isLcp
    ? `<link rel="preload" as="image" href="${mainSrc}" type="image/webp" fetchpriority="high" />`
    : "";

  return { pictureHtml, preloadTag };
}

export interface WebVitalsImpact {
  originalKb: number;
  optimizedKb: number;
  savedKb: number;
  savedPercent: number;
  downloadTime4gMs: number;
  downloadTime3gMs: number;
  savedTime4gMs: number;
  savedTime3gMs: number;
  lcpRating: "excellent" | "good" | "moderate";
}

export function calculateWebVitalsImpact(
  originalBytes: number,
  optimizedBytes: number
): WebVitalsImpact {
  const originalKb = Number((originalBytes / 1024).toFixed(1));
  const optimizedKb = Number((optimizedBytes / 1024).toFixed(1));
  const savedKb = Number(Math.max(0, originalKb - optimizedKb).toFixed(1));
  const savedPercent =
    originalBytes > 0
      ? Number((((originalBytes - optimizedBytes) / originalBytes) * 100).toFixed(1))
      : 0;

  // 4G average speed: ~15 Mbps (approx 1.87 MB/s = 1920 KB/s) + 50ms latency
  const downloadTime4gMsOrig = Math.round((originalKb / 1920) * 1000 + 50);
  const downloadTime4gMsOpt = Math.round((optimizedKb / 1920) * 1000 + 50);
  const savedTime4gMs = Math.max(0, downloadTime4gMsOrig - downloadTime4gMsOpt);

  // Fast 3G average speed: ~1.6 Mbps (approx 200 KB/s) + 150ms latency
  const downloadTime3gMsOrig = Math.round((originalKb / 200) * 1000 + 150);
  const downloadTime3gMsOpt = Math.round((optimizedKb / 200) * 1000 + 150);
  const savedTime3gMs = Math.max(0, downloadTime3gMsOrig - downloadTime3gMsOpt);

  let lcpRating: WebVitalsImpact["lcpRating"] = "good";
  if (savedPercent >= 70 || savedKb > 300) {
    lcpRating = "excellent";
  } else if (savedPercent >= 40) {
    lcpRating = "good";
  } else {
    lcpRating = "moderate";
  }

  return {
    originalKb,
    optimizedKb,
    savedKb,
    savedPercent,
    downloadTime4gMs: downloadTime4gMsOpt,
    downloadTime3gMs: downloadTime3gMsOpt,
    savedTime4gMs,
    savedTime3gMs,
    lcpRating,
  };
}
