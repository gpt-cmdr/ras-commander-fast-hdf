"""Performance charts and tables for docs/performance.md (large-project presentation).

Usage: python scripts/perf_charts.py <benchmark summary.json> [threshold_ms]

The summary is the release benchmark's summary.json: one row per (method, file,
warm|cold) with medians in ms for ``base_h5py`` (the pre-engine release, pure
h5py), ``new_h5py``, ``new_auto`` and ``new_native``. For each method the row on
its largest model (slowest baseline repeat call) is used. Methods whose
baseline time on that model is at least ``threshold_ms`` (default 100) are the
headline set; the rest are small calls. Writes the headline chart (light and
dark SVG) to docs/images/ and prints both markdown tables.
"""
import json
import os
import re
import sys
from pathlib import Path

import matplotlib
matplotlib.use("svg")
import matplotlib.pyplot as plt
from matplotlib.ticker import FixedLocator, FuncFormatter, NullLocator

SRC = Path(sys.argv[1])
THRESHOLD = float(sys.argv[2]) if len(sys.argv) > 2 else 100.0
OUT = Path(__file__).resolve().parents[1] / "docs" / "images"
OUT.mkdir(parents=True, exist_ok=True)

THEMES = {
    "light": dict(surface="#fcfcfb", text="#0b0b0b", text2="#52514e", grid="#e4e3df",
                  new="#2a78d6", base="#eb6834"),
    "dark": dict(surface="#1a1a19", text="#f0efec", text2="#c3c2b7", grid="#3a3a37",
                 new="#3987e5", base="#d95926"),
}

rows = [r for r in json.load(open(SRC))["rows"] if r["which"] == "warm"]


def label(method):
    cls, name = method.split(".", 1)
    m = re.match(r"(\w+)(?:\[(.+)\])?", name)
    base, variant = m.group(1), m.group(2)
    if variant == "table":
        return f"{base} (include_geometry=False)"
    return f"{base} ({variant.lower()})" if variant else base


def reference(r):
    return r["base_h5py"] if r["base_h5py"] is not None else r["new_h5py"]


largest = {}
for r in rows:
    if r["method"] not in largest or reference(r) > reference(largest[r["method"]]):
        largest[r["method"]] = r
headline = sorted((r for r in largest.values() if reference(r) >= THRESHOLD),
                  key=lambda r: -reference(r))
small = sorted((r for r in largest.values() if reference(r) < THRESHOLD),
               key=lambda r: -reference(r))


def ms(v):
    if v is None:
        return "–"
    return f"{v:,.0f} ms" if v >= 100 else f"{v:.1f} ms" if v >= 10 else f"{v:.2f} ms"


def ratio(a, b):
    return "–" if a is None or b is None or not b else f"{a / b:.1f}×"


def style(ax, t):
    ax.set_facecolor(t["surface"])
    for side in ("top", "right", "left"):
        ax.spines[side].set_visible(False)
    ax.spines["bottom"].set_color(t["grid"])
    ax.tick_params(colors=t["text2"], length=0, labelsize=9)
    ax.xaxis.grid(True, color=t["grid"], linewidth=0.8)
    ax.set_axisbelow(True)


def headline_chart(theme):
    """Grouped horizontal bars, log time axis: baseline vs new auto on each call's largest model."""
    t = THEMES[theme]
    data = [r for r in headline if r["base_h5py"] is not None]
    n = len(data)
    fig, ax = plt.subplots(figsize=(8.6, 0.62 * n + 1.5), dpi=100)
    fig.patch.set_facecolor(t["surface"])
    style(ax, t)
    h = 0.34
    lo = min(min(r["new_auto"], r["base_h5py"]) for r in data)
    hi = max(r["base_h5py"] for r in data)
    ax.text(hi * 9, n - 0.45, "speed-up", va="bottom", ha="right", fontsize=8, color=t["text2"])
    for i, r in enumerate(data):
        y = n - 1 - i
        for dy, key, color in ((h / 2 + 0.02, "base_h5py", t["base"]), (-h / 2 - 0.02, "new_auto", t["new"])):
            ax.barh(y + dy, r[key], height=h, color=color, left=1)
            ax.text(r[key] * 1.08, y + dy, ms(r[key]), va="center", fontsize=8, color=t["text2"])
        ax.text(hi * 9, y, ratio(r["base_h5py"], r["new_auto"]),
                va="center", ha="right", fontsize=9, color=t["text"], fontweight="bold")
    ax.set_yticks(range(n))
    ax.set_yticklabels([f"{label(r['method'])}\n{r['fixture']}" for r in reversed(data)],
                       fontsize=8.5, color=t["text"])
    ax.set_xscale("log")
    ticks = [v for v in (10, 30, 100, 300, 1000, 3000, 10000, 30000, 100000) if lo / 3 <= v <= hi * 4]
    ax.xaxis.set_major_locator(FixedLocator(ticks))
    ax.xaxis.set_minor_locator(NullLocator())
    ax.xaxis.set_major_formatter(FuncFormatter(lambda v, _: f"{v / 1000:g} s" if v >= 1000 else f"{v:g} ms"))
    ax.set_xlim(lo / 2, hi * 10)
    ax.set_xlabel("Repeat-call time on the largest test model (log scale)", color=t["text2"], fontsize=9)
    handles = [plt.Rectangle((0, 0), 1, 1, color=t["base"]), plt.Rectangle((0, 0), 1, 1, color=t["new"])]
    leg = ax.legend(handles, ["0.106.1 (before the engine)", "0.110.1 (default auto mode)"],
                    loc="lower center", bbox_to_anchor=(0.5, 1.0), ncol=2, frameon=False, fontsize=8.5)
    for text in leg.get_texts():
        text.set_color(t["text"])
    fig.tight_layout()
    fig.savefig(OUT / f"large-project-calls-{theme}.svg", facecolor=t["surface"])
    if os.environ.get("PERF_CHART_PREVIEW"):
        fig.savefig(Path(os.environ["PERF_CHART_PREVIEW"]) / f"large-project-calls-{theme}.png", facecolor=t["surface"])
    plt.close(fig)


