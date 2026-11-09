import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Contract tests for the motion system.
 *
 * A page once put entrance timing and hover timing on the same `transition`
 * property of the same element. A stagger `transition-delay` declared at
 * higher specificity then outranked each card's own transition, so hover
 * lagged on entry and cut on exit. These tests make that shape fail the build
 * rather than fail in a browser.
 */

const STYLE_DIR = fileURLToPath(new URL("../styles", import.meta.url));
const COMPONENT_DIR = fileURLToPath(new URL("../components", import.meta.url));
const ROUTE_DIR = fileURLToPath(new URL("../routes", import.meta.url));

/** The interaction channel — must never carry a delay. */
const INTERACTION_CHANNEL = /page-interactive/;

interface CssRule {
  selector: string;
  body: string;
  atRules: readonly string[];
  source: string;
}

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/**
 * Minimal flat-CSS reader. These stylesheets deliberately avoid CSS nesting,
 * so tracking at-rule preludes on a stack is enough to attribute every rule.
 */
function parseRules(css: string, source: string): CssRule[] {
  const rules: CssRule[] = [];
  const atRules: string[] = [];
  let prelude = "";

  for (let index = 0; index < css.length; index += 1) {
    const character = css[index];

    if (character === "{") {
      const trimmed = prelude.trim();
      prelude = "";

      if (trimmed.startsWith("@")) {
        atRules.push(trimmed);
        continue;
      }

      let depth = 1;
      let cursor = index + 1;
      while (cursor < css.length && depth > 0) {
        if (css[cursor] === "{") depth += 1;
        if (css[cursor] === "}") depth -= 1;
        if (depth > 0) cursor += 1;
      }

      rules.push({
        selector: trimmed,
        body: css.slice(index + 1, cursor),
        atRules: [...atRules],
        source,
      });
      index = cursor;
      continue;
    }

    if (character === "}") {
      atRules.pop();
      prelude = "";
      continue;
    }

    prelude += character;
  }

  return rules;
}

function walk(directory: string, extension: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return walk(path, extension);
    return entry.name.endsWith(extension) ? [path] : [];
  });
}

/** Every stylesheet the site ships, so a new one cannot escape the contract. */
function readStylesheets(): CssRule[] {
  return walk(STYLE_DIR, ".css").flatMap((path) =>
    parseRules(stripComments(readFileSync(path, "utf8")), path),
  );
}

/**
 * Classes that share a `className` with an interaction-channel marker.
 *
 * Deriving this from the markup means the hover checks below stay correct as
 * components are added, instead of drifting against a hand-maintained list.
 */
function readInteractionClasses(): { subjects: Set<string>; parts: Set<string> } {
  const subjects = new Set<string>();
  const parts = new Set<string>();

  const sources = [...walk(COMPONENT_DIR, ".tsx"), ...walk(ROUTE_DIR, ".tsx")];

  for (const path of sources) {
    const code = readFileSync(path, "utf8");
    for (const match of code.match(/className=(?:"[^"]*"|\{`[^`]*`\})/g) ?? []) {
      const classes = match
        .replace(/^className=\{?`?"?|"?`?\}?$/g, "")
        .replace(/\$\{[^}]*\}/g, " ")
        .split(/\s+/)
        .filter(Boolean);

      const target = classes.includes("page-interactive")
        ? subjects
        : classes.includes("page-interactive-part")
          ? parts
          : undefined;
      if (!target) continue;
      for (const name of classes) target.add(name);
    }
  }

  return { subjects, parts };
}

/**
 * Classes given a `transition` by a base (non-hover) rule anywhere in the
 * site's CSS. Not scoped per file: the cascade is not, either.
 */
function transitionedClasses(): Set<string> {
  const names = new Set<string>();

  for (const rule of rules) {
    if (rule.selector.includes(":hover")) continue;
    if (!/(^|[;{\s])transition\s*:/.test(rule.body)) continue;
    for (const name of rule.selector.match(/\.[a-zA-Z0-9_-]+/g) ?? []) names.add(name.slice(1));
  }

  return names;
}

/** True when a declaration block sets a non-zero transition delay. */
function declaresTransitionDelay(body: string): boolean {
  if (/transition-delay\s*:/.test(body)) return true;

  // In the `transition` shorthand a second <time> value is the delay.
  const shorthand = body.match(/transition\s*:([^;}]*)/g) ?? [];
  return shorthand.some((declaration) =>
    declaration
      .split(",")
      .some(
        (part) => (part.match(/(?:\d*\.?\d+m?s\b|calc\([^)]*stagger[^)]*\))/g) ?? []).length > 1,
      ),
  );
}

