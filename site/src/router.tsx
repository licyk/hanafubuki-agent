import { createRouter } from "@tanstack/react-router";

import { NotFound } from "@/components/not-found";
import { routerBasePath } from "@/lib/i18n";
import { routeTree } from "@/routeTree.gen";

export function getRouter() {
  return createRouter({
    routeTree,
    basepath: routerBasePath(import.meta.env.BASE_URL),
    defaultPreload: "intent",
    // Static hosts serve a prerendered page at `page/` and redirect `page`
    // there, and giscus maps `location.pathname` as it is: in-site links that
    // dropped the slash would give every page a second discussion.
    trailingSlash: "always",
    scrollRestoration: true,
    defaultNotFoundComponent: NotFound,
  });
}