def polygon_chart(theme):
    """get_mesh_cell_polygons by mesh size: 0.106.1 vs 0.110.1 h5py vs 0.110.1 auto (warm)."""
    t = THEMES[theme]
    data = sorted((r for r in rows if r["method"] == "HdfMesh.get_mesh_cell_polygons"),
                  key=lambda r: r["rows"])
    fig, ax = plt.subplots(figsize=(8.4, 4.4), dpi=100)
    fig.patch.set_facecolor(t["surface"])
    style(ax, t)
    h = 0.26
    series = ((h + 0.03, "base_h5py", t["base"], "0.106.1 (before the engine)"),
              (0.0, "new_h5py", t["text2"], "0.110.1, h5py mode"),
              (-h - 0.03, "new_auto", t["new"], "0.110.1 (default auto mode)"))
    for i, r in enumerate(data):
        for off, key, color, name in series:
            v = r[key]
            ax.barh(i + off, v, height=h, color=color, label=name if i == 0 else None)
            ax.annotate(ms(v), (v, i + off), xytext=(4, 0), textcoords="offset points",
                        va="center", fontsize=8, color=t["text2"])
    ax.set_yticks(range(len(data)))
    ax.set_yticklabels([f"{r['rows']:,} cells\n{r['fixture']}" for r in data], color=t["text"], fontsize=8.5)
    ax.set_xscale("log")
    ax.xaxis.set_major_locator(FixedLocator([10, 30, 100, 300, 1000, 3000, 10000, 30000]))
    ax.xaxis.set_minor_locator(NullLocator())
    ax.xaxis.set_major_formatter(FuncFormatter(lambda v, _: f"{v / 1000:g} s" if v >= 1000 else f"{v:g} ms"))
    ax.set_xlim(8, 90000)
    ax.set_xlabel("Repeat-call time (log scale)", color=t["text2"], fontsize=9)
    leg = ax.legend(loc="lower center", bbox_to_anchor=(0.5, 1.0), ncol=3, frameon=False, fontsize=8.5)
    for text in leg.get_texts():
        text.set_color(t["text"])
    fig.tight_layout()
    fig.savefig(OUT / f"mesh-polygons-{theme}.svg", facecolor=t["surface"])
    if os.environ.get("PERF_CHART_PREVIEW"):
        fig.savefig(Path(os.environ["PERF_CHART_PREVIEW"]) / f"mesh-polygons-{theme}.png", facecolor=t["surface"])
    plt.close(fig)


for theme in THEMES:
    headline_chart(theme)
    polygon_chart(theme)

print(f"## Headline (baseline >= {THRESHOLD:g} ms on the largest model)\n")
print("| Method | Largest test model (rows) | 0.106.1 (before the engine) | 0.110.1 h5py | 0.110.1 `auto` | Speed-up vs 0.106.1 | Native vs h5py, 0.110.1 |")
print("|---|---|---:|---:|---:|---:|---:|")
for r in headline:
    rows_txt = f"{r['rows']:,}" if isinstance(r.get("rows"), int) else ""
    print(f"| `{label(r['method'])}` | {r['fixture']} ({rows_txt}) | {ms(r['base_h5py'])} | {ms(r['new_h5py'])} | "
          f"{ms(r['new_auto'])} | **{ratio(r['base_h5py'], r['new_auto'])}** | {ratio(r['new_h5py'], r['new_auto'])} |")
print(f"\n## Small calls (baseline < {THRESHOLD:g} ms)\n")
print("| Method | Largest test model (rows) | 0.106.1 (before the engine) | 0.110.1 h5py | 0.110.1 `auto` |")
print("|---|---|---:|---:|---:|")
for r in small:
    rows_txt = f"{r['rows']:,}" if isinstance(r.get("rows"), int) else ""
    print(f"| `{label(r['method'])}` | {r['fixture']} ({rows_txt}) | {ms(r['base_h5py'])} | {ms(r['new_h5py'])} | {ms(r['new_auto'])} |")
