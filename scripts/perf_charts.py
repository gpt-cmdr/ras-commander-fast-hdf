"""Performance charts and table for the README and docs/performance.md.

Usage: python scripts/perf_charts.py <benchmark summary.json>

The summary is a list of benchmark cases: ras-commander public calls timed in
h5py, auto and native mode (medians of fresh processes). Writes the SVG charts
to docs/images/ and prints the markdown table.
"""
import json
import re
import sys
from pathlib import Path

import matplotlib
matplotlib.use("svg")
import matplotlib.pyplot as plt
from matplotlib.ticker import FixedLocator, FuncFormatter, NullLocator

SRC = Path(sys.argv[1])
OUT = Path(__file__).resolve().parents[1] / "docs" / "images"
OUT.mkdir(parents=True, exist_ok=True)

THEMES = {
    "light": dict(surface="#fcfcfb", text="#0b0b0b", text2="#52514e", grid="#e4e3df",
                  native="#2a78d6", h5py="#eb6834"),
    "dark": dict(surface="#1a1a19", text="#f0efec", text2="#c3c2b7", grid="#3a3a37",
                 native="#3987e5", h5py="#d95926"),
}

rows = json.load(open(SRC))


def label(method):
    cls, name = method.split(".", 1)
    m = re.match(r"(\w+)(?:\[(.+)\])?", name)
    base, variant = m.group(1), m.group(2)
    return f"{base} ({variant.lower()})" if variant else base


# One row per method: its heaviest timed workload (slowest h5py repeat call).
largest = {}
for r in rows:
    if r["method"] not in largest or r["h5py_warm"] > largest[r["method"]]["h5py_warm"]:
        largest[r["method"]] = r
per_method = sorted(largest.values(), key=lambda r: r["h5py_warm"] / r["auto_warm"])


def style(ax, t):
    ax.set_facecolor(t["surface"])
    for side in ("top", "right", "left"):
        ax.spines[side].set_visible(False)
    ax.spines["bottom"].set_color(t["grid"])
    ax.tick_params(colors=t["text2"], length=0, labelsize=9)
    ax.xaxis.grid(True, color=t["grid"], linewidth=0.8)
    ax.set_axisbelow(True)


def speedup_chart(theme):
    t = THEMES[theme]
    fig, ax = plt.subplots(figsize=(8.4, 6.4), dpi=100)
    fig.patch.set_facecolor(t["surface"])
    style(ax, t)
    ys = range(len(per_method))
    vals = [r["h5py_warm"] / r["auto_warm"] for r in per_method]
    ax.axvline(1, color=t["text2"], linewidth=1, linestyle=(0, (3, 3)))
    ax.hlines(list(ys), 1, vals, color=t["native"], linewidth=2)
    ax.scatter(vals, list(ys), s=64, color=t["native"], edgecolors=t["surface"], linewidths=2, zorder=3)
    for y, v in zip(ys, vals):
        ax.annotate(f"{v:.1f}×", (v, y), xytext=(8 if v >= 1 else -8, 0), textcoords="offset points",
                    ha="left" if v >= 1 else "right", va="center", fontsize=9, color=t["text"])
    ax.set_yticks(list(ys))
    ax.set_yticklabels([label(r["method"]) for r in per_method], color=t["text"])
    ax.set_xscale("log")
    ticks = [0.25, 0.5, 1, 2, 5, 10, 20]
    ax.xaxis.set_major_locator(FixedLocator(ticks))
    ax.xaxis.set_minor_locator(NullLocator())
    ax.xaxis.set_major_formatter(FuncFormatter(lambda v, _: f"{v:g}×"))
    ax.set_xlim(0.2, 40)
    ax.set_xlabel("Speed-up of the native engine over h5py (repeat calls, log scale; >1× is faster)",
                  color=t["text2"], fontsize=9)
    ax.text(1, len(per_method) - 0.3, " same speed", color=t["text2"], fontsize=8, va="bottom")
    fig.suptitle("Native engine speed-up by public method", x=0.02, ha="left", fontsize=13,
                 color=t["text"], fontweight="bold")
    fig.text(0.02, 0.925, "Heaviest test file per method. Identical results in every row.",
             fontsize=9, color=t["text2"])
    fig.tight_layout(rect=(0, 0, 1, 0.92))
    fig.savefig(OUT / f"speedup-by-method-{theme}.svg", facecolor=t["surface"])
    plt.close(fig)


