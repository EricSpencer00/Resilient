#!/usr/bin/env python3
"""Generate the Resilient language-capability matrix (SVG heatmap + markdown tables).

Single source of truth: data/language_matrix.json
Outputs:
  docs/compare/language-matrix.svg   -- the big 50x50 "box grid" heatmap
  docs/compare/language-matrix.md    -- full page: methodology + legend + thematic tables

Every cell is a FACTUAL claim about WHERE a capability lives (the tier ladder),
adjudicated against each capability's written `rule`. Regenerate after editing the
JSON:  python3 scripts/gen_matrix.py
"""
from __future__ import annotations

import json
import os
from xml.sax.saxutils import escape as xesc

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(ROOT, "data", "language_matrix.json")
SVG_OUT = os.path.join(ROOT, "docs", "compare", "language-matrix.svg")
MD_OUT = os.path.join(ROOT, "docs", "compare", "language-matrix.md")

TIERS = ("core", "standard", "ecosystem", "none")

# Brand-purple intensity ramp: intensity == "how native the capability is".
FILL = {
    "core": "#4423a0",
    "standard": "#8163d6",
    "ecosystem": "#c9bced",
    "none": "#efecf9",
}
# Glyph drawn inside each cell (also carried by the markdown tables for a
# colour-blind-safe, copy-pasteable encoding).
GLYPH = {"core": "●", "standard": "◐", "ecosystem": "○", "none": ""}
GLYPH_FILL = {"core": "#ffffff", "standard": "#ffffff", "ecosystem": "#4a3d7a", "none": "#c9c2e4"}

ACCENT = "#e0a92e"      # Resilient row highlight
ACCENT_BG = "#fff6dc"
INK = "#241a45"
MUTE = "#6b6390"

THEME_BAND = {
    "A": "#5a34c0", "B": "#6f45c9", "C": "#4a7bd0", "D": "#2f9c8f",
    "E": "#c0603a", "F": "#b04a86", "G": "#7a7086", "H": "#3a8f45",
}
# Short band labels so they never overflow a narrow theme's column span.
THEME_SHORT = {
    "A": "Verify", "B": "Types", "C": "Memory", "D": "Embedded",
    "E": "Reliability", "F": "Concurrency", "G": "Tooling", "H": "Provenance",
}


def load():
    with open(DATA, encoding="utf-8") as fh:
        return json.load(fh)


def build_grid(d):
    """grid[lang_id][cap_id] = tier, defaulting to 'none'."""
    lang_ids = [l["id"] for l in d["languages"]]
    valid = set(lang_ids)
    grid = {lid: {} for lid in lang_ids}
    for cap in d["capabilities"]:
        cid = cap["id"]
        for lid in lang_ids:
            grid[lid][cid] = "none"
        for tier in ("core", "standard", "ecosystem"):
            for lid in cap["buckets"].get(tier, []):
                if lid in valid:
                    grid[lid][cid] = tier
    return grid


