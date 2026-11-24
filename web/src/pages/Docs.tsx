import { useEffect, useMemo, useRef, useState } from "react";
import { Link, NavLink, useLocation, useNavigate, useParams } from "react-router-dom";
import { SearchIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { DOC_SECTIONS, DOCS, docBySlug, docPath, render, search } from "../lib/docs";

/**
 * The documentation, laid out like Datadog's: the pages by section on the
 * left, the page in the middle, its headings on the right, a search over
 * every page.
 */
export function Docs() {
  const { slug = "" } = useParams();
  const page = docBySlug(slug);
  const [query, setQuery] = useState("");
  const [navOpen, setNavOpen] = useState(false);
  const { pathname } = useLocation();
  useEffect(() => {
    setQuery("");
    setNavOpen(false);
  }, [pathname]);

  return (
    <>
      <PageHeader title="Documentation">
        <button className="btn small docs-nav-toggle" onClick={() => setNavOpen(!navOpen)}>
          {navOpen ? "Hide pages" : "All pages"}
        </button>
      </PageHeader>
      <div className="docs">
        <aside className={`docs-nav${navOpen ? " open" : ""}`}>
          <label className="docs-search">
            <SearchIcon />
            <input
              className="input"
              placeholder="Search the documentation"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <DocsNav current={slug} />
        </aside>
        <div className="docs-main page-scroll" id="docs-scroll">
          {query.trim() ? (
            <Results query={query} />
          ) : page ? (
            <Article key={page.slug} slug={page.slug} />
          ) : (
            <div className="empty-state">
              <h3>No such page</h3>
              <Link to="/docs">Back to the documentation</Link>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

function DocsNav({ current }: { current: string }) {
  const currentSection = docBySlug(current)?.section;
  const sections = DOC_SECTIONS.map((s) => ({
    title: s,
    pages: DOCS.filter((d) => d.section === s && d.slug),
  })).filter((s) => s.pages.length);
  const [closed, setClosed] = useState<Set<string>>(new Set());
  return (
    <nav>
      <NavLink to="/docs" end className="docs-nav-home">
        Overview
      </NavLink>
      {sections.map((s) => {
        const open = !closed.has(s.title) || s.title === currentSection;
        return (
          <div key={s.title} className="docs-nav-section">
            <button
              className={`docs-nav-title${open ? " open" : ""}`}
              onClick={() => {
                const next = new Set(closed);
                if (next.has(s.title)) next.delete(s.title);
                else next.add(s.title);
                setClosed(next);
              }}
            >
              {s.title}
            </button>
            {open &&
              s.pages.map((p) => (
                <NavLink key={p.slug} to={docPath(p.slug)}>
                  {p.title}
                </NavLink>
              ))}
          </div>
        );
      })}
    </nav>
  );
}

function Results({ query }: { query: string }) {
  const hits = useMemo(() => search(query), [query]);
  return (
    <div className="docs-results">
      <h1>
        {hits.length} result{hits.length === 1 ? "" : "s"} for “{query.trim()}”
      </h1>
      {hits.map((h) => (
        <Link key={h.page.slug} to={docPath(h.page.slug)} className="docs-hit">
          <span className="docs-hit-section">{h.page.section}</span>
          <span className="docs-hit-title">{h.page.title}</span>
          {h.snippet && <span className="docs-hit-snippet">{h.snippet}</span>}
        </Link>
      ))}
      {!hits.length && <p className="muted">No page mentions every word of it.</p>}
    </div>
  );
}

function Article({ slug }: { slug: string }) {
  const page = docBySlug(slug)!;
  const { html, toc } = useMemo(() => render(page), [page]);
  const ref = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();
  const { hash } = useLocation();
  const [active, setActive] = useState("");
  const i = DOCS.indexOf(page);
  const prev = DOCS[i - 1];
  const next = DOCS[i + 1];

  // Open at the heading the link names, or at the top.
  useEffect(() => {
    const scroller = document.getElementById("docs-scroll");
    const target = hash ? document.getElementById(decodeURIComponent(hash.slice(1))) : null;
    if (target) target.scrollIntoView();
    else scroller?.scrollTo(0, 0);
  }, [slug, hash]);

  // The heading being read, for the table of contents.
  useEffect(() => {
    const scroller = document.getElementById("docs-scroll");
    if (!scroller || !toc.length) return;
    const onScroll = () => {
      const top = scroller.getBoundingClientRect().top + 80;
      let current = toc[0].id;
      for (const t of toc) {
        const el = document.getElementById(t.id);
        if (el && el.getBoundingClientRect().top <= top) current = t.id;
      }
      setActive(current);
    };
    onScroll();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, [toc]);

  const onClick = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    const copy = el.closest(".doc-copy");
    if (copy) {
      const code = copy.closest(".doc-code")?.querySelector("code")?.textContent ?? "";
      navigator.clipboard?.writeText(code).then(() => {
        copy.textContent = "Copied";
        setTimeout(() => (copy.textContent = "Copy"), 1500);
      });
      return;
    }
    const a = el.closest("a");
    const href = a?.getAttribute("href");
    if (!a || !href || a.target === "_blank" || e.metaKey || e.ctrlKey) return;
    if (href.startsWith("/") || href.startsWith("#")) {
      e.preventDefault();
      navigate(href.startsWith("#") ? { hash: href } : href);
    }
  };

  return (
    <div className="docs-page">
      <article className="docs-article">
        <div className="docs-crumbs">
          <Link to="/docs">Docs</Link>
          {page.slug && (
            <>
              <span>/</span>
              <span>{page.section}</span>
            </>
          )}
        </div>
        <div className="docs-title">
          <h1>{page.title}</h1>
          {page.app && (
            <Link className="btn small" to={page.app}>
              Open in Robine Dog
            </Link>
          )}
        </div>
        {page.description && <p className="docs-lead">{page.description}</p>}
        <div className="doc-body" ref={ref} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />
        <div className="docs-pager">
          {prev ? (
            <Link to={docPath(prev.slug)}>
              <span className="muted">Previous</span>
              {prev.title}
            </Link>
          ) : (
            <span />
          )}
          {next && (
            <Link to={docPath(next.slug)} className="next">
              <span className="muted">Next</span>
              {next.title}
            </Link>
          )}
        </div>
      </article>
      {toc.length > 1 && (
        <nav className="docs-toc">
          <div className="docs-toc-title">On this page</div>
          {toc.map((t) => (
            <a
              key={t.id}
              href={`#${t.id}`}
              className={`${t.depth === 3 ? "sub " : ""}${active === t.id ? "active" : ""}`}
              onClick={(e) => {
                e.preventDefault();
                navigate({ hash: t.id }, { replace: true });
                document.getElementById(t.id)?.scrollIntoView({ behavior: "smooth" });
              }}
            >
              {t.text}
            </a>
          ))}
        </nav>
      )}
    </div>
  );
}
