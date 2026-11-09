import { describe, expect, it } from "vitest";

import { GISCUS_ATTRIBUTES, giscusLanguage, giscusTheme } from "@/lib/giscus";
import { LOCALES } from "@/lib/i18n";

describe("giscus", () => {
  it("uses the repository's discussion category, one discussion per page path", () => {
    expect(GISCUS_ATTRIBUTES).toEqual({
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
    });
  });

  it("follows the site's resolved theme, and the system's until it is known", () => {
    expect(giscusTheme("dark")).toBe("dark");
    expect(giscusTheme("light")).toBe("light");
    expect(giscusTheme(undefined)).toBe("preferred_color_scheme");
  });

  it("has a giscus language for every site locale", () => {
    for (const locale of LOCALES) expect(giscusLanguage(locale)).toBeTruthy();
    expect(giscusLanguage("zh-CN")).toBe("zh-CN");
    expect(giscusLanguage("en")).toBe("en");
  });
});
