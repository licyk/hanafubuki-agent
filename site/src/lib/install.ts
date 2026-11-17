export type InstallPlatform = "windows" | "macos" | "linux";

export const INSTALL_PLATFORMS: readonly InstallPlatform[] = ["windows", "macos", "linux"];
export const SCRIPT_BASE = "https://raw.githubusercontent.com/licyk/hanafubuki-agent/main";
export const POWERSHELL_INSTALL = `irm ${SCRIPT_BASE}/install.ps1 | iex`;
export const SHELL_INSTALL = `curl -fsSL ${SCRIPT_BASE}/install.sh | bash`;
export const CARGO_INSTALL = "cargo install --locked hanafubuki-agent-cli";
export const SOURCE_INSTALL =
  "git clone https://github.com/licyk/hanafubuki-agent.git\ncd hanafubuki-agent\ncargo install --locked --path crates/cli";

export function installCommand(platform: InstallPlatform): string {
  return platform === "windows" ? POWERSHELL_INSTALL : SHELL_INSTALL;
}

export function detectPlatform(navigator: {
  userAgent: string;
  platform: string;
  maxTouchPoints?: number;
}): InstallPlatform | null {
  const { userAgent, platform, maxTouchPoints = 0 } = navigator;
  if (
    /Android|iPhone|iPad|iPod/i.test(userAgent) ||
    (/Mac/i.test(platform) && maxTouchPoints > 1)
  ) {
    return null;
  }
  if (/Win/i.test(platform) || /Windows/i.test(userAgent)) return "windows";
  if (/Mac/i.test(platform) || /Macintosh/i.test(userAgent)) return "macos";
  if (/Linux/i.test(platform) || /Linux/i.test(userAgent)) return "linux";
  return null;
}
