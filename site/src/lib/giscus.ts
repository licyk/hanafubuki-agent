import type { Locale } from "@/lib/i18n";

export const GISCUS_ORIGIN = "https://giscus.app";

/**
 * The giscus attributes for the documentation comments.
 *
 * `data-theme` and `data-lang` are only the starting values: once the frame
 * has loaded, it follows the site's theme toggle and language switch.
 */
export const GISCUS_ATTRIBUTES = {
  "data-repo": "licyk/hanafubuki-agent",
  "data-repo-id": "R_kgDOVnCscQ",
  "data-category": "Comment",
  "data-category-id": "DIC_kwDOVnCscc4DKsvU",
  "data-mapping": "pathname",
  "data-strict": "1",
  "data-reactions-enabled": "1",
  "data-emit-metadata": "0",
  "data-input-position": "top",
  "data-loading": "lazy",
  "data-theme": "preferred_color_scheme",
  "data-lang": "zh-CN",
} as const;

export type GiscusTheme = "light" | "dark" | "preferred_color_scheme";

/** The theme `next-themes` resolved, or the operating system's while it is still unknown. */
export function giscusTheme(resolvedTheme: string | undefined): GiscusTheme {
  if (resolvedTheme === undefined) return "preferred_color_scheme";
  return resolvedTheme === "dark" ? "dark" : "light";
}

const GISCUS_LANGUAGES: Record<Locale, string> = {
  "zh-CN": "zh-CN",
  en: "en",
};

export function giscusLanguage(locale: Locale): string {
  return GISCUS_LANGUAGES[locale];
}
