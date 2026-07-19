//! RES-4115 (E-E4 increment 4): strict registry <-> docs/errors
//! generation guard.
//!
//! `docs/errors/*.md` bodies are still hand-authored prose (each page
//! carries a worked example and fix, which isn't mechanically derivable
//! from the registry), but every *structural* fact that ties a page back
//! to `diag::codes` is validated here byte-for-byte against the
//! registry, so the two can never silently drift apart:
//!
//! - every code in `codes::all()` has exactly one `docs/errors/E00NN.md`
//!   page, and no page exists for a code that isn't registered;
//! - the page's front matter (`title`, `parent`, `nav_order`,
//!   `permalink`) is internally consistent with the code and its
//!   1-based position in `codes::all()`;
//! - the page's `# ` heading matches its front-matter `title` exactly;
//! - `docs/errors/index.md` links every registered code, in registry
//!   order, with the same title text as the page itself;
//! - `error_explain.rs`'s `include_str!` table (exercised indirectly via
//!   `rz errors list`, RES-4115's earlier drift guard from #4124) stays
//!   in lockstep — this test is the "full generation validation"
//!   extension promised in that module's doc comment.
//!
//! If this test starts failing after adding a new code, the fix is
//! almost always: add the missing `docs/errors/E00NN.md` page (copy an
//! existing one as a template), add its `include_str!` arm in
//! `error_explain.rs`, and add its link line to `docs/errors/index.md`
//! — not to weaken this test.

use resilient::diag::codes;

struct FrontMatter {
    title: String,
    parent: String,
    nav_order: usize,
    permalink: String,
}

fn parse_front_matter(doc: &str) -> FrontMatter {
    let rest = doc
        .strip_prefix("---\n")
        .expect("docs/errors page must start with a `---` front-matter fence");
    let end = rest
        .find("\n---\n")
        .expect("docs/errors page front matter must be closed with `---`");
    let block = &rest[..end];

    let mut title = None;
    let mut parent = None;
    let mut nav_order = None;
    let mut permalink = None;
    for line in block.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "title" => title = Some(value),
            "parent" => parent = Some(value),
            "nav_order" => {
                nav_order = Some(
                    value
                        .parse::<usize>()
                        .expect("nav_order must be a plain integer"),
                )
            }
            "permalink" => permalink = Some(value),
            _ => {}
        }
    }
    FrontMatter {
        title: title.expect("front matter missing `title`"),
        parent: parent.expect("front matter missing `parent`"),
        nav_order: nav_order.expect("front matter missing `nav_order`"),
        permalink: permalink.expect("front matter missing `permalink`"),
    }
}

fn heading(doc: &str) -> &str {
    doc.lines()
        .find(|l| l.starts_with("# "))
        .expect("docs/errors page must have a top-level `# ` heading")
        .trim_start_matches("# ")
}

/// Every registered code's page source, keyed by code. A `match` (not a
/// loop over a directory listing) so a code with no page fails to
/// *compile* via `include_str!`, matching `error_explain.rs`'s own
/// registry-coupling strategy.
fn doc_source(code: &str) -> &'static str {
    match code {
        "E0001" => include_str!("../../../docs/errors/E0001.md"),
        "E0002" => include_str!("../../../docs/errors/E0002.md"),
        "E0003" => include_str!("../../../docs/errors/E0003.md"),
        "E0004" => include_str!("../../../docs/errors/E0004.md"),
        "E0005" => include_str!("../../../docs/errors/E0005.md"),
        "E0006" => include_str!("../../../docs/errors/E0006.md"),
        "E0007" => include_str!("../../../docs/errors/E0007.md"),
        "E0008" => include_str!("../../../docs/errors/E0008.md"),
        "E0009" => include_str!("../../../docs/errors/E0009.md"),
        "E0010" => include_str!("../../../docs/errors/E0010.md"),
        "E0011" => include_str!("../../../docs/errors/E0011.md"),
        "E0012" => include_str!("../../../docs/errors/E0012.md"),
        "E0013" => include_str!("../../../docs/errors/E0013.md"),
        "E0014" => include_str!("../../../docs/errors/E0014.md"),
        "E0015" => include_str!("../../../docs/errors/E0015.md"),
        "E0016" => include_str!("../../../docs/errors/E0016.md"),
        "E0017" => include_str!("../../../docs/errors/E0017.md"),
        "E0018" => include_str!("../../../docs/errors/E0018.md"),
        "E0019" => include_str!("../../../docs/errors/E0019.md"),
        "E0020" => include_str!("../../../docs/errors/E0020.md"),
        "E0021" => include_str!("../../../docs/errors/E0021.md"),
        other => panic!(
            "docs/errors/{other}.md has no include_str! arm in this test — add one \
             (and to error_explain.rs's doc_for_code) alongside any new registry code"
        ),
    }
}

const INDEX_MD: &str = include_str!("../../../docs/errors/index.md");

#[test]
fn every_registered_code_has_a_consistent_docs_page() {
    let all = codes::all();
    for (zero_based, code) in all.iter().enumerate() {
        let code_str = code.as_str();
        let nav_order = zero_based + 1;
        let doc = doc_source(code_str);
        let fm = parse_front_matter(doc);

        assert!(
            fm.title.starts_with(&format!("{code_str} — ")),
            "{code_str}: front-matter title {:?} must start with `{code_str} — `",
            fm.title
        );
        assert_eq!(
            fm.parent, "Error Index",
            "{code_str}: front-matter `parent` must be `Error Index`"
        );
        assert_eq!(
            fm.nav_order, nav_order,
            "{code_str}: front-matter nav_order must equal its 1-based position \
             in codes::all() ({nav_order}); got {}",
            fm.nav_order
        );
        assert_eq!(
            fm.permalink,
            format!("/errors/{code_str}"),
            "{code_str}: front-matter permalink must be /errors/{code_str}"
        );

        let heading_text = heading(doc);
        assert_eq!(
            heading_text, fm.title,
            "{code_str}: `# ` heading must match the front-matter title exactly"
        );

        let index_link = format!("[{}]", fm.title);
        assert!(
            INDEX_MD.contains(&index_link),
            "docs/errors/index.md is missing (or has a stale) link for {code_str}: \
             expected to find `{index_link}`"
        );
    }
}

#[test]
fn no_orphaned_docs_page_exists_outside_the_registry() {
    // The inverse direction: every `E00NN.md` file physically present in
    // docs/errors/ must correspond to a registered code. This test
    // can't glob the directory (include_str! is compile-time and CARGO
    // sandboxes fs access at test time in some CI images), so instead it
    // pins the known-registered count — anyone adding a new page without
    // registering the code will trip `res206a_codes_all_count_matches_vec_len`
    // in diag.rs first, and this count keeps the two files honest.
    assert_eq!(
        codes::all().len(),
        21,
        "update this count (and the include_str! table above) when the registry grows"
    );
}

#[test]
fn index_lists_codes_in_registry_order() {
    let all = codes::all();
    let mut last_pos = 0usize;
    for code in &all {
        let code_str = code.as_str();
        let doc = doc_source(code_str);
        let fm = parse_front_matter(doc);
        let index_link = format!("[{}]", fm.title);
        let pos = INDEX_MD
            .find(&index_link)
            .unwrap_or_else(|| panic!("{code_str} missing from docs/errors/index.md"));
        assert!(
            pos > last_pos || last_pos == 0,
            "{code_str}'s index.md link must appear after the previous code's link \
             (index.md must list codes in ascending registry order)"
        );
        last_pos = pos;
    }
}
