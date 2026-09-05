#!/usr/bin/env node
// Builds the multipage documentation site served from ./docs on GitHub Pages.
//
// Sources of truth stay Markdown (docs/*.md). This script renders a fixed
// page list into committed HTML so the Pages deploy (static upload of ./docs)
// needs no build step. After editing any listed guide, run:
//
//   node scripts/docs-site.mjs
//
// and commit the regenerated output. CI enforces zero drift via --check.
//
// Visual identity mirrors docs/index.html :root tokens and component grammar
// (eyebrow kickers, numbered index, hairline rules, display-dot headings).
// Keep them in sync by hand; the landing page is maintained separately
// because release scripts grep exact strings inside it.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { marked } = require("marked");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DOCS = path.join(ROOT, "docs");
const GUIDE_DIR = path.join(DOCS, "guide");
const SITE = "https://infimount.github.io/infimount";
const TODAY = new Date().toISOString().slice(0, 10);
const GITHUB_MD_PREFIX = "https://github.com/infimount/infimount/blob/main/docs/";

// Every generated guide: source file, URL slug, nav group, one-line dek.
// Deks live here (not in the Markdown) so sources stay free of site chrome.
const PAGES = [
  { file: "migration-v0.8.md", slug: "upgrade", group: "Start", dek: "Move from v0.7 to v0.8 with credentials, MCP settings, and backups intact." },
  { file: "troubleshooting.md", slug: "troubleshooting", group: "Start", dek: "Diagnose connection, credential, and launch problems with fix hints." },
  { file: "backend-capabilities.md", slug: "backends", group: "Connect storage", dek: "What each backend can and cannot do, before you rely on it." },
  { file: "oauth-drive-setup.md", slug: "oauth-drives", group: "Connect storage", dek: "Connect Google Drive and OneDrive with local loopback OAuth." },
  { file: "agent-integrations.md", slug: "agent-integrations", group: "Connect storage", dek: "Wire Claude Desktop, HTTP clients, OpenCode, and editor agents." },
  { file: "security.md", slug: "security", group: "Agents and safety", dek: "The full security model: local storage, secrets, policy, and audit." },
  { file: "agent-workspaces.md", slug: "workspaces", group: "Agents and safety", dek: "Scoped workspace folders with memory files and checkpoints." },
  { file: "mcp-client-setup.md", slug: "mcp-setup", group: "Agents and safety", dek: "Expose storage to MCP clients with explicit tools and scopes." },
  { file: "privacy.md", slug: "privacy", group: "Agents and safety", dek: "Diagnostics, product events, and telemetry consent controls." },
  { file: "recovery.md", slug: "recovery", group: "Backup and recovery", dek: "Encrypted recovery backups, restore flow, and integrity checks." },
  { file: "release-notes-0.8.0.md", slug: "release-0-8-0", group: "Releases", dek: "Local trust, recovery, and agent activation." },
  { file: "release-notes-0.7.1.md", slug: "release-0-7-1", group: "Releases", dek: "Remote file backends and mutation safety." },
  { file: "release-notes-0.7.0.md", slug: "release-0-7-0", group: "Releases", dek: "Storage expansion and validation clarity." },
  { file: "release-notes-0.8.0-rc.5.md", slug: "release-rc-5", group: "Releases", dek: "Security and release-policy candidate." },
  { file: "release-notes-0.8.0-rc.4.md", slug: "release-rc-4", group: "Releases", dek: "Release-preflight and recovery-ordering candidate." },
  { file: "release-notes-0.8.0-rc.3.md", slug: "release-rc-3", group: "Releases", dek: "Release-preflight and recovery-ordering candidate." },
  { file: "release-notes-0.8.0-rc.2.md", slug: "release-rc-2", group: "Releases", dek: "Trust and activation release candidate." },
  { file: "release-notes-0.8.0-rc.1.md", slug: "release-rc-1", group: "Releases", dek: "Trust and activation release candidate." },
  { file: "releasing.md", slug: "releasing", group: "Releases", dek: "How releases are cut, signed, and published." },
  { file: "zero-manual-product-coverage.md", slug: "coverage-notes", group: "Reference", dek: "Tracked product-coverage gaps and their status." },
];

