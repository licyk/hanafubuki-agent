import { createFileRoute } from "@tanstack/react-router";
import Link from "fumadocs-core/link";
import { HomeLayout } from "fumadocs-ui/layouts/home";
import { ArrowRight } from "lucide-react";

import { useDocumentMetadata } from "@/lib/document-metadata";
import { DEFAULT_LOCALE } from "@/lib/i18n";
import { baseOptions, REPOSITORY_URL, SITE_NAME } from "@/lib/layout.shared";
import { getSiteCopy, useSiteCopy, useSiteLocale } from "@/lib/site-copy";

const INSTALL_COMMAND = "cargo install hanafubuki-agent-cli";
const AUTHOR = "licyk";

export const Route = createFileRoute("/")({
  head: () => {
    const copy = getSiteCopy(DEFAULT_LOCALE).home;
    return {
      meta: [{ title: copy.title }, { name: "description", content: copy.description }],
    };
  },
  component: Home,
});

function Home() {
  const locale = useSiteLocale();
  const copy = useSiteCopy().home;

  useDocumentMetadata(copy);

  return (
    <HomeLayout {...baseOptions(locale)} className="paged">
      <section aria-labelledby="home-title" className="page-section">
        <div className="page-section-backdrop home-hero-backdrop" aria-hidden="true" />
        <div className="page-shell">
          <div className="home-hero">
            <h1 id="home-title">{SITE_NAME}</h1>
            <p>{copy.description}</p>
            <code aria-label={copy.install} className="home-install">
              <span aria-hidden="true">$</span>
              {INSTALL_COMMAND}
            </code>
            <div className="home-actions">
              <Link className="home-primary-link page-interactive" href="/docs/installation">
                {copy.installation}
                <ArrowRight className="page-interactive-part" aria-hidden="true" />
              </Link>
              <Link className="home-secondary-link page-interactive" href="/docs/quick-start">
                {copy.quickStart}
              </Link>
              <Link className="home-secondary-link page-interactive" href={REPOSITORY_URL}>
                {copy.github}
              </Link>
            </div>
          </div>
          <footer className="home-footer">
            {/*
             * The page is prerendered, so the year is the build year until the
             * document is served again. Hydration must not warn about that.
             */}
            <p suppressHydrationWarning>
              © {new Date().getFullYear()} {AUTHOR} · GPL-3.0-only
            </p>
            <nav aria-label={copy.footer.label}>
              <Link className="home-footer-link page-interactive" href="/docs">
                {copy.footer.docs}
              </Link>
              <Link className="home-footer-link page-interactive" href={REPOSITORY_URL}>
                {copy.footer.github}
              </Link>
            </nav>
          </footer>
        </div>
      </section>
    </HomeLayout>
  );
}
