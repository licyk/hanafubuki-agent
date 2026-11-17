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
  install: {
    copy: "复制命令",
    copied: "已复制",
    copyFailed: "复制失败，请手动复制",
    platform: "安装平台",
    platforms: { windows: "Windows", macos: "macOS", linux: "Linux" },
    choose: "选择要安装的平台，即可查看安装命令。",
    hint: "在终端中运行命令。已安装？再次运行即可更新。",
  },
  download: {
    title: "下载 Hanafubuki Agent",
    description: "选择适合你的安装方式，开始使用终端里的 Agent。",
    methods: "安装方式",
    scripts: "一键安装",
    binaries: "预编译程序",
    source: "从源码安装",
    scriptDescription: "自动选择当前系统和架构，下载最新稳定版、校验 SHA-256，并配置用户 PATH。",
    shellNote:
      "install.sh 需要 Bash 和 curl，也支持 Windows 的 Git Bash / MSYS2。PowerShell 脚本支持 Windows PowerShell 5.1 和 PowerShell 7。",
    options: "指定版本、安装目录与卸载方法",
    binaryDescription:
      "无需 Rust 环境。下载与你的系统和处理器对应的压缩包，解压后运行 hanafubuki-agent。",
    get: "下载",
    loading: "正在获取最新稳定版…",
    error: "暂时无法获取版本信息，请重试或前往 GitHub Releases 下载。",
    unavailable: "当前没有可用的稳定版文件，请前往 GitHub Releases 查看。",
    retry: "重试",
    releases: "所有版本 · GitHub Releases",
    platformNote:
      "Linux 需要 glibc 2.35 或更新版本。目前不提供 32 位或 musl 预编译包。SHA-256 链接提供对应压缩包的校验文件。",
    cargo: "通过 Cargo 安装",
    cargoDescription: "已有 Rust 工具链？需要 Rust 1.99 或更新版本，Cargo 将在本机编译最新发布版。",
    sourceDescription: "获取 main 分支的最新代码并在本机编译。需要 Git 与 Rust 1.99 或更新版本。",
    update:
      "安装后，可运行 hanafubuki-agent update 更新预编译程序；Cargo 用户重新运行安装命令即可。更新会保留配置与对话。",
    start: "快速开始",
    guide: "完整安装文档",
  },
  home: {
    title: "hanafubuki-agent — 终端里的编程与运维 Agent",
    description: "终端里的编程与运维 Agent，也可以嵌入到应用中。",
    installation: "下载与安装",
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
  install: {
    copy: "Copy command",
    copied: "Copied",
    copyFailed: "Copy failed; select the command manually",
    platform: "Install platform",
    platforms: { windows: "Windows", macos: "macOS", linux: "Linux" },
    choose: "Choose the platform you want to install on to see its command.",
    hint: "Run this command in your terminal. Run it again to update an existing installation.",
  },
  download: {
    title: "Download Hanafubuki Agent",
    description: "Choose an installation method and get started with the agent in your terminal.",
    methods: "Installation methods",
    scripts: "One-command install",
    binaries: "Prebuilt binaries",
    source: "Install from source",
    scriptDescription:
      "Detect your system and architecture, download the latest stable release, verify SHA-256, and configure your user PATH.",
    shellNote:
      "install.sh requires Bash and curl, and also supports Git Bash / MSYS2 on Windows. The PowerShell script supports Windows PowerShell 5.1 and PowerShell 7.",
    options: "Choose a version or directory, and uninstall",
    binaryDescription:
      "No Rust toolchain needed. Download the archive for your system and processor, extract it, then run hanafubuki-agent.",
    get: "Download",
    loading: "Fetching the latest stable release…",
    error:
      "Release information is temporarily unavailable. Retry or download from GitHub Releases.",
    unavailable: "No stable release files are available here yet. Check GitHub Releases.",
    retry: "Retry",
    releases: "All versions · GitHub Releases",
    platformNote:
      "Linux requires glibc 2.35 or newer. No 32-bit or musl binaries are provided. Each SHA-256 link downloads the archive’s checksum file.",
    cargo: "Install with Cargo",
    cargoDescription:
      "Already have a Rust toolchain? Rust 1.99 or newer is required. Cargo compiles the latest published version on your machine.",
    sourceDescription:
      "Build the latest code from the main branch. Requires Git and Rust 1.99 or newer.",
    update:
      "After installation, run hanafubuki-agent update to update a prebuilt binary, or rerun the Cargo install command. Updates preserve configuration and conversations.",
    start: "Quick start",
    guide: "Full installation guide",
  },
  home: {
    title: "hanafubuki-agent — A coding and operations agent for the terminal",
    description: "A coding and operations agent for the terminal, embeddable in applications.",
    installation: "Download",
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
