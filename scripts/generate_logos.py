"""Generate Rust-flavoured derivatives of the ras-commander logo.

Usage: python scripts/generate_logos.py

Writes the chosen logo (Option C, "Engine Inside") and its link-preview cards
to docs/images/logo/ for each engine (FAST-HDF, FAST-MAP, FAST-DSS) and for the
Native Rust Engines family, with FAST-DSS either "Coming Soon" or released. The
five review concepts and two icons go to docs/images/logo/concepts/. Render the PNGs with scripts/render_logo_png.ps1.
"""
import math
from pathlib import Path


def gear_path(cx, cy, r_root, r_tip, n, tooth_frac=0.55, taper=0.35, rot=0.0):
    """Closed SVG path for a cog: n trapezoid teeth joined by root arcs."""
    p = 2 * math.pi / n
    hr = tooth_frac * p / 2
    ht = hr * (1 - taper)

    def pt(r, a):
        return f"{cx + r * math.cos(a):.2f},{cy + r * math.sin(a):.2f}"

    d = []
    for i in range(n):
        c = rot + i * p - math.pi / 2
        if i == 0:
            d.append(f"M{pt(r_root, c - hr)}")
        d.append(f"L{pt(r_tip, c - ht)}")
        d.append(f"L{pt(r_tip, c + ht)}")
        d.append(f"L{pt(r_root, c + hr)}")
        d.append(f"A{r_root},{r_root} 0 0 1 {pt(r_root, c + p - hr)}")
    d.append("Z")
    return " ".join(d)


def rivets(cx, cy, r, n, size, fill, rot=-math.pi / 2):
    """Small inward-pointing triangles around a ring (a nod to the Rust cog)."""
    out = []
    for i in range(n):
        a = rot + i * 2 * math.pi / n
        tip = (cx + (r - size) * math.cos(a), cy + (r - size) * math.sin(a))
        b1 = (cx + r * math.cos(a - 0.07), cy + r * math.sin(a - 0.07))
        b2 = (cx + r * math.cos(a + 0.07), cy + r * math.sin(a + 0.07))
        out.append(
            f'<polygon points="{tip[0]:.2f},{tip[1]:.2f} {b1[0]:.2f},{b1[1]:.2f} '
            f'{b2[0]:.2f},{b2[1]:.2f}" fill="{fill}" />'
        )
    return "\n    ".join(out)


WAVES = """<g opacity="{op}">
    <path d="M125,130 C145,120 165,140 185,130 C205,120 225,140 245,130 C265,120 285,140 325,130" fill="none" stroke="{c}" stroke-width="4" />
    <path d="M125,150 C145,140 165,160 185,150 C205,140 225,160 245,150 C265,140 285,160 325,150" fill="none" stroke="{c}" stroke-width="4" />
    <path d="M125,170 C145,160 165,180 185,170 C205,160 225,180 245,170 C265,160 285,180 325,170" fill="none" stroke="{c}" stroke-width="4" />
  </g>"""

CIRCUIT = """<pattern id="circuit-pattern" patternUnits="userSpaceOnUse" width="50" height="50" patternTransform="rotate(45)">
      <path d="M10,0 L10,50 M0,10 L50,10 M20,20 L30,20 L30,30 M40,10 L40,40 L30,40" fill="none" stroke="{c}" stroke-width="0.5" stroke-opacity="0.3" />
    </pattern>"""


def terminal(line1, line2, x=160, w=130, fs=16, fg="#5CDB95"):
    return f"""<rect x="{x}" y="75" width="{w}" height="55" rx="5" ry="5" fill="#000" fill-opacity="0.85" />
  <text x="{x + 10}" y="95" font-family="Consolas, monospace" font-size="{fs}" fill="{fg}">{line1}</text>
  <text x="{x + 10}" y="120" font-family="Consolas, monospace" font-size="{fs}" fill="{fg}">{line2}</text>
  <rect x="{x + 10}" y="125" width="8" height="2" fill="{fg}">
    <animate attributeName="opacity" values="1;0;1" dur="1.2s" repeatCount="indefinite" />
  </rect>"""


RUST = "#E5652E"      # warm rust orange
RUST_DEEP = "#B7410E" # oxidised iron
COPPER = "#DEA584"    # pale copper


