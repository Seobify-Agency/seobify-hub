import { describe, expect, it } from "vitest";
import en from "../i18n/en";
import tr from "../i18n/tr";
import { interpolate, translate } from "../i18n";
import { change, formatBytes, shortUrl, toCsv } from "../lib/format";
import { niceMax } from "../components/charts";

describe("i18n", () => {
  it("tr and en have the same keys", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(tr).sort());
  });
  it("interpolates params and falls back to the key", () => {
    expect(interpolate("{a} / {b}", { a: 1, b: "x" })).toBe("1 / x");
    expect(translate("en", "common.rows", { count: 3 })).toBe("3 rows");
    expect(translate("tr", "missing.key")).toBe("missing.key");
  });
});

describe("format", () => {
  it("computes relative change", () => {
    expect(change(150, 100)).toBeCloseTo(0.5);
    expect(change(5, 0)).toBeNull();
  });
  it("quotes CSV cells", () => {
    expect(toCsv(["a", "b"], [["x,y", 'q"t'], [1, null]])).toBe('a,b\r\n"x,y","q""t"\r\n1,');
  });
  it("formats bytes and urls", () => {
    expect(formatBytes(2048, "en-US")).toBe("2.0 KB");
    expect(shortUrl("https://www.a.com/x")).toBe("a.com/x");
  });
  it("rounds axis maxima", () => {
    expect(niceMax(87)).toBe(100);
    expect(niceMax(0)).toBe(1);
    expect(niceMax(2300)).toBe(2500);
  });
});
