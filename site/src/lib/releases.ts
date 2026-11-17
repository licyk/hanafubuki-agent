import { REPOSITORY_URL } from "@/lib/layout.shared";

export const RELEASES_URL = `${REPOSITORY_URL}/releases`;
export const RELEASE_API = "https://api.github.com/repos/licyk/hanafubuki-agent/releases/latest";
export const RELEASE_TARGETS = [
  { target: "x86_64-pc-windows-msvc", platform: "Windows", arch: "x64", extension: "zip" },
  { target: "aarch64-pc-windows-msvc", platform: "Windows", arch: "ARM64", extension: "zip" },
  { target: "x86_64-apple-darwin", platform: "macOS", arch: "Intel", extension: "tar.gz" },
  { target: "aarch64-apple-darwin", platform: "macOS", arch: "Apple silicon", extension: "tar.gz" },
  { target: "x86_64-unknown-linux-gnu", platform: "Linux", arch: "x64", extension: "tar.gz" },
  { target: "aarch64-unknown-linux-gnu", platform: "Linux", arch: "ARM64", extension: "tar.gz" },
] as const;

export interface ReleaseAsset {
  name: string;
  platform: string;
  arch: string;
  extension: string;
  url: string;
  checksumUrl?: string;
}

export type ReleaseResult =
  | { status: "available"; version: string; url: string; assets: ReleaseAsset[] }
  | { status: "unavailable" };

export function parseRelease(value: unknown): ReleaseResult {
  if (!value || typeof value !== "object") throw new Error("Invalid release response");
  const data = value as Record<string, unknown>;
  if (data.draft === true || data.prerelease === true) return { status: "unavailable" };
  if (typeof data.tag_name !== "string" || !Array.isArray(data.assets)) {
    throw new Error("Incomplete release response");
  }
  const version = data.tag_name.replace(/^v/, "");
  const tag = encodeURIComponent(data.tag_name);
  const names = new Set(
    data.assets.flatMap((asset) =>
      asset && typeof asset === "object" && typeof asset.name === "string" ? [asset.name] : [],
    ),
  );
  const download = (name: string) => `${RELEASES_URL}/download/${tag}/${encodeURIComponent(name)}`;
  const assets = RELEASE_TARGETS.flatMap(({ target, ...platform }) => {
    const name = `hanafubuki-agent-${version}-${target}.${platform.extension}`;
    return names.has(name)
      ? [
          {
            ...platform,
            name,
            url: download(name),
            checksumUrl: names.has(`${name}.sha256`) ? download(`${name}.sha256`) : undefined,
          },
        ]
      : [];
  });
  return assets.length
    ? { status: "available", version: data.tag_name, url: `${RELEASES_URL}/tag/${tag}`, assets }
    : { status: "unavailable" };
}

export async function fetchRelease(signal: AbortSignal): Promise<ReleaseResult> {
  const response = await fetch(RELEASE_API, {
    headers: { Accept: "application/vnd.github+json" },
    signal,
  });
  if (response.status === 404) return { status: "unavailable" };
  if (!response.ok) throw new Error(`GitHub HTTP ${response.status}`);
  return parseRelease(await response.json());
}