const GROUPS = ["Start", "Connect storage", "Agents and safety", "Backup and recovery", "Releases", "Reference"];
const CSP = "default-src 'self'; img-src 'self' data: https://infimount.github.io; font-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'none'; base-uri 'self'; form-action 'none';";

const esc = (s) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function slugify(text, seen = new Map()) {
  let slug = text
    .toLowerCase()
    .replace(/`([^`]*)`/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/[\s_]+/g, "-")
    .replace(/-+/g, "-");
  if (!slug) slug = "section";
  const count = seen.get(slug) ?? 0;
  seen.set(slug, count + 1);
  return count === 0 ? slug : `${slug}-${count}`;
}

function rewriteHref(href) {
  if (!href) return href;
  if (href.startsWith("#")) return href;
  if (href.startsWith(GITHUB_MD_PREFIX)) {
    const rest = href.slice(GITHUB_MD_PREFIX.length);
    const [file, frag] = rest.split("#");
    const target = PAGES.find((p) => p.file === file);
    if (target) return `./${target.slug}.html${frag ? `#${frag}` : ""}`;
    return href;
  }
  const m = href.match(/^([A-Za-z0-9._-]+\.md)(#.*)?$/);
  if (m) {
    const target = PAGES.find((p) => p.file === m[1]);
    if (target) return `./${target.slug}.html${m[2] ?? ""}`;
  }
  return href;
}

// Render Markdown with heading ids, rewritten doc links, and TOC data.
function renderMarkdown(source) {
  const seen = new Map();
  const headings = [];
  const ext = {
    renderer: {
      heading({ tokens, depth }) {
        const text = this.parser.parseInline(tokens);
        const plain = text.replace(/<[^>]*>/g, "");
        const id = slugify(plain, seen);
        if (depth === 2 || depth === 3) headings.push({ depth, id, text: plain });
        return `<h${depth} id="${id}">${text}</h${depth}>\n`;
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens);
        const url = rewriteHref(href);
        const external = /^(https?:)?\/\//.test(url ?? "");
        const attrs = external ? ' rel="noopener" target="_blank"' : "";
        const titleAttr = title ? ` title="${esc(title)}"` : "";
        return `<a href="${esc(url ?? "")}"${titleAttr}${attrs}>${text}</a>`;
      },
      image({ href, title, text }) {
        const src = href.startsWith("./assets/") ? `../${href.slice(2)}` : href;
        const titleAttr = title ? ` title="${esc(title)}"` : "";
        return `<img src="${esc(src)}"${titleAttr} alt="${esc(text)}" loading="lazy" />`;
      },
      table({ header, rows }) {
        const head = `<thead><tr>${header}</tr></thead>`;
        const body = `<tbody>${rows.map((r) => `<tr>${r}</tr>`).join("")}</tbody>`;
        return `<div class="table-scroll" role="region" aria-label="Data table" tabindex="0"><table>${head}${body}</table></div>\n`;
      },
      code({ text, lang }) {
        const label = lang ? `<span class="code-lang">${esc(lang)}</span>` : "";
        return `<div class="code-frame">${label}<pre><code>${esc(text.replace(/\n$/, ""))}</code></pre><button class="copy-command" type="button">Copy</button></div>\n`;
      },
    },
  };
  marked.use(ext);
  const html = marked.parse(source);
  marked.use({ renderer: {} });
  return { html, headings };
}

function plainText(source, max = 480) {
  const text = source
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/!?\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/[#>*_~-]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > max ? `${text.slice(0, max - 1).trim()}…` : text;
}

function firstParagraph(source) {
  const blocks = source.split(/\n\s*\n/);
  for (const block of blocks) {
    const line = block.trim();
    if (!line || line.startsWith("#") || line.startsWith("|") || line.startsWith("```")) continue;
    const clean = plainText(line, 200);
    if (clean.length > 20) return clean;
  }
  return "";
}

