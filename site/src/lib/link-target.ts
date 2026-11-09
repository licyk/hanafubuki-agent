/**
 * The router's destination for an in-site link: its path, and its fragment
 * apart. Given the whole `href` as `to`, the router adds its trailing slash
 * after the fragment (`docs/x#id/`), which leaves the browser at the top of
 * the page and gives giscus a pathname no page has.
 */
export function linkTarget(href: string): { to: string; hash?: string } {
  const index = href.indexOf("#");
  if (index === -1) return { to: href };
  return { to: href.slice(0, index), hash: href.slice(index + 1) };
}