polygons = sorted((r for r in rows if r["method"] == "HdfMesh.get_mesh_cell_polygons"), key=lambda r: r["len"])


def polygon_chart(theme):
    t = THEMES[theme]
    fig, ax = plt.subplots(figsize=(8.4, 3.6), dpi=100)
    fig.patch.set_facecolor(t["surface"])
    style(ax, t)
    h = 0.36
    for i, r in enumerate(polygons):
        for off, key, color, name in ((h / 2 + 0.02, "h5py_cold", t["h5py"], "h5py"),
                                      (-h / 2 - 0.02, "auto_cold", t["native"], "native")):
            v = r[key]
            ax.barh(i + off, v, height=h, color=color, label=name if i == 0 else None)
            ax.annotate(f"{v:.2f} s" if v < 10 else f"{v:.1f} s", (v, i + off), xytext=(5, 0),
                        textcoords="offset points", va="center", fontsize=8.5, color=t["text"])
    ax.set_yticks(range(len(polygons)))
    ax.set_yticklabels([f"{r['len']:,} cells" for r in polygons], color=t["text"])
    ax.set_xscale("log")
    ticks = [0.05, 0.1, 0.5, 1, 5, 10, 30]
    ax.xaxis.set_major_locator(FixedLocator(ticks))
    ax.xaxis.set_minor_locator(NullLocator())
    ax.xaxis.set_major_formatter(FuncFormatter(lambda v, _: f"{v:g} s"))
    ax.set_xlim(0.04, 60)
    ax.set_xlabel("Seconds for the first call in a new process (log scale)", color=t["text2"], fontsize=9)
    leg = ax.legend(loc="lower right", frameon=False, fontsize=9)
    for text in leg.get_texts():
        text.set_color(t["text"])
    fig.suptitle("HdfMesh.get_mesh_cell_polygons: h5py vs native", x=0.02, ha="left", fontsize=13,
                 color=t["text"], fontweight="bold")
    fig.tight_layout(rect=(0, 0, 1, 0.93))
    fig.savefig(OUT / f"mesh-polygons-{theme}.svg", facecolor=t["surface"])
    plt.close(fig)


for theme in THEMES:
    speedup_chart(theme)
    polygon_chart(theme)

# Markdown table (largest file per method), fastest first.
lines = ["| Method | Result rows | h5py | Native | Speed-up |", "|---|---:|---:|---:|---:|"]


def ms(s):
    v = s * 1000
    return f"{v:,.0f} ms" if v >= 10 else f"{v:.1f} ms"


for r in reversed(per_method):
    lines.append(f"| `{r['method'].split('[')[0]}`{' (' + r['method'].split('[')[1].rstrip(']').lower() + ')' if '[' in r['method'] else ''} "
                 f"| {r['len']:,} | {ms(r['h5py_warm'])} | {ms(r['auto_warm'])} | {r['h5py_warm'] / r['auto_warm']:.1f}× |")
print("\n".join(lines))

# Overall geometric means for the text.
import math
def gmean(key_a, key_b, rs):
    return math.exp(sum(math.log(r[key_a] / r[key_b]) for r in rs) / len(rs))
print("rows", len(rows), "methods", len(per_method))
print("gmean cold", round(gmean("h5py_cold", "auto_cold", rows), 2), "warm", round(gmean("h5py_warm", "auto_warm", rows), 2))
print("identical data all", all(r["hash_identical"] for r in rows), "errors", sum(r["errors"] for r in rows))