def concept_a():
    """Cog Ring: original logo, circle swapped for a rust-orange cog."""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 450 300">
  <!-- Concept A - Cog Ring: the original layout with the emblem circle turned into a Rust-style cog -->
  <defs>
    <linearGradient id="bg-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#05386B" />
      <stop offset="100%" stop-color="#0B93C7" />
    </linearGradient>
    {CIRCUIT.format(c="#5CDB95")}
  </defs>
  <rect width="450" height="300" fill="url(#bg-gradient)" />
  <rect width="450" height="300" fill="url(#circuit-pattern)" opacity="0.2" />

  <!-- Cog emblem and name band -->
  <path d="{gear_path(225, 150, 96, 106, 32)}" fill="#05386B" stroke="{RUST}" stroke-width="2.5" stroke-linejoin="round" />
  <circle cx="225" cy="150" r="90" fill="none" stroke="{RUST}" stroke-width="1" stroke-opacity="0.6" />
  {rivets(225, 150, 90, 5, 8, RUST)}
  <rect x="55" y="185" width="340" height="50" fill="#05386B" stroke="{RUST}" stroke-width="2" rx="2" ry="2" />

  {terminal("$ cargo build", "  --release")}

  {WAVES.format(c="#5CDB95", op=0.7)}

  <text x="225" y="222" font-family="Arial, sans-serif" font-size="36" font-weight="bold" text-anchor="middle" fill="#EDF5E1">RAS-COMMANDER</text>
  <text x="225" y="278" font-family="Arial, sans-serif" font-size="18" font-style="italic" text-anchor="middle" fill="{COPPER}">Fast HDF • Native Rust Engine</text>
</svg>
"""


def concept_b():
    """Oxidized: full palette swap to charcoal and rust."""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 450 300">
  <!-- Concept B - Oxidized: same composition, charcoal-to-rust palette, copper waves -->
  <defs>
    <linearGradient id="bg-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#1A1614" />
      <stop offset="60%" stop-color="#4A2314" />
      <stop offset="100%" stop-color="{RUST_DEEP}" />
    </linearGradient>
    {CIRCUIT.format(c=COPPER)}
  </defs>
  <rect width="450" height="300" fill="url(#bg-gradient)" />
  <rect width="450" height="300" fill="url(#circuit-pattern)" opacity="0.25" />

  <path d="{gear_path(225, 150, 96, 106, 32)}" fill="#201A17" stroke="{RUST}" stroke-width="2.5" stroke-linejoin="round" />
  <circle cx="225" cy="150" r="90" fill="none" stroke="{COPPER}" stroke-width="1" stroke-opacity="0.5" />
  {rivets(225, 150, 90, 5, 8, RUST)}
  <rect x="55" y="185" width="340" height="50" fill="#201A17" stroke="{RUST}" stroke-width="2" rx="2" ry="2" />

  {terminal("$ pip install", "fast-hdf", fg="#F4A261")}

  {WAVES.format(c="#F4A261", op=0.8)}

  <text x="225" y="222" font-family="Arial, sans-serif" font-size="36" font-weight="bold" text-anchor="middle" fill="#F7EDE6">RAS-COMMANDER</text>
  <text x="225" y="278" font-family="Arial, sans-serif" font-size="18" font-style="italic" text-anchor="middle" fill="{COPPER}">Fast HDF • Native Rust Engine</text>
</svg>
"""


# Product variants of Option C. Each pill is (label, fill, note); a pill with a
# note is drawn greyed out with the note beside its label ("Coming Soon").
# Cogs are the small engines meshed with the main rust cog.
MAP_GREEN = "#1E8E62"  # FAST-MAP pill and cog
DSS_AMBER = "#B7791F"  # FAST-DSS pill and cog
GREY = "#5D6B78"       # an engine that is not released yet

HDF_PILL = ("FAST-HDF", RUST, None)
MAP_PILL = ("FAST-MAP", MAP_GREEN, None)
DSS_PILL = ("FAST-DSS", DSS_AMBER, None)
DSS_SOON = ("FAST-DSS", GREY, "(Coming Soon)")

