import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchRelease, parseRelease, RELEASE_API } from "@/lib/releases";

const archiveNames = [
  "hanafubuki-agent-0.7.3-aarch64-apple-darwin.tar.gz",
  "hanafubuki-agent-0.7.3-aarch64-pc-windows-msvc.zip",
  "hanafubuki-agent-0.7.3-aarch64-unknown-linux-gnu.tar.gz",
  "hanafubuki-agent-0.7.3-x86_64-apple-darwin.tar.gz",
  "hanafubuki-agent-0.7.3-x86_64-pc-windows-msvc.zip",
  "hanafubuki-agent-0.7.3-x86_64-unknown-linux-gnu.tar.gz",
];

describe("release downloads", () => {
  it("lists all six published platforms and their checksum files", () => {
    const result = parseRelease({
      tag_name: "v0.7.3",
      assets: archiveNames.flatMap((name) => [{ name }, { name: `${name}.sha256` }]),
    });
    expect(result.status).toBe("available");
    if (result.status !== "available") throw new Error("Missing release");
    expect(result.assets).toHaveLength(6);
    expect(result.assets.map((asset) => asset.name).sort()).toEqual(archiveNames);
    for (const asset of result.assets) {
      expect(asset.url).toBe(
        `https://github.com/licyk/hanafubuki-agent/releases/download/v0.7.3/${asset.name}`,
      );
      expect(asset.checksumUrl).toBe(`${asset.url}.sha256`);
    }
    expect(
      result.assets.filter((asset) => asset.platform === "Windows").map((asset) => asset.extension),
    ).toEqual(["zip", "zip"]);
  });

  it("keeps a partial release usable without inventing missing files", () => {
    const result = parseRelease({
      tag_name: "v0.7.3",
      assets: [null, {}, { name: 42 }, { name: "unknown.zip" }, { name: archiveNames[0] }],
    });
    expect(result.status).toBe("available");
    if (result.status !== "available") throw new Error("Missing release");
    expect(result.assets).toHaveLength(1);
    expect(result.assets[0].checksumUrl).toBeUndefined();
  });

  it.each([{ draft: true }, { prerelease: true }, { tag_name: "v0.7.3", assets: [] }])(
    "does not offer unavailable stable assets: %j",
    (value) => {
      expect(parseRelease(value)).toEqual({ status: "unavailable" });
    },
  );

  it.each([null, "invalid", {}, { tag_name: "v0.7.3", assets: null }])(
    "reports malformed responses for retry: %j",
    (value) => {
      expect(() => parseRelease(value)).toThrow();
    },
  );
});

describe("release API failures", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("distinguishes no release from an API rate limit", async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 404 }))
      .mockResolvedValueOnce(new Response(null, { status: 403 }));
    vi.stubGlobal("fetch", request);
    const signal = new AbortController().signal;
    await expect(fetchRelease(signal)).resolves.toEqual({ status: "unavailable" });
    await expect(fetchRelease(signal)).rejects.toThrow("GitHub HTTP 403");
    expect(request).toHaveBeenCalledWith(RELEASE_API, {
      headers: { Accept: "application/vnd.github+json" },
      signal,
    });
  });
});