function chromeHead({ title, description, canonical, extra = "" }) {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${esc(title)}</title>
    <meta name="description" content="${esc(description)}" />
    <meta name="robots" content="index,follow,max-image-preview:large" />
    <meta name="theme-color" content="#f0ede6" />
    <meta http-equiv="Content-Security-Policy" content="${CSP}" />
    <link rel="canonical" href="${canonical}" />
    <link rel="icon" href="../assets/infimount-logo.png" />
    <meta property="og:type" content="website" />
    <meta property="og:site_name" content="Infimount" />
    <meta property="og:title" content="${esc(title)}" />
    <meta property="og:description" content="${esc(description)}" />
    <meta property="og:url" content="${canonical}" />
    <meta property="og:image" content="${SITE}/assets/infimount-banner.png" />
    ${extra}
    <style>
${SHARED_CSS}
    </style>
  </head>`;
}

function topbar(active, root) {
  const link = (href, label) =>
    `<a href="${href}"${active === label ? ' aria-current="page"' : ""}>${label}</a>`;
  return `<header class="topbar">
      <div class="shell topbar-inner">
        <a class="wordmark" href="${root}index.html" aria-label="Infimount home"><span class="mark" aria-hidden="true">∞</span><span>infimount</span></a>
        <nav class="nav" aria-label="Primary navigation">
          ${link(`${root}index.html#workbench`, "Product")}
          ${link(`${root}guides.html`, "Guides")}
          ${link(`${root}index.html#install`, "Install")}
          <a class="nav-cta" href="https://github.com/infimount/infimount/releases/latest">Download</a>
        </nav>
      </div>
    </header>`;
}

function footer(root) {
  return `<footer class="footer">
      <div class="shell">
        <div class="footer-grid"><div class="footer-brand"><a class="wordmark" href="${root}index.html"><span class="mark" aria-hidden="true">∞</span><span>infimount</span></a><p>One surface for every storage. Built with Tauri, Rust, React, and Apache OpenDAL.</p></div><div><h3>Product</h3><a href="${root}index.html#workbench">Workbench</a><a href="${root}index.html#connectors">Connectors</a><a href="${root}index.html#agents">Agent access</a></div><div><h3>Guides</h3><a href="${root}guides.html">All guides</a><a href="${root}guide/security.html">Security model</a><a href="${root}guide/backends.html">Capabilities</a></div><div><h3>Project</h3><a href="https://github.com/infimount/infimount">GitHub</a><a href="https://github.com/infimount/infimount/blob/main/LICENSE">MIT license</a><a href="https://github.com/infimount/infimount/releases">Releases</a></div></div>
        <div class="footer-bottom"><span>© Infimount contributors</span><span>Storage configuration and credentials remain local by default.</span></div>
      </div>
    </footer>`;
}

function sharedScript() {
  return `<script>
      document.documentElement.classList.add("js");
      document.querySelectorAll(".copy-command").forEach((button) => button.addEventListener("click", async () => {
        const frame = button.closest(".code-frame, .command");
        const code = frame?.querySelector("code")?.textContent?.trim();
        if (!code) return;
        try { await navigator.clipboard.writeText(code); button.textContent = "Copied"; window.setTimeout(() => { button.textContent = "Copy"; }, 1600); }
        catch { button.textContent = "Copy failed"; window.setTimeout(() => { button.textContent = "Copy"; }, 1600); }
      }));
      const tocLinks = Array.from(document.querySelectorAll(".toc a"));
      if (tocLinks.length && "IntersectionObserver" in window) {
        const byId = new Map(tocLinks.map((a) => [a.getAttribute("href").slice(1), a]));
        const observer = new IntersectionObserver((entries) => {
          entries.forEach((entry) => {
            const link = byId.get(entry.target.id);
            if (link && entry.isIntersecting) {
              tocLinks.forEach((a) => a.removeAttribute("aria-current"));
              link.setAttribute("aria-current", "true");
            }
          });
        }, { rootMargin: "-20% 0px -70% 0px" });
        document.querySelectorAll(".prose h2[id], .prose h3[id]").forEach((h) => observer.observe(h));
      }
    </script>`;
}

// Identity tokens mirrored from docs/index.html. Keep in sync by hand.
const SHARED_CSS = `:root {
        --paper: #f0ede6;
        --paper-deep: #e5e0d6;
        --ink: #1d2421;
        --muted: #626861;
        --faint: #8b918a;
        --rule: #c9c8bf;
        --orange: #c96035;
        --orange-dark: #914325;
        --forest: #29433a;
        --night: #1d2824;
        --white: #f9f7f2;
        --serif: "DM Serif Display", Georgia, serif;
        --sans: "Inter Tight", Inter, ui-sans-serif, system-ui, sans-serif;
        --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
        --measure: 72rem;
        --ease: cubic-bezier(.22, 1, .36, 1);
      }
      *, *::before, *::after { box-sizing: border-box; }
      html { scroll-behavior: smooth; }
      @media (prefers-reduced-motion: reduce) {
        html { scroll-behavior: auto; }
      }
      body { margin: 0; min-width: 320px; color: var(--ink); background: var(--paper); font-family: var(--sans); font-size: 16px; line-height: 1.6; text-rendering: optimizeLegibility; }
      a { color: inherit; }
      img { max-width: 100%; }
      :focus-visible { outline: 2px solid var(--orange); outline-offset: 2px; }
      .skip-link { position: absolute; left: -999px; top: 8px; z-index: 20; padding: 10px 14px; background: var(--orange); color: var(--white); }
      .skip-link:focus { left: 8px; }
      .shell { width: min(var(--measure), calc(100% - 48px)); margin-inline: auto; }
      .eyebrow { display: flex; align-items: center; gap: 9px; margin: 0; color: var(--orange-dark); font-size: .72rem; font-weight: 800; letter-spacing: .15em; text-transform: uppercase; }
      .eyebrow::before { content: ""; width: 8px; height: 8px; border-radius: 50%; background: var(--orange); }
      .display-dot::after { content: "·"; margin-left: .08em; color: var(--orange); font-family: var(--sans); font-weight: 800; }
      .topbar { position: sticky; top: 0; z-index: 10; border-bottom: 1px solid rgba(201, 200, 191, .86); background: rgba(240, 237, 230, .92); }
      .topbar-inner { display: flex; align-items: center; justify-content: space-between; min-height: 74px; gap: 24px; }
      .wordmark { display: inline-flex; align-items: center; gap: 11px; color: var(--ink); font-size: 1.15rem; font-weight: 800; letter-spacing: -.055em; text-decoration: none; }
      .mark { display: grid; width: 27px; height: 27px; place-items: center; border: 2px solid var(--ink); border-radius: 50% 50% 50% 12%; font-size: .78rem; line-height: 1; transform: rotate(-12deg); }
      .nav { display: flex; align-items: center; gap: 22px; color: var(--muted); font-size: .84rem; font-weight: 600; }
      .nav a { text-decoration: none; }
      .nav a:hover, .nav a[aria-current="page"] { color: var(--orange-dark); }
      .nav-cta { padding: 9px 14px; border: 1px solid var(--ink); color: var(--white); background: var(--ink); }
      .guide-hero { padding: 56px 0 8px; }
      .guide-hero h1 { max-width: 16ch; margin: 20px 0 0; font-size: clamp(2.6rem, 5.5vw, 4.6rem); line-height: .95; letter-spacing: -.06em; }
      .guide-hero .dek { max-width: 44rem; margin: 18px 0 0; color: var(--muted); font-size: 1.08rem; }
      .crumbs { display: flex; gap: 10px; align-items: center; margin: 26px 0 0; padding: 0; list-style: none; color: var(--muted); font-size: .8rem; }
      .crumbs a { text-decoration: none; }
      .crumbs a:hover { color: var(--orange-dark); }
      .crumbs li + li::before { content: "/"; margin-right: 10px; color: var(--faint); }
      .guide-layout { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: clamp(32px, 5vw, 72px); align-items: start; padding: 36px 0 96px; }
      .toc { position: sticky; top: 104px; max-height: calc(100vh - 140px); overflow: auto; margin: 0; padding: 18px 0 0; border-top: 1px solid var(--ink); list-style: none; font-size: .84rem; }
      .toc li { margin: 2px 0; }
      .toc li.toc-h3 { padding-left: 14px; }
      .toc a { display: block; padding: 5px 0 5px 10px; margin-left: -12px; color: var(--muted); text-decoration: none; border-left: 2px solid transparent; }
      .toc a:hover { color: var(--ink); }
      .toc a[aria-current="true"] { color: var(--orange-dark); font-weight: 700; border-left-color: var(--orange); }
      .prose { max-width: 46rem; }
      .prose h2 { margin: 52px 0 0; padding-top: 22px; border-top: 1px solid var(--rule); font-size: clamp(1.5rem, 2.6vw, 2rem); line-height: 1.05; letter-spacing: -.035em; }
      .prose h2:first-child { margin-top: 0; padding-top: 0; border-top: 0; }
      .prose h3 { margin: 34px 0 0; font-size: 1.15rem; letter-spacing: -.02em; }
      .prose p, .prose li { color: #333936; }
      .prose > p:first-of-type { font-size: 1.08rem; }
      .prose a { color: var(--orange-dark); text-decoration-thickness: 1px; text-underline-offset: 2px; }
      .prose code { padding: 1px 5px; border: 1px solid var(--rule); background: var(--paper-deep); font: .82em var(--mono); }
      .prose pre { margin: 0; overflow: auto; border: 0; background: none; }
      .prose pre code { display: block; padding: 16px 18px; border: 0; font-size: .84rem; line-height: 1.55; background: none; }
      .code-frame { position: relative; margin: 18px 0; border: 1px solid var(--rule); background: var(--night); color: #d3dfd3; }
      .code-frame pre code { color: #d3dfd3; }
      .code-frame .code-lang { position: absolute; top: 8px; right: 86px; color: #8b978c; font-size: .68rem; font-weight: 700; letter-spacing: .1em; text-transform: uppercase; }
      .code-frame .copy-command { position: absolute; top: 6px; right: 6px; border-color: #4b5b51; color: #d3dfd3; background: transparent; }
      .code-frame .copy-command:hover { border-color: var(--orange); color: #d88a62; }
      .copy-command { padding: 6px 9px; border: 1px solid var(--rule); color: var(--ink); background: var(--paper); cursor: pointer; font: 700 .72rem var(--sans); }
      .copy-command:hover { border-color: var(--orange); color: var(--orange-dark); }
      .table-scroll { margin: 20px 0; overflow-x: auto; border-top: 1px solid var(--ink); }
      .table-scroll table { width: 100%; border-collapse: collapse; font-size: .86rem; }
      .table-scroll th { padding: 10px 12px 10px 0; text-align: left; font-size: .72rem; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); border-bottom: 1px solid var(--ink); white-space: nowrap; }
      .table-scroll td { padding: 10px 12px 10px 0; border-bottom: 1px solid var(--rule); vertical-align: top; }
      .prose blockquote { margin: 22px 0; padding: 4px 0 4px 18px; border-left: 2px solid var(--orange); color: var(--muted); }
      .prose blockquote p { margin: 6px 0; }
      .prose ul, .prose ol { padding-left: 22px; }
      .prose li + li { margin-top: 6px; }
      .prose hr { margin: 44px 0; border: 0; border-top: 1px solid var(--rule); }
      .page-foot { margin-top: 64px; border-top: 1px solid var(--ink); }
      .page-foot nav { display: flex; justify-content: space-between; gap: 20px; padding: 18px 0; }
      .page-foot a { text-decoration: none; }
      .page-foot a:hover .page-foot-label { color: var(--orange-dark); }
      .page-foot-eyebrow { display: block; color: var(--faint); font-size: .7rem; font-weight: 800; letter-spacing: .14em; text-transform: uppercase; }
      .page-foot-label { display: block; margin-top: 4px; font-size: 1.02rem; font-weight: 700; letter-spacing: -.02em; }
      .page-foot .next { text-align: right; }
      .page-meta { margin-top: 26px; color: var(--faint); font-size: .78rem; }
      .page-meta a { color: var(--muted); }
      .guides-hero { padding: 56px 0 12px; }
      .guides-hero h1 { max-width: 14ch; margin: 20px 0 0; font-size: clamp(2.6rem, 5.5vw, 4.6rem); line-height: .95; letter-spacing: -.06em; }
      .guides-hero .dek { max-width: 42rem; margin: 18px 0 0; color: var(--muted); font-size: 1.08rem; }
      .search-row { display: flex; gap: 12px; align-items: center; margin: 30px 0 8px; max-width: 34rem; }
      .search-row label { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
      .search-row input { flex: 1; min-height: 48px; padding: 10px 14px; border: 1px solid var(--ink); background: var(--white); color: var(--ink); font: 400 .95rem var(--sans); }
      .search-row kbd { padding: 4px 8px; border: 1px solid var(--rule); color: var(--muted); font: .75rem var(--mono); }
      .search-count { color: var(--faint); font-size: .8rem; min-height: 1.2em; }
      .guide-group { margin-top: 46px; }
      .guide-group > h2 { display: flex; align-items: baseline; gap: 14px; margin: 0; padding-top: 20px; border-top: 1px solid var(--ink); font-size: 1.35rem; letter-spacing: -.03em; }
      .guide-group > h2 .count { color: var(--faint); font-size: .8rem; font-weight: 600; }
      .guide-rows { margin-top: 6px; border-bottom: 1px solid var(--rule); }
      .guide-row { display: grid; grid-template-columns: 64px 1fr auto; gap: 18px; align-items: baseline; padding: 17px 0; border-top: 1px solid var(--rule); text-decoration: none; }
      .guide-row:hover .guide-row-title { color: var(--orange-dark); }
      .guide-row-index { color: var(--orange); font-size: .75rem; font-weight: 800; }
      .guide-row-title { font-size: 1.06rem; font-weight: 700; letter-spacing: -.02em; }
      .guide-row-dek { margin: 5px 0 0; color: var(--muted); font-size: .88rem; max-width: 46rem; }
      .guide-row-go { color: var(--faint); font-weight: 800; }
      .guide-row:hover .guide-row-go { color: var(--orange); }
      .footer { margin-top: 0; padding: 44px 0 26px; background: var(--ink); color: var(--white); }
      .footer-grid { display: grid; grid-template-columns: 1.7fr repeat(3, 1fr); gap: 30px; }
      .footer .wordmark { color: var(--white); }
      .footer .mark { border-color: var(--white); }
      .footer-brand p { max-width: 15rem; margin: 16px 0 0; color: #aeb8af; font-size: .84rem; }
      .footer h3 { margin: 0 0 13px; color: #d88a62; font-size: .72rem; letter-spacing: .12em; text-transform: uppercase; }
      .footer a:not(.wordmark) { display: block; width: fit-content; margin: 7px 0; color: #e7ebe4; font-size: .82rem; text-decoration: none; }
      .footer a:not(.wordmark):hover { color: #fff; text-decoration: underline; }
      .footer-bottom { display: flex; justify-content: space-between; gap: 16px; margin-top: 30px; padding-top: 16px; border-top: 1px solid #3a4640; color: #aeb8af; font-size: .76rem; }
      .empty-state { padding: 40px 0 90px; color: var(--muted); }
      @media (max-width: 900px) {
        .guide-layout { grid-template-columns: 1fr; }
        .toc { position: static; max-height: none; border-top: 0; border-bottom: 1px solid var(--rule); padding: 0 0 14px; columns: 2; }
        .toc li.toc-h3 { padding-left: 0; }
        .guide-row { grid-template-columns: 44px 1fr; }
        .guide-row-go { display: none; }
        .footer-grid { grid-template-columns: 1fr 1fr; }
      }
      @media (max-width: 650px) {
        .nav a:not(.nav-cta):not([aria-current="page"]) { display: none; }
        .toc { columns: 1; }
        .footer-bottom { flex-direction: column; gap: 4px; }
      }
      @media (prefers-reduced-motion: reduce) {
        html { scroll-behavior: auto; }
      }
      @media print {
        .topbar, .toc, .page-foot nav, .copy-command, .search-row { display: none; }
        body { background: #fff; font-size: 12px; }
        .prose { max-width: none; }
      }`;

function titleOf(source) {
  const m = source.match(/^#\s+(.+)$/m);
  return m ? m[1].replace(/\[([^\]]*)\]\([^)]*\)/g, "$1").trim() : "Guide";
}

function buildGuidePage(page, index, titles) {
  const source = fs.readFileSync(path.join(DOCS, page.file), "utf8");
  const title = titles.get(page.file);
  const description = firstParagraph(source) || page.dek;
  const { html, headings } = renderMarkdown(source);
  const url = `${SITE}/guide/${page.slug}.html`;
  const prev = PAGES[index - 1];
  const next = PAGES[index + 1];
  const toc = headings.length
    ? `<nav class="toc-wrap" aria-label="On this page"><ol class="toc">${headings
        .map(
          (h) =>
            `<li class="${h.depth === 3 ? "toc-h3" : "toc-h2"}"><a href="#${h.id}">${esc(h.text)}</a></li>`
        )
        .join("")}</ol></nav>`
    : "";
  const pager = `<div class="page-foot"><nav aria-label="Guide pages">${
    prev
      ? `<a href="./${prev.slug}.html"><span class="page-foot-eyebrow">Previous</span><span class="page-foot-label">${esc(titles.get(prev.file))}</span></a>`
      : "<span></span>"
  }${
    next
      ? `<a class="next" href="./${next.slug}.html"><span class="page-foot-eyebrow">Next</span><span class="page-foot-label">${esc(titles.get(next.file))}</span></a>`
      : "<span></span>"
  }</nav><p class="page-meta">Source: <a href="https://github.com/infimount/infimount/blob/main/docs/${page.file}" rel="noopener" target="_blank">docs/${page.file}</a> · <a href="../guides.html">All guides</a></p></div>`;
  const body = `<body>
    <a class="skip-link" href="#main">Skip to content</a>
    ${topbar("Guides", "../")}
    <main id="main">
      <div class="shell guide-hero">
        <p class="eyebrow">Guide / ${esc(page.group)}</p>
        <h1 class="display-dot">${esc(title)}</h1>
        <p class="dek">${esc(page.dek)}</p>
        <ol class="crumbs" aria-label="Breadcrumb"><li><a href="../index.html">Home</a></li><li><a href="../guides.html">Guides</a></li><li aria-current="page">${esc(title)}</li></ol>
      </div>
      <div class="shell guide-layout">
        ${toc}
        <article class="prose">${html}${pager}</article>
      </div>
    </main>
    ${footer("../")}
    ${sharedScript()}
  </body>
</html>`;
  return {
    path: path.join(GUIDE_DIR, `${page.slug}.html`),
    html: `${chromeHead({
      title: `${title} | Infimount guides`,
      description,
      canonical: url,
    })}\n${body}`,
    search: { title, dek: page.dek, url: `./guide/${page.slug}.html`, text: plainText(source, 1200) },
    sitemap: { loc: `${SITE}/guide/${page.slug}.html`, priority: "0.7" },
  };
}

function buildGuidesIndex(entries) {
  const numbered = entries.map((p, i) => ({ ...p, n: String(i + 1).padStart(2, "0") }));
  const groups = GROUPS.map((g) => ({
    name: g,
    items: numbered.filter((p) => p.group === g),
  })).filter((g) => g.items.length);
  const rows = groups
    .map(
      (g) => `<section class="guide-group" aria-labelledby="group-${esc(g.name.replace(/\s+/g, "-").toLowerCase())}">
        <h2 id="group-${esc(g.name.replace(/\s+/g, "-").toLowerCase())}">${esc(g.name)} <span class="count">${g.items.length} guide${g.items.length === 1 ? "" : "s"}</span></h2>
        <div class="guide-rows">${g.items
          .map(
            (p) => `<a class="guide-row" href="./guide/${p.slug}.html" data-title="${esc(p.title.toLowerCase())}" data-text="${esc((p.dek + " " + p.searchText).toLowerCase())}">
              <span class="guide-row-index">${p.n}</span>
              <span><span class="guide-row-title">${esc(p.title)}</span><span class="guide-row-dek">${esc(p.dek)}</span></span>
              <span class="guide-row-go" aria-hidden="true">→</span>
            </a>`
          )
          .join("")}</div>
      </section>`
    )
    .join("");
  const body = `<body>
    <a class="skip-link" href="#main">Skip to content</a>
    ${topbar("Guides", "./")}
    <main id="main">
      <div class="shell guides-hero">
        <p class="eyebrow">Field guides</p>
        <h1 class="display-dot">Read the manual.</h1>
        <p class="dek">Every guide below is the same Markdown the repository ships, rendered for reading. Sources stay in <code>docs/</code> on GitHub.</p>
        <div class="search-row">
          <label for="guide-search">Search guides</label>
          <input id="guide-search" type="search" placeholder="Search titles and topics, for example oauth or checksum" autocomplete="off" />
          <kbd>/</kbd>
        </div>
        <p class="search-count" role="status" aria-live="polite"></p>
      </div>
      <div class="shell" style="padding-bottom: 96px;">
        <div id="guide-groups">${rows}</div>
        <div class="empty-state" hidden><p>No guides match that search. Try a shorter word, or browse the full list above.</p></div>
      </div>
    </main>
    ${footer("./")}
    <script>
      document.documentElement.classList.add("js");
      const input = document.getElementById("guide-search");
      const count = document.querySelector(".search-count");
      const rows = Array.from(document.querySelectorAll(".guide-row"));
      const groups = Array.from(document.querySelectorAll(".guide-group"));
      const params = new URLSearchParams(location.search);
      if (params.get("q")) input.value = params.get("q");
      function apply() {
        const q = input.value.trim().toLowerCase();
        let shown = 0;
        rows.forEach((row) => {
          const hit = !q || row.dataset.title.includes(q) || row.dataset.text.includes(q);
          row.style.display = hit ? "" : "none";
          if (hit) shown += 1;
        });
        groups.forEach((group) => {
          const visible = Array.from(group.querySelectorAll(".guide-row")).some((r) => r.style.display !== "none");
          group.style.display = visible ? "" : "none";
        });
        document.querySelector(".empty-state").hidden = shown !== 0;
        count.textContent = q ? (shown === 1 ? "1 guide" : shown + " guides") + " match \\"" + input.value.trim() + "\\"" : rows.length + " guides total";
      }
      input.addEventListener("input", apply);
      document.addEventListener("keydown", (e) => {
        if (e.key === "/" && document.activeElement !== input) { e.preventDefault(); input.focus(); }
      });
      apply();
    </script>
  </body>
</html>`;
  return `${chromeHead({
    title: "Guides | Infimount",
    description: "Field guides for installing, connecting, securing, and recovering Infimount storage.",
    canonical: `${SITE}/guides.html`,
  })}\n${body}`;
}

function buildSitemap(entries) {
  const urls = [
    { loc: `${SITE}/`, priority: "1.0", changefreq: "weekly" },
    { loc: `${SITE}/guides.html`, priority: "0.9", changefreq: "weekly" },
    ...entries.map((e) => ({ ...e.sitemap, changefreq: "monthly" })),
  ];
  const items = urls
    .map(
      (u) => `  <url>
    <loc>${u.loc}</loc>
    <lastmod>${TODAY}</lastmod>
${u.changefreq ? `    <changefreq>${u.changefreq}</changefreq>\n` : ""}    <priority>${u.priority}</priority>
  </url>`
    )
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${items}\n</urlset>\n`;
}

function main() {
  const checkOnly = process.argv.includes("--check");
  const out = new Map();
  const titles = new Map(PAGES.map((p) => [p.file, titleOf(fs.readFileSync(path.join(DOCS, p.file), "utf8"))]));
  const entries = PAGES.map((page, i) => {
    const built = buildGuidePage(page, i, titles);
    out.set(built.path, built.html);
    return { ...page, title: titles.get(page.file), searchText: built.search.text, sitemap: built.sitemap };
  });
  const guides = buildGuidesIndex(entries);
  out.set(path.join(DOCS, "guides.html"), guides);
  const search = entries.map((e) => ({
    title: titles.get(e.file),
    dek: e.dek,
    url: `./guide/${e.slug}.html`,
    text: plainText(fs.readFileSync(path.join(DOCS, e.file), "utf8"), 1200),
  }));
  out.set(path.join(DOCS, "search.json"), `${JSON.stringify(search, null, 2)}\n`);
  out.set(path.join(DOCS, "sitemap.xml"), buildSitemap(entries));

  if (checkOnly) {
    const dirty = [];
    for (const [file, expected] of out) {
      const current = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : null;
      if (current !== expected) dirty.push(path.relative(ROOT, file));
    }
    if (dirty.length) {
      console.error(`docs-site drift detected in: ${dirty.join(", ")}\nRun node scripts/docs-site.mjs and commit the output.`);
      process.exit(1);
    }
    console.log(`docs-site check passed (${out.size} files in sync).`);
    return;
  }
  fs.mkdirSync(GUIDE_DIR, { recursive: true });
  for (const [file, content] of out) fs.writeFileSync(file, content);
  console.log(`docs-site built ${out.size} files (${entries.length} guides).`);
}

main();