# --------------------------------------------------------------------------- SVG
def svg(d, grid):
    langs = d["languages"]
    caps = d["capabilities"]
    themes = {t["id"]: t["name"] for t in d["themes"]}
    ncap, nlang = len(caps), len(langs)

    pitch = 20
    box = 17
    pad = 26
    title_h = 46
    theme_h = 20
    label_h = 158
    gutter_x = 150          # language names
    grid_left = pad + gutter_x
    grid_top = pad + title_h + theme_h + label_h
    grid_w = ncap * pitch
    grid_h = nlang * pitch
    legend_h = 88
    W = grid_left + grid_w + pad
    H = grid_top + grid_h + legend_h

    p = []
    p.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" '
        f'font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif" '
        f'width="{W}" height="{H}">'
    )
    p.append(f'<rect x="0" y="0" width="{W}" height="{H}" fill="#ffffff"/>')

    # Title
    p.append(
        f'<text x="{pad}" y="{pad+20}" font-size="19" font-weight="700" fill="{INK}">'
        f'Resilient &#215; 49 languages &#215; 50 capabilities</text>'
    )
    p.append(
        f'<text x="{pad}" y="{pad+38}" font-size="12" fill="{MUTE}">'
        f'Each cell states <tspan font-style="italic">where</tspan> a capability lives — a checkable fact, not a rating. '
        f'Darker = more native.</text>'
    )

    # Theme bands + capability column labels
    # find contiguous theme spans
    col = 0
    spans = []
    while col < ncap:
        th = caps[col]["theme"]
        start = col
        while col < ncap and caps[col]["theme"] == th:
            col += 1
        spans.append((th, start, col))  # [start,end)
    band_y = pad + title_h
    for th, s, e in spans:
        x = grid_left + s * pitch
        w = (e - s) * pitch - 2
        p.append(
            f'<rect x="{x+1}" y="{band_y}" width="{w}" height="{theme_h-4}" rx="3" '
            f'fill="{THEME_BAND[th]}"/>'
        )
        cx = x + ((e - s) * pitch) / 2
        p.append(
            f'<text x="{cx:.1f}" y="{band_y+theme_h-7}" font-size="9.5" font-weight="700" '
            f'fill="#ffffff" text-anchor="middle">{th} &#183; {xesc(THEME_SHORT[th])}</text>'
        )

    # rotated capability labels
    for i, cap in enumerate(caps):
        cx = grid_left + i * pitch + pitch / 2
        y = grid_top - 6
        p.append(
            f'<g transform="translate({cx:.1f},{y}) rotate(-55)">'
            f'<text x="4" y="4" font-size="9" fill="{INK}" text-anchor="start">{xesc(cap["short"])}</text></g>'
        )

    # Resilient highlight band (row 0) behind everything in the grid area
    p.append(
        f'<rect x="{pad}" y="{grid_top-2}" width="{gutter_x+grid_w+2}" height="{pitch}" '
        f'rx="4" fill="{ACCENT_BG}"/>'
    )
    p.append(
        f'<rect x="{pad}" y="{grid_top-2}" width="{gutter_x+grid_w+2}" height="{pitch}" '
        f'rx="4" fill="none" stroke="{ACCENT}" stroke-width="1.6"/>'
    )

    # family separators + labels
    prev_family = None
    for r, lang in enumerate(langs):
        fam = lang["family"]
        if prev_family is not None and fam != prev_family:
            y = grid_top + r * pitch
            p.append(
                f'<line x1="{pad}" y1="{y}" x2="{grid_left+grid_w}" y2="{y}" '
                f'stroke="#d9d3ee" stroke-width="1.5"/>'
            )
        prev_family = fam

    # language row labels
    for r, lang in enumerate(langs):
        y = grid_top + r * pitch + pitch / 2 + 3.5
        is_res = lang["id"] == "resilient"
        weight = "700" if is_res else "400"
        fill = INK if is_res else "#3a3358"
        p.append(
            f'<text x="{grid_left-8}" y="{y:.1f}" font-size="10.5" font-weight="{weight}" '
            f'fill="{fill}" text-anchor="end">{xesc(lang["name"])}</text>'
        )

    # cells
    for r, lang in enumerate(langs):
        for c, cap in enumerate(caps):
            tier = grid[lang["id"]][cap["id"]]
            x = grid_left + c * pitch + (pitch - box) / 2
            y = grid_top + r * pitch + (pitch - box) / 2
            p.append(
                f'<rect x="{x:.1f}" y="{y:.1f}" width="{box}" height="{box}" rx="3" '
                f'fill="{FILL[tier]}"/>'
            )
            g = GLYPH[tier]
            if g:
                p.append(
                    f'<text x="{x+box/2:.1f}" y="{y+box/2+3.6:.1f}" font-size="10.5" '
                    f'fill="{GLYPH_FILL[tier]}" text-anchor="middle">{g}</text>'
                )

    # legend
    ly = grid_top + grid_h + 30
    p.append(f'<text x="{pad}" y="{ly-8}" font-size="11" font-weight="700" fill="{INK}">The ladder</text>')
    lx = pad
    labels = {
        "core": "Core — guaranteed by the language / its standard toolchain",
        "standard": "Standard — official opt-in tooling or stdlib",
        "ecosystem": "Ecosystem — third-party libraries / research only",
        "none": "None — absent or incompatible",
    }
    for tier in TIERS:
        p.append(f'<rect x="{pad}" y="{ly}" width="15" height="15" rx="3" fill="{FILL[tier]}"/>')
        if GLYPH[tier]:
            p.append(
                f'<text x="{pad+7.5}" y="{ly+11}" font-size="9.5" fill="{GLYPH_FILL[tier]}" '
                f'text-anchor="middle">{GLYPH[tier]}</text>'
            )
        p.append(f'<text x="{pad+21}" y="{ly+12}" font-size="10.5" fill="{INK}">{xesc(labels[tier])}</text>')
        ly += 18
    return "\n".join(p) + "\n</svg>\n"


