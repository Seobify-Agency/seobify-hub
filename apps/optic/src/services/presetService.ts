import type { Preset } from "../types";

export const PRESETS: Preset[] = [
  {
    id: "balanced",
    name: "Balanced (Recommended)",
    description: "Web-ready balance of quality and size (q80)",
    config: {
      quality: 80,
      alphaQuality: 90,
      method: 4,
      lossless: false,
      nearLossless: null,
      metadata: "none",
    },
  },
  {
    id: "premium-hero",
    name: "Highest Quality",
    description: "Hero and showcase images, hardly compressed (q95, m6)",
    config: {
      quality: 95,
      alphaQuality: 100,
      method: 6,
      lossless: false,
      nearLossless: null,
      metadata: "none",
    },
  },
  {
    id: "aggressive",
    name: "Smallest File",
    description: "Size first, slight quality loss (q60)",
    config: {
      quality: 60,
      alphaQuality: 75,
      method: 4,
      lossless: false,
      nearLossless: null,
      metadata: "none",
    },
  },
  {
    id: "transparency-safe",
    name: "Transparent PNG (Alpha Safe)",
    description: "Keeps transparent edges clean (q90, alpha 100)",
    config: {
      quality: 90,
      alphaQuality: 100,
      method: 5,
      lossless: false,
      nearLossless: null,
      metadata: "none",
    },
  },
  {
    id: "glow-safe",
    name: "Smooth Gradients",
    description: "No banding in glows, shadows and gradients (q92, m6)",
    config: {
      quality: 92,
      alphaQuality: 100,
      method: 6,
      lossless: false,
      nearLossless: null,
      metadata: "none",
    },
  },
  {
    id: "near-lossless",
    name: "Near Lossless",
    description: "No visible difference, larger file (lossless + near_lossless 60)",
    config: {
      quality: 100,
      alphaQuality: 100,
      method: 6,
      lossless: true,
      nearLossless: 60,
      metadata: "none",
    },
  },
];
