import { createFileRoute } from "@tanstack/react-router";
import Link from "fumadocs-core/link";
import { HomeLayout } from "fumadocs-ui/layouts/home";
import { ArrowDownToLine, ArrowRight, ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";

import CommandBlock from "@/components/command-block";
import { useDocumentMetadata } from "@/lib/document-metadata";
import { DEFAULT_LOCALE } from "@/lib/i18n";
import { CARGO_INSTALL, POWERSHELL_INSTALL, SHELL_INSTALL, SOURCE_INSTALL } from "@/lib/install";
import { baseOptions } from "@/lib/layout.shared";
import { fetchRelease, RELEASES_URL, type ReleaseResult } from "@/lib/releases";
import { getSiteCopy, useSiteCopy, useSiteLocale } from "@/lib/site-copy";

export const Route = createFileRoute("/download")({
  head: () => {
    const text = getSiteCopy(DEFAULT_LOCALE).download;
    return { meta: [{ title: text.title }, { name: "description", content: text.description }] };
  },
  component: DownloadPage,
});

function DownloadPage() {
  const locale = useSiteLocale();
  const text = useSiteCopy().download;
  const [result, setResult] = useState<ReleaseResult | { status: "loading" | "error" }>({
    status: "loading",
  });
  const [attempt, setAttempt] = useState(0);
  useDocumentMetadata(text);

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setResult({ status: "loading" });
    const timeout = window.setTimeout(() => controller.abort(), 12_000);
    fetchRelease(controller.signal)
      .then((value) => {
        if (active) setResult(value);
      })
      .catch(() => {
        if (active) setResult({ status: "error" });
      })
      .finally(() => window.clearTimeout(timeout));
    return () => {
      active = false;
      window.clearTimeout(timeout);
      controller.abort();
    };
  }, [attempt]);

  return (
    <HomeLayout {...baseOptions(locale)} className="paged">
      <div className="download-page">
        <header className="download-header">
          <span className="download-eyebrow">Hanafubuki Agent</span>
          <h1>{text.title}</h1>
          <p>{text.description}</p>
          <nav className="download-nav" aria-label={text.methods}>
            <Link className="page-interactive" href="#scripts">
              {text.scripts}
            </Link>
            <Link className="page-interactive" href="#binaries">
              {text.binaries}
            </Link>
            <Link className="page-interactive" href="#cargo">
              Cargo
            </Link>
            <Link className="page-interactive" href="#source">
              {text.source}
            </Link>
          </nav>
        </header>

        <section id="scripts" className="download-card" aria-labelledby="scripts-title">
          <h2 id="scripts-title">{text.scripts}</h2>
          <p>{text.scriptDescription}</p>
          <h3>Windows</h3>
          <CommandBlock command={POWERSHELL_INSTALL} label="PowerShell" />
          <h3>Linux / macOS</h3>
          <CommandBlock command={SHELL_INSTALL} label="Bash" />
          <p>{text.shellNote}</p>
          <Link className="download-text-link" href="/docs/installation#install-script">
            {text.options}
            <ArrowRight size={16} aria-hidden="true" />
          </Link>
        </section>

        <section id="binaries" className="download-card" aria-labelledby="binaries-title">
          <h2 id="binaries-title">{text.binaries}</h2>
          <p>{text.binaryDescription}</p>
          <div aria-live="polite">
            {result.status === "available" ? (
              <>
                <a className="download-version" href={result.url}>
                  {result.version}
                </a>
                <div className="download-grid">
                  {result.assets.map((asset) => (
                    <article className="download-asset" key={asset.name}>
                      <h3>{asset.platform}</h3>
                      <p>{asset.arch}</p>
                      <a
                        className="home-primary-link page-interactive"
                        href={asset.url}
                        aria-label={`${text.get} ${asset.platform} ${asset.arch} ${asset.extension}`}
                      >
                        <ArrowDownToLine size={16} aria-hidden="true" />
                        {asset.extension === "zip" ? "ZIP" : "TAR.GZ"}
                      </a>
                      {asset.checksumUrl ? (
                        <a
                          className="download-checksum"
                          href={asset.checksumUrl}
                          aria-label={`${asset.platform} ${asset.arch} SHA-256`}
                        >
                          SHA-256
                        </a>
                      ) : null}
                    </article>
                  ))}
                </div>
              </>
            ) : (
              <p className="download-status">
                {result.status === "loading"
                  ? text.loading
                  : result.status === "error"
                    ? text.error
                    : text.unavailable}
              </p>
            )}
          </div>
          <div className="download-actions">
            {result.status === "error" || result.status === "unavailable" ? (
              <button
                type="button"
                className="home-secondary-link page-interactive"
                onClick={() => setAttempt((value) => value + 1)}
              >
                {text.retry}
              </button>
            ) : null}
            <a className="home-secondary-link page-interactive" href={RELEASES_URL}>
              {text.releases}
              <ExternalLink size={16} aria-hidden="true" />
            </a>
          </div>
          <p>{text.platformNote}</p>
        </section>

        <div className="download-methods">
          <section id="cargo" className="download-card" aria-labelledby="cargo-title">
            <h2 id="cargo-title">{text.cargo}</h2>
            <p>{text.cargoDescription}</p>
            <CommandBlock command={CARGO_INSTALL} label="Cargo" />
          </section>
          <section id="source" className="download-card" aria-labelledby="source-title">
            <h2 id="source-title">{text.source}</h2>
            <p>{text.sourceDescription}</p>
            <CommandBlock command={SOURCE_INSTALL} label="Git + Cargo" />
          </section>
        </div>
        <footer className="download-footer">
          <p>{text.update}</p>
          <div className="download-actions">
            <Link className="home-primary-link page-interactive" href="/docs/quick-start">
              {text.start}
              <ArrowRight size={16} aria-hidden="true" />
            </Link>
            <Link className="home-secondary-link page-interactive" href="/docs/installation">
              {text.guide}
            </Link>
          </div>
        </footer>
      </div>
    </HomeLayout>
  );
}
