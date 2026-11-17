import { describe, expect, it } from "vitest";

import { detectPlatform, installCommand } from "@/lib/install";

describe("desktop installer selection", () => {
  it.each([
    ["Win32", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "windows", "install.ps1 | iex"],
    ["MacIntel", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "macos", "install.sh | bash"],
    ["Linux aarch64", "Mozilla/5.0 (X11; Linux aarch64)", "linux", "install.sh | bash"],
    ["", "Mozilla/5.0 (Windows NT 10.0; ARM64)", "windows", "install.ps1 | iex"],
  ])("selects the shell for %s %s", (platform, userAgent, expected, suffix) => {
    const detected = detectPlatform({ platform, userAgent });
    expect(detected).toBe(expected);
    expect(installCommand(detected!)).toMatch(new RegExp(`${suffix.replace(/[.|]/g, "\\$&")}$`));
  });

  it.each([
    { platform: "Linux armv8l", userAgent: "Mozilla/5.0 (Linux; Android 16)" },
    { platform: "iPhone", userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)" },
    {
      platform: "MacIntel",
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
      maxTouchPoints: 5,
    },
    { platform: "", userAgent: "" },
    { platform: "FreeBSD", userAgent: "Mozilla/5.0 (X11; FreeBSD amd64)" },
  ])("asks for a manual platform on unsupported devices: %j", (browser) => {
    expect(detectPlatform(browser)).toBeNull();
  });
});
