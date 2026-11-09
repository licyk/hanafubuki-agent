import { Link } from "@tanstack/react-router";
import type { ComponentProps } from "react";

import { linkTarget } from "@/lib/link-target";

/** Fumadocs' TanStack link, with the fragment passed to the router apart. */
export function SiteLink({
  href = "",
  prefetch = true,
  ...props
}: ComponentProps<"a"> & { prefetch?: boolean }) {
  if (href.startsWith("#")) return <a href={href} {...props} />;
  const { to, hash } = linkTarget(href);
  return <Link to={to} hash={hash} preload={prefetch ? "intent" : false} {...props} />;
}