# ---------------------------------------------------------------------- markdown
def md(d, grid):
    langs = d["languages"]
    caps = d["capabilities"]
    themes = d["themes"]
    theme_name = {t["id"]: t["name"] for t in themes}
    sym = {t: d["tier_ladder"][t]["symbol"] for t in TIERS}

    by_theme = {t["id"]: [c for c in caps if c["theme"] == t["id"]] for t in themes}

    # per-language tally
    def tally(lid):
        counts = {t: 0 for t in TIERS}
        for c in caps:
            counts[grid[lid][c["id"]]] += 1
        return counts

    out = []
    out.append("---")
    out.append("title: Language capability matrix")
    out.append("parent: Compare")
    out.append("nav_order: 1")
    out.append('description: "Where 50 programming languages put 50 capabilities — a factual, tier-based comparison against Resilient."')
    out.append("---\n")
    out.append("# The language capability matrix\n")
    out.append(
        "> **This is not a scoreboard of opinions.** Every cell answers one factual, "
        "checkable question: *where does this capability live for this language?* — baked "
        "into the language, bolted on through official tooling, reachable only via third-party "
        "libraries, or absent. The tiers below are defined precisely enough that any cell is "
        "falsifiable: cite the rule and the language's official documentation.\n"
    )

    # the grid image
    out.append("![Resilient vs 49 languages across 50 capabilities](language-matrix.svg)\n")
    out.append(
        "*The full 50×50 grid. Resilient is the highlighted top row; capabilities are grouped "
        "into eight themes left-to-right; languages are grouped by family top-to-bottom.*\n"
    )

    # ladder
    out.append("## The tier ladder\n")
    out.append("| | Tier | What it means |")
    out.append("|---|---|---|")
    for t in TIERS:
        L = d["tier_ladder"][t]
        out.append(f"| {L['symbol']} | **{L['label']}** | {L['def']} |")
    out.append("")
    out.append(
        "The ladder measures **provenance, not quality**. “Core” does not mean *good* and "
        "“Ecosystem” does not mean *bad* — a mature third-party library can be more "
        "battle-tested than a young built-in. It means exactly what it says: how *native* the "
        "capability is. Two layers of backing make each cell defensible — one universal ladder, "
        "plus a per-capability rule (shown in each section) stating what earns each tier **for that "
        "specific capability**.\n"
    )

    # Resilient scorecard
    rc = tally("resilient")
    out.append("## Resilient at a glance\n")
    out.append(
        f"Across all 50 capabilities, Resilient rates "
        f"**{rc['core']} {sym['core']} Core**, **{rc['standard']} {sym['standard']} Standard**, "
        f"**{rc['ecosystem']} {sym['ecosystem']} Ecosystem**, and **{rc['none']} {sym['none']} None**. "
        "It concentrates its Core tiers in verification, memory/embedded safety, fault handling, and "
        "AI-code provenance — and is honestly **weakest on ecosystem maturity** (Theme G): a single "
        "young implementation, a nascent registry, no qualified toolchain, and no field track record "
        "yet. That gap is a fact about the language's age, not its design.\n"
    )

    # thematic tables
    out.append("## Capabilities by theme\n")
    for t in themes:
        tid = t["id"]
        tcaps = by_theme[tid]
        out.append(f"### {tid} &middot; {theme_name[tid]}\n")
        header = "| Language | " + " | ".join(f"{c['short']}" for c in tcaps) + " |"
        sep = "|---|" + "|".join([":-:"] * len(tcaps)) + "|"
        out.append(header)
        out.append(sep)
        for lang in langs:
            name = f"**{lang['name']}**" if lang["id"] == "resilient" else lang["name"]
            cells = " | ".join(sym[grid[lang["id"]][c["id"]]] for c in tcaps)
            out.append(f"| {name} | {cells} |")
        out.append("")
        # rules + explanations
        out.append("<details><summary>Tier rules &amp; notes for this theme</summary>\n")
        for c in tcaps:
            out.append(f"**{c['short']}** — {c['name']}  ")
            out.append(f"*Rule:* {c['rule']}  ")
            if c.get("explain"):
                out.append(f"*Reading:* {c['explain']}  ")
            out.append("")
        out.append("</details>\n")

    # full per-language tally table
    out.append("## Every language, tallied\n")
    out.append(f"| Language | Family | {sym['core']} Core | {sym['standard']} Standard | {sym['ecosystem']} Ecosystem | {sym['none']} None |")
    out.append("|---|---|:-:|:-:|:-:|:-:|")
    for lang in langs:
        c = tally(lang["id"])
        name = f"**{lang['name']}**" if lang["id"] == "resilient" else lang["name"]
        out.append(f"| {name} | {lang['family']} | {c['core']} | {c['standard']} | {c['ecosystem']} | {c['none']} |")
    out.append("")

    out.append("## How to dispute a cell\n")
    out.append(
        "Found one you disagree with? Good — that is the point of defining the tiers. Open an "
        "issue citing (1) the capability's **rule**, (2) the **language's official documentation**, "
        "and (3) which tier you believe applies. Because every rating is a claim about provenance "
        "rather than taste, disagreements resolve against evidence, not vibes.\n"
    )
    out.append("## Regenerating\n")
    out.append(
        "The grid and every table on this page are generated from "
        "[`data/language_matrix.json`](https://github.com/EricSpencer00/Resilient/blob/main/data/language_matrix.json) "
        "by [`scripts/gen_matrix.py`](https://github.com/EricSpencer00/Resilient/blob/main/scripts/gen_matrix.py):\n"
    )
    out.append("```bash\npython3 scripts/gen_matrix.py\n```\n")
    return "\n".join(out)


def main():
    d = load()
    grid = build_grid(d)
    os.makedirs(os.path.dirname(SVG_OUT), exist_ok=True)
    with open(SVG_OUT, "w", encoding="utf-8") as fh:
        fh.write(svg(d, grid))
    with open(MD_OUT, "w", encoding="utf-8") as fh:
        fh.write(md(d, grid))
    # quick stats to stdout
    res = grid["resilient"]
    from collections import Counter
    c = Counter(res.values())
    print(f"wrote {SVG_OUT}")
    print(f"wrote {MD_OUT}")
    print(f"Resilient: core={c['core']} standard={c['standard']} ecosystem={c['ecosystem']} none={c['none']}")


if __name__ == "__main__":
    main()
