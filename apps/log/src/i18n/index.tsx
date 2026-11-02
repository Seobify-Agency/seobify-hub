import { createContext, useCallback, useContext, useMemo, useState } from "react";
import type { ReactNode } from "react";
import en from "./en";
import tr from "./tr";

export type Lang = "tr" | "en";
export type Params = Record<string, string | number>;

const dictionaries: Record<Lang, Record<string, string>> = { tr, en };

/** Replaces `{name}` placeholders. Exported for tests and non-React code. */
export function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, key: string) => (key in params ? String(params[key]) : match));
}

export function translate(lang: Lang, key: string, params?: Params): string {
  const value = dictionaries[lang][key] ?? dictionaries.tr[key] ?? dictionaries.en[key] ?? key;
  return interpolate(value, params);
}

interface I18nValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: string, params?: Params) => string;
  locale: string;
}

const I18nContext = createContext<I18nValue>({
  lang: "tr",
  setLang: () => {},
  t: (key, params) => translate("tr", key, params),
  locale: "tr-TR",
});

const STORAGE_KEY = "seobify-hub-lang";

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() => {
    try {
      return localStorage.getItem(STORAGE_KEY) === "en" ? "en" : "tr";
    } catch {
      return "tr";
    }
  });
  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* storage unavailable */
    }
    document.documentElement.lang = next;
  }, []);
  const t = useCallback((key: string, params?: Params) => translate(lang, key, params), [lang]);
  const value = useMemo(() => ({ lang, setLang, t, locale: lang === "tr" ? "tr-TR" : "en-US" }), [lang, setLang, t]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  return useContext(I18nContext);
}
