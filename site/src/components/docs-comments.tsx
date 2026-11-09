import { useTheme } from "fumadocs-ui/provider/base";
import { useEffect, useEffectEvent, useRef } from "react";

import { GISCUS_ATTRIBUTES, GISCUS_ORIGIN, giscusLanguage, giscusTheme } from "@/lib/giscus";
import { useSiteCopy, useSiteLocale } from "@/lib/site-copy";

/**
 * giscus discussion for the current documentation page.
 *
 * The page route remounts this per path, so each page loads its own thread
 * from `location.pathname`. The frame starts from the fixed attributes, and
 * the site's theme and language are posted to it instead of reloading it.
 */
export function DocsComments() {
  const containerRef = useRef<HTMLDivElement>(null);
  const isFrameReady = useRef(false);
  const { resolvedTheme } = useTheme();
  const locale = useSiteLocale();
  const copy = useSiteCopy().docs;
  const theme = giscusTheme(resolvedTheme);
  const lang = giscusLanguage(locale);

  const sync = useEffectEvent(() => {
    const frame = containerRef.current?.querySelector<HTMLIFrameElement>("iframe.giscus-frame");
    frame?.contentWindow?.postMessage({ giscus: { setConfig: { theme, lang } } }, GISCUS_ORIGIN);
  });

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const script = document.createElement("script");
    script.src = `${GISCUS_ORIGIN}/client.js`;
    script.async = true;
    script.crossOrigin = "anonymous";
    for (const [name, value] of Object.entries(GISCUS_ATTRIBUTES)) {
      script.setAttribute(name, value);
    }
    container.append(script);

    // A message posted before the lazy frame has loaded is lost, so the first
    // message the frame sends is what marks it ready for the site's settings.
    function onMessage(event: MessageEvent) {
      if (event.origin !== GISCUS_ORIGIN || isFrameReady.current) return;
      isFrameReady.current = true;
      sync();
    }
    window.addEventListener("message", onMessage);

    return () => {
      window.removeEventListener("message", onMessage);
      isFrameReady.current = false;
      container.replaceChildren();
    };
  }, []);

  useEffect(() => {
    if (isFrameReady.current) sync();
  }, [theme, lang]);

  return (
    <section aria-label={copy.comments} className="mt-12 border-t pt-8">
      <div ref={containerRef} />
    </section>
  );
}
