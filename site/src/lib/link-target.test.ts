import { describe, expect, it } from "vitest";

import { linkTarget } from "@/lib/link-target";

describe("linkTarget", () => {
  it("keeps a link to a page as its path", () => {
    expect(linkTarget("/docs/embedding")).toEqual({ to: "/docs/embedding" });
  });

  it("gives a link to a heading on another page its fragment apart", () => {
    expect(linkTarget("/docs/embedding#the-panic-hook-bridge")).toEqual({
      to: "/docs/embedding",
      hash: "the-panic-hook-bridge",
    });
    expect(linkTarget("/en/docs/sessions/#show")).toEqual({
      to: "/en/docs/sessions/",
      hash: "show",
    });
  });
});
