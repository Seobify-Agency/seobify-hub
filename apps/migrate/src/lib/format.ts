// Locale-aware formatting helpers (pure functions, unit-tested).

export function formatNumber(value: number | null | undefined, locale = "tr-TR", digits = 0): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "—";
  return new Intl.NumberFormat(locale, { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value);
}

export function formatCompact(value: number | null | undefined, locale = "tr-TR"): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "—";
  return new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

export function formatPercent(ratio: number | null | undefined, locale = "tr-TR", digits = 1): string {
  if (ratio === null || ratio === undefined || Number.isNaN(ratio)) return "—";
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: digits, minimumFractionDigits: digits }).format(ratio);
}

/** Relative change `(current - previous) / previous`; null when previous is 0. */
export function change(current: number, previous: number): number | null {
  if (!previous) return null;
  return (current - previous) / previous;
}

export function formatBytes(bytes: number | null | undefined, locale = "tr-TR"): string {
  if (bytes === null || bytes === undefined) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${formatNumber(value, locale, unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatDate(iso: string | null | undefined, locale = "tr-TR", withTime = true): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, withTime ? { dateStyle: "medium", timeStyle: "short" } : { dateStyle: "medium" }).format(date);
}

/** Builds CSV text (RFC 4180 quoting). Pair with api.writeTextFile(path, csv, true) for Excel. */
export function toCsv(headers: string[], rows: (string | number | null | undefined)[][]): string {
  const cell = (v: string | number | null | undefined) => {
    const s = v === null || v === undefined ? "" : String(v);
    return /[",\r\n;]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [headers.map(cell).join(","), ...rows.map((r) => r.map(cell).join(","))].join("\r\n");
}

/** Shortens a URL for display: drops the scheme and `www.`. */
export function shortUrl(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/^www\./, "");
}