VARIANTS = {
    # variant: (pills, tagline, cogs)
    "fast-hdf": ((HDF_PILL,), "Native Rust HDF Engine", ()),
    "fast-map": ((MAP_PILL,), "Native Rust Mapping Engine", ("map",)),
    "fast-dss": ((DSS_PILL,), "Native Rust DSS Engine", ("dss",)),
    "engines": ((HDF_PILL, MAP_PILL, DSS_SOON), "Native Rust Engines", ("map", "dss-soon")),
    "engines-dss": ((HDF_PILL, MAP_PILL, DSS_PILL), "Native Rust Engines", ("map", "dss")),
}

# Depth classes for the FAST-MAP cog's raster, shallow to deep.
DEPTH = ("#C6E9F7", "#86CBEC", "#3FA3D9", "#1668AE", "#0B3F7A")


def mapping_cog(cx=335, cy=57):
    """FAST-MAP: a small cog meshed with the main one, holding a depth raster."""
    cells, size = [], 9
    for row in range(7):
        for col in range(7):
            # Depth grows toward a channel running diagonally through the raster.
            d = abs((col - row) * 0.9 + 0.5)
            k = max(0, min(4, 4 - int(d)))
            x, y = cx - 31.5 + col * size, cy - 31.5 + row * size
            cells.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{size}" height="{size}" fill="{DEPTH[k]}" />')
    grid = "\n    ".join(cells)
    return f"""<path d="{gear_path(cx, cy, 33, 42, 12, rot=0.13)}" fill="{MAP_GREEN}" stroke="#5CDB95" stroke-width="2" stroke-linejoin="round" />
  <clipPath id="map-face"><circle cx="{cx}" cy="{cy}" r="27" /></clipPath>
  <g clip-path="url(#map-face)">
    {grid}
  </g>
  <circle cx="{cx}" cy="{cy}" r="27" fill="none" stroke="#05386B" stroke-width="2" />
  <circle cx="{cx}" cy="{cy}" r="4" fill="#05386B" stroke="#5CDB95" stroke-width="1.5" />"""


def dss_cog(cx=115, cy=57, soon=False):
    """FAST-DSS: the mirror cog, holding a hydrograph (DSS time series).

    soon=True greys it out with a dashed rim for an engine not released yet.
    """
    fill, rim, line = (GREY, "#9AA5B1", "#B8C2CC") if soon else (DSS_AMBER, "#F2C46D", "#F2C46D")
    dash = ' stroke-dasharray="4 3"' if soon else ""
    group = ' opacity="0.8"' if soon else ""
    return f"""<g{group}>
  <path d="{gear_path(cx, cy, 33, 42, 12, rot=-0.13)}" fill="{fill}" stroke="{rim}" stroke-width="2" stroke-linejoin="round"{dash} />
  <circle cx="{cx}" cy="{cy}" r="27" fill="#05386B" stroke="#05386B" stroke-width="2" />
  <path d="M{cx - 18},{cy + 15} L{cx + 19},{cy + 15} M{cx - 18},{cy + 15} L{cx - 18},{cy - 17}" fill="none" stroke="{rim}" stroke-width="1.5" />
  <path d="M{cx - 17},{cy + 12} C{cx - 9},{cy + 11} {cx - 7},{cy - 15} {cx - 2},{cy - 15} C{cx + 4},{cy - 15} {cx + 6},{cy + 5} {cx + 18},{cy + 9}" fill="none" stroke="{line}" stroke-width="2.5" stroke-linecap="round" />
  </g>"""


def _text_width(text, size, bold=False):
    """Rough Arial advance width; enough to centre a row of pills and a tagline."""
    per_char = 0.64 if bold else 0.50
    if not bold and text.upper() != text:
        per_char = 0.47
    return len(text) * size * per_char


