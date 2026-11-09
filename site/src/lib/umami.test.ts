import { describe, expect, it } from "vitest";

import { Route } from "@/routes/__root";

describe("umami", () => {
  it("loads the site's tracking script asynchronously", async () => {
    const head = await Route.options.head?.({} as never);

    expect(head?.scripts).toContainEqual({
      src: "https://licyk-umami.netlify.app/script.js",
      async: true,
      "data-website-id": "11187ebe-09a5-4a52-a81b-7438a4e3f765",
    });
  });
});
