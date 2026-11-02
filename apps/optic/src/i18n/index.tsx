import { createContext, useContext, useState, useCallback } from "react";
import type { ReactNode } from "react";
import en from "./en";
import type { Translations } from "./en";
import tr from "./tr";

export type Lang = "en" | "tr";

const translations: Record<Lang, Translations> = { en, tr };

interface I18nContextValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: string, ...args: number[]) => string;
}

const I18nContext = createContext<I18nContextValue>({
  lang: "en",
  setLang: () => {},
  t: (key: string) => key,
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() => {
    const saved = localStorage.getItem("app-lang");
    return saved === "en" ? "en" : "tr";
  });

  const setLang = useCallback((newLang: Lang) => {
    setLangState(newLang);
    localStorage.setItem("app-lang", newLang);
  }, []);

  const t = useCallback(
    (key: string, ...args: number[]): string => {
      const val = translations[lang]?.[key] ?? translations.en[key] ?? key;
      if (typeof val === "function") return val(...args);
      return val;
    },
    [lang]
  );

  return (
    <I18nContext.Provider value={{ lang, setLang, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useTranslation() {
  return useContext(I18nContext);
}