def _pill_row(pills, tagline, y=263):
    """Pills centred under the band; the tagline follows them, or gets its own
    centred row below when there are several pills."""
    pill_fs, note_fs, tag_fs, gap = 14, 12, 17, 8
    widths = []
    for label, _, note in pills:
        w = _text_width(label, pill_fs, bold=True) + 24
        if note:
            w += _text_width(note, note_fs) + 6
        widths.append(w)
    tag_w = _text_width(tagline, tag_fs)
    own_row = len(pills) > 1
    total = sum(widths) + gap * (len(pills) - 1) + (0 if own_row else gap + 4 + tag_w)
    x = 225 - total / 2
    out = []
    for (label, colour, note), w in zip(pills, widths):
        if note:
            lw = _text_width(label, pill_fs, bold=True)
            out.append(f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="24" rx="12" ry="12" fill="{colour}" fill-opacity="0.55" stroke="#9AA5B1" stroke-width="1.5" stroke-dasharray="4 3" />')
            out.append(f'<text x="{x + 12:.1f}" y="{y + 17}" font-family="Arial, sans-serif" font-size="{pill_fs}" font-weight="bold" fill="#C9D1D9">{label}</text>')
            out.append(f'<text x="{x + 18 + lw:.1f}" y="{y + 16.5}" font-family="Arial, sans-serif" font-size="{note_fs}" font-style="italic" fill="#C9D1D9">{note}</text>')
        else:
            out.append(f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="24" rx="12" ry="12" fill="{colour}" />')
            out.append(f'<text x="{x + w / 2:.1f}" y="{y + 17}" font-family="Arial, sans-serif" font-size="{pill_fs}" font-weight="bold" text-anchor="middle" fill="#FFFFFF">{label}</text>')
        x += w + gap
    if own_row:
        out.append(f'<text x="225" y="{y + 47}" font-family="Arial, sans-serif" font-size="{tag_fs}" font-style="italic" text-anchor="middle" fill="#8EE4AF">{tagline}</text>')
    else:
        out.append(f'<text x="{x + 4:.1f}" y="{y + 18}" font-family="Arial, sans-serif" font-size="{tag_fs}" font-style="italic" fill="#8EE4AF">{tagline}</text>')
    return "\n  ".join(out)


def body_height(variant):
    """Logo height: 300 like the original, 320 when the tagline needs its own row."""
    return 320 if len(VARIANTS[variant][0]) > 1 else 300


def engine_inside_body(variant="fast-hdf"):
    """Option C artwork in the original 450-wide logo coordinates, without background."""
    pills, tagline, cogs = VARIANTS[variant]
    small = {"map": mapping_cog(), "dss": dss_cog(), "dss-soon": dss_cog(soon=True)}
    cog_svg = "\n  ".join(small[c] for c in cogs)
    return f"""{cog_svg}
  <path d="{gear_path(225, 150, 104, 114, 36)}" fill="{RUST_DEEP}" stroke="{RUST}" stroke-width="2" stroke-linejoin="round" />
  <circle cx="225" cy="150" r="100" fill="#05386B" stroke="#5CDB95" stroke-width="2" />
  <rect x="55" y="185" width="340" height="50" fill="#05386B" stroke="#5CDB95" stroke-width="2" rx="2" ry="2" />

  {terminal("$ pip install", "ras-commander")}

  {WAVES.format(c="#5CDB95", op=0.7)}

  <text x="225" y="222" font-family="Arial, sans-serif" font-size="36" font-weight="bold" text-anchor="middle" fill="#EDF5E1">RAS-COMMANDER</text>
  <!-- product pills and tagline -->
  {_pill_row(pills, tagline)}"""


def concept_c(width=450, height=None, variant="fast-hdf"):
    """Engine Inside: original blue/green kept, a rust cog wraps the green circle.

    With no size this is the logo (450 wide). Other sizes (link-preview cards)
    extend the background and centre the logo, scaled to fill the height.
    """
    bh = body_height(variant)
    height = height or bh
    body = engine_inside_body(variant)
    if (width, height) != (450, bh):
        scale = min(width / 450, (height - 12) / bh)
        tx = (width - 450 * scale) / 2
        ty = (height - bh * scale) / 2
        body = f'<g transform="translate({tx:g},{ty:g}) scale({scale:g})">\n  {body}\n  </g>'
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
  <!-- RAS Commander {VARIANTS[variant][1]}, Option C "Engine Inside": the original green emblem is kept and a rust cog drives it from outside -->
  <defs>
    <linearGradient id="bg-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#05386B" />
      <stop offset="100%" stop-color="#0B93C7" />
    </linearGradient>
    {CIRCUIT.format(c="#5CDB95")}
  </defs>
  <rect width="{width}" height="{height}" fill="url(#bg-gradient)" />
  <rect width="{width}" height="{height}" fill="url(#circuit-pattern)" opacity="0.2" />

  {body}
</svg>
"""


def concept_d():
    """HDF Core: cog around a stacked-data cylinder, product name forward."""
    cyl = []
    # three stacked disks, waves running through their faces
    for i, y in enumerate((96, 128, 160)):
        cyl.append(
            f'<path d="M165,{y} L165,{y + 22} A60,14 0 0 0 285,{y + 22} L285,{y} Z" fill="#0A4A7A" stroke="#5CDB95" stroke-width="1.5" />'
        )
        cyl.append(
            f'<ellipse cx="225" cy="{y}" rx="60" ry="14" fill="#0D5E93" stroke="#5CDB95" stroke-width="1.5" />'
        )
        cyl.append(
            f'<path d="M177,{y} C192,{y - 6} 207,{y + 6} 222,{y} C237,{y - 6} 252,{y + 6} 273,{y}" fill="none" stroke="#5CDB95" stroke-width="2.5" opacity="0.85" />'
        )
    # draw bottom disk first so upper disks overlap it
    order = cyl[6:9] + cyl[3:6] + cyl[0:3]
    body = "\n  ".join(order)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 450 300">
  <!-- Concept D - HDF Core: a rust cog around a stack of data disks (HDF datasets) carrying the RAS water waves -->
  <defs>
    <linearGradient id="bg-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#05386B" />
      <stop offset="100%" stop-color="#0B93C7" />
    </linearGradient>
    {CIRCUIT.format(c="#5CDB95")}
  </defs>
  <rect width="450" height="300" fill="url(#bg-gradient)" />
  <rect width="450" height="300" fill="url(#circuit-pattern)" opacity="0.2" />

  <path d="{gear_path(225, 128, 88, 99, 28)}" fill="#05386B" stroke="{RUST}" stroke-width="2.5" stroke-linejoin="round" />
  {rivets(225, 128, 82, 5, 7, RUST)}

  {body}

  <text x="225" y="258" font-family="Arial, sans-serif" font-size="38" font-weight="bold" text-anchor="middle" fill="#EDF5E1">FAST-HDF</text>
  <text x="225" y="282" font-family="Arial, sans-serif" font-size="15" letter-spacing="3" text-anchor="middle" fill="{COPPER}">RAS-COMMANDER · RUST ENGINE</text>
</svg>
"""


def concept_e():
    """Velocity: cog with motion streaks, flow moving left-to-right."""
    streaks = "\n  ".join(
        f'<line x1="{x1}" y1="{y}" x2="{x2}" y2="{y}" stroke="{RUST}" stroke-width="{w}" stroke-linecap="round" opacity="{o}" />'
        for x1, x2, y, w, o in (
            (15, 83, 112, 4, 0.9),
            (35, 89, 135, 4, 0.7),
            (5, 85, 158, 4, 0.9),
            (45, 93, 181, 3, 0.5),
        )
    )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 450 300">
  <!-- Concept E - Velocity: the cog emblem shifted left with speed streaks; name stacked to the right -->
  <defs>
    <linearGradient id="bg-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#05386B" />
      <stop offset="100%" stop-color="#0B93C7" />
    </linearGradient>
    {CIRCUIT.format(c="#5CDB95")}
    <clipPath id="disc"><circle cx="160" cy="145" r="72" /></clipPath>
  </defs>
  <rect width="450" height="300" fill="url(#bg-gradient)" />
  <rect width="450" height="300" fill="url(#circuit-pattern)" opacity="0.2" />

  {streaks}

  <path d="{gear_path(160, 145, 80, 90, 26, rot=0.06)}" fill="#05386B" stroke="{RUST}" stroke-width="2.5" stroke-linejoin="round" />
  <circle cx="160" cy="145" r="72" fill="none" stroke="#5CDB95" stroke-width="1.5" />
  <g clip-path="url(#disc)" opacity="0.85">
    <path d="M80,120 C100,110 120,130 140,120 C160,110 180,130 200,120 C220,110 240,130 260,120" fill="none" stroke="#5CDB95" stroke-width="4" />
    <path d="M80,145 C100,135 120,155 140,145 C160,135 180,155 200,145 C220,135 240,155 260,145" fill="none" stroke="#5CDB95" stroke-width="4" />
    <path d="M80,170 C100,160 120,180 140,170 C160,160 180,180 200,170 C220,160 240,180 260,170" fill="none" stroke="#5CDB95" stroke-width="4" />
  </g>
  <polygon points="221,138 237,145 221,152" fill="{RUST}" />

  <text x="262" y="128" font-family="Arial, sans-serif" font-size="25" font-weight="bold" fill="#EDF5E1">RAS-</text>
  <text x="262" y="156" font-family="Arial, sans-serif" font-size="25" font-weight="bold" fill="#EDF5E1">COMMANDER</text>
  <text x="263" y="182" font-family="Arial, sans-serif" font-size="20" font-weight="bold" letter-spacing="2" fill="{RUST}">FAST-HDF</text>
  <text x="225" y="268" font-family="Arial, sans-serif" font-size="18" font-style="italic" text-anchor="middle" fill="#8EE4AF">Native Rust • Read at Speed</text>
</svg>
"""


def icon(variant):
    """Square mark for favicons, PyPI and GitHub social cards."""
    if variant == "blue":
        bg1, bg2, face, wave, cog = "#05386B", "#0B93C7", "#05386B", "#5CDB95", RUST
    else:
        bg1, bg2, face, wave, cog = "#1A1614", RUST_DEEP, "#201A17", "#F4A261", RUST
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <!-- Square mark ({variant}): cog + RAS water waves, for favicon / PyPI / GitHub avatar -->
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="{bg1}" />
      <stop offset="100%" stop-color="{bg2}" />
    </linearGradient>
    <clipPath id="face"><circle cx="128" cy="128" r="80" /></clipPath>
  </defs>
  <rect width="256" height="256" rx="40" ry="40" fill="url(#bg)" />
  <path d="{gear_path(128, 128, 92, 106, 24)}" fill="{face}" stroke="{cog}" stroke-width="5" stroke-linejoin="round" />
  <circle cx="128" cy="128" r="80" fill="none" stroke="{cog}" stroke-width="2" stroke-opacity="0.6" />
  {rivets(128, 128, 80, 5, 10, cog)}
  <g clip-path="url(#face)" stroke="{wave}" stroke-width="9" fill="none" stroke-linecap="round">
    <path d="M40,100 C60,86 80,114 100,100 C120,86 140,114 160,100 C180,86 200,114 220,100" />
    <path d="M40,130 C60,116 80,144 100,130 C120,116 140,144 160,130 C180,116 200,144 220,130" />
    <path d="M40,160 C60,146 80,174 100,160 C120,146 140,174 160,160 C180,146 200,174 220,160" />
  </g>
</svg>
"""


def main():
    logo_dir = Path(__file__).resolve().parent.parent / "docs" / "images" / "logo"
    out = logo_dir / "concepts"
    out.mkdir(parents=True, exist_ok=True)
    files = {
        "concept-a-cog-ring.svg": concept_a(),
        "concept-b-oxidized.svg": concept_b(),
        "concept-c-engine-inside.svg": concept_c(),
        "concept-d-hdf-core.svg": concept_d(),
        "concept-e-velocity.svg": concept_e(),
        "icon-cog-waves-blue.svg": icon("blue"),
        "icon-cog-waves-oxidized.svg": icon("oxidized"),
    }
    # The chosen logo (Option C) and its link-preview cards live one folder up.
    final = {}
    # One logo per engine, and the "Native Rust Engines" family logo with
    # FAST-DSS greyed out (engines) or released (engines-dss).
    for variant, stem in (
        ("fast-hdf", "ras-commander-fast-hdf"),
        ("fast-map", "ras-commander-fast-map"),
        ("fast-dss", "ras-commander-fast-dss"),
        ("engines", "ras-commander-rust-engines"),
        ("engines-dss", "ras-commander-rust-engines-dss"),
    ):
        final[f"{stem}_logo.svg"] = concept_c(variant=variant)
        final[f"{stem}_social.svg"] = concept_c(1200, 627, variant)         # LinkedIn / Open Graph
        final[f"{stem}_github-social.svg"] = concept_c(1280, 640, variant)  # GitHub social preview
    for name, svg in files.items():
        (out / name).write_text(svg, encoding="utf-8", newline="\n")
        print(name)
    for name, svg in final.items():
        (logo_dir / name).write_text(svg, encoding="utf-8", newline="\n")
        print(name)


if __name__ == "__main__":
    main()
