import { createRootRoute, HeadContent, Outlet, Scripts } from "@tanstack/react-router";

import { SiteProvider } from "@/components/site-provider";
import { DEFAULT_LOCALE } from "@/lib/i18n";
import { SITE_NAME } from "@/lib/layout.shared";
import { getSiteCopy } from "@/lib/site-copy";
import appCss from "@/styles/app.css?url";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      {
        title: SITE_NAME,
      },
      {
        name: "description",
        content: getSiteCopy(DEFAULT_LOCALE).home.description,
      },
    ],
    links: [
      { rel: "stylesheet", href: appCss },
      {
        rel: "icon",
        type: "image/svg+xml",
        href: `${import.meta.env.BASE_URL}favicon.svg`,
      },
    ],
    scripts: [
      {
        src: "https://licyk-umami.netlify.app/script.js",
        async: true,
        "data-website-id": "11187ebe-09a5-4a52-a81b-7438a4e3f765",
      },
    ],
  }),
  component: RootComponent,
});

function RootComponent() {
  return (
    <html lang={DEFAULT_LOCALE} suppressHydrationWarning>
      <head>
        <HeadContent />
      </head>
      <body className="flex min-h-screen flex-col">
        <SiteProvider>
          <Outlet />
        </SiteProvider>
        <Scripts />
      </body>
    </html>
  );
}
