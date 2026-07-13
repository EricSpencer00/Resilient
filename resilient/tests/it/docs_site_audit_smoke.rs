//! RES-3835: website audit keeps recent public docs reachable from the site.

struct SiteDoc {
    label: &'static str,
    doc: &'static str,
    permalink: &'static str,
}

const RECENT_PUBLIC_DOCS: &[SiteDoc] = &[
    SiteDoc {
        label: "Verification model",
        doc: include_str!("../../../docs/VERIFICATION_MODEL.md"),
        permalink: "/verification-model",
    },
    SiteDoc {
        label: "Failure model",
        doc: include_str!("../../../docs/FAILURE_MODEL.md"),
        permalink: "/failure-model",
    },
    SiteDoc {
        label: "Backend architecture",
        doc: include_str!("../../../docs/BACKENDS.md"),
        permalink: "/backends",
    },
    SiteDoc {
        label: "Tooling quality",
        doc: include_str!("../../../docs/TOOLING_QUALITY.md"),
        permalink: "/tooling-quality",
    },
    SiteDoc {
        label: "Stability policy",
        doc: include_str!("../../../docs/STABILITY_POLICY.md"),
        permalink: "/stability-policy",
    },
    SiteDoc {
        label: "Standard library portability",
        doc: include_str!("../../../docs/STDLIB_PORTABILITY.md"),
        permalink: "/stdlib-portability",
    },
    SiteDoc {
        label: "Module system",
        doc: include_str!("../../../docs/MODULE_SYSTEM.md"),
        permalink: "/module-system",
    },
    SiteDoc {
        label: "Type system roadmap",
        doc: include_str!("../../../docs/TYPE_SYSTEM_ROADMAP.md"),
        permalink: "/type-system-roadmap",
    },
    SiteDoc {
        label: "AI-generated design",
        doc: include_str!("../../../docs/AI_GENERATED_DESIGN.md"),
        permalink: "/ai-generated-design",
    },
];

#[test]
fn recent_public_docs_are_jekyll_pages() {
    for site_doc in RECENT_PUBLIC_DOCS {
        assert!(
            site_doc.doc.starts_with("---\n"),
            "{} should have Jekyll front matter so GitHub Pages renders it",
            site_doc.label
        );
        assert!(
            site_doc
                .doc
                .contains(&format!("permalink: {}", site_doc.permalink)),
            "{} should publish at {}",
            site_doc.label,
            site_doc.permalink
        );
    }
}

#[test]
fn landing_page_surfaces_recent_public_docs() {
    let landing = include_str!("../../../docs/_layouts/landing.html");

    for site_doc in RECENT_PUBLIC_DOCS {
        assert!(
            landing.contains(&format!(
                "href=\"{{{{ '{}' | relative_url }}}}\"",
                site_doc.permalink
            )),
            "landing page should link to {} at {}",
            site_doc.label,
            site_doc.permalink
        );
    }

    assert!(
        landing.contains("Website audit trail"),
        "landing page should name the audit trail for recently documented capabilities"
    );
}