const rules = readStylesheets();
const interactionClasses = readInteractionClasses();

describe("motion channel separation", () => {
  it("finds rules to check", () => {
    expect(rules.length).toBeGreaterThan(40);
  });

  it("declares no transition delay anywhere", () => {
    // Entrances are keyframe animations; a transition delay would reach hover.
    const offenders = rules
      .filter((rule) => declaresTransitionDelay(rule.body))
      .map((rule) => `${rule.source}: ${rule.selector}`);

    expect(offenders).toEqual([]);
  });

  it("never puts a transition delay on the interaction channel", () => {
    const offenders = rules
      .filter((rule) => INTERACTION_CHANNEL.test(rule.selector))
      .filter((rule) => declaresTransitionDelay(rule.body))
      .map((rule) => `${rule.source}: ${rule.selector}`);

    expect(offenders).toEqual([]);
  });

  it("never gives an entrance animation to an element that also hovers", () => {
    const interactive = new Set([...interactionClasses.subjects, ...interactionClasses.parts]);
    const offenders = rules
      .filter((rule) => /(^|[;{\s])animation(-delay)?\s*:/.test(rule.body))
      .filter(
        (rule) =>
          rule.selector.includes(":hover") ||
          (rule.selector.match(/\.[a-zA-Z0-9_-]+/g) ?? []).some((name) =>
            interactive.has(name.slice(1)),
          ),
      )
      .map((rule) => `${rule.source}: ${rule.selector}`);

    expect(offenders).toEqual([]);
  });
});

describe("hover safety", () => {
  it("guards every hover rule behind (any-hover: hover)", () => {
    const offenders = rules
      .filter((rule) => rule.selector.includes(":hover"))
      .filter(
        (rule) => !rule.atRules.some((atRule) => /\(\s*any-hover\s*:\s*hover\s*\)/.test(atRule)),
      )
      .map((rule) => `${rule.source}: ${rule.selector}`);

    expect(offenders).toEqual([]);
  });

  it("never gates hover on the primary-pointer `hover` feature", () => {
    // `(hover: hover)` is false on a touch laptop that also has a mouse.
    const offenders = rules
      .flatMap((rule) => rule.atRules)
      .filter((atRule) => /(?<!any-)hover\s*:\s*hover/.test(atRule));

    expect(offenders).toEqual([]);
  });

  it("gives every hovered element a base transition to return along", () => {
    // A hover rule that changes an animatable property must reach the
    // interaction channel: some class in its selector either carries
    // `.page-interactive`/`.page-interactive-part` in the markup, or is given a
    // `transition` by a base rule in the same stylesheet. Otherwise the hover
    // applies instantly and, worse, disappears instantly.
    const animated = /transform|opacity|color|box-shadow|background|border-color|max-width|height/;
    const transitioned = transitionedClasses();

    const offenders = rules
      .filter((rule) => rule.selector.includes(":hover"))
      .filter((rule) => animated.test(rule.body))
      .filter((rule) => {
        if (INTERACTION_CHANNEL.test(rule.selector)) return false;

        return !(rule.selector.match(/\.[a-zA-Z0-9_-]+/g) ?? []).some((name) => {
          const bare = name.slice(1);
          return (
            interactionClasses.subjects.has(bare) ||
            interactionClasses.parts.has(bare) ||
            transitioned.has(bare)
          );
        });
      })
      .map((rule) => `${rule.source}: ${rule.selector}`);

    expect(offenders).toEqual([]);
  });

  it("derives interaction classes from real markup", () => {
    // Guards the check above from silently passing on an empty set.
    expect(interactionClasses.subjects.size).toBeGreaterThan(3);
    expect(interactionClasses.subjects.has("home-primary-link")).toBe(true);
    expect(interactionClasses.parts.size).toBeGreaterThan(0);
  });
});
