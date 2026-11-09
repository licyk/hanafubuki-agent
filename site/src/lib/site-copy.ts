import { useI18n } from "fumadocs-ui/contexts/i18n";

import { DEFAULT_LOCALE, isLocale, type Locale } from "@/lib/i18n";

const zhCN = {
  theme: {
    hint: "点击切换主题",
    modes: { system: "主题：跟随系统", light: "主题：浅色", dark: "主题：深色" },
  },
  docs: {
    comments: "评论",
  },
  home: {
    title: "hanafubuki-agent — 终端里的编程与运维 Agent",
    description: "终端里的编程与运维 Agent，也可以嵌入到应用中。",
    install: "安装命令",
    installation: "安装",
    quickStart: "快速开始",
    github: "GitHub",
    footer: {
      label: "页脚导航",
      docs: "文档",
      github: "GitHub",
    },
  },
} as const;

type TranslationShape<T> = T extends string
  ? string
  : T extends readonly unknown[]
    ? { readonly [K in keyof T]: TranslationShape<T[K]> }
    : T extends object
      ? { readonly [K in keyof T]: TranslationShape<T[K]> }
      : T;

export type SiteCopy = TranslationShape<typeof zhCN>;

const en = {
  theme: {
    hint: "Click to switch theme",
    modes: { system: "Theme: follow system", light: "Theme: light", dark: "Theme: dark" },
  },
  docs: {
    comments: "Comments",
  },
  home: {
    title: "hanafubuki-agent — A coding and operations agent for the terminal",
    description: "A coding and operations agent for the terminal, embeddable in applications.",
    install: "Install command",
    installation: "Installation",
    quickStart: "Quick start",
    github: "GitHub",
    footer: {
      label: "Footer navigation",
      docs: "Docs",
      github: "GitHub",
    },
  },
} as const satisfies SiteCopy;

const SITE_COPY: Record<Locale, SiteCopy> = { "zh-CN": zhCN, en };

export function getSiteCopy(locale: Locale): SiteCopy {
  return SITE_COPY[locale];
}

export function useSiteLocale(): Locale {
  const { locale } = useI18n();
  return isLocale(locale) ? locale : DEFAULT_LOCALE;
}

export function useSiteCopy(): SiteCopy {
  return getSiteCopy(useSiteLocale());
}
