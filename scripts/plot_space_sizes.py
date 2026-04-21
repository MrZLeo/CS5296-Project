#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
PLOT_CACHE_DIR = REPO_ROOT / ".plot-cache"
PLOT_CACHE_DIR.mkdir(parents=True, exist_ok=True)
os.environ.setdefault("MPLCONFIGDIR", str(PLOT_CACHE_DIR / "matplotlib"))
os.environ.setdefault("XDG_CACHE_HOME", str(PLOT_CACHE_DIR / "xdg"))

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Patch


RUNTIME_ORDER = ["wasmedge-wasm", "wasmedge-aot", "docker"]
RUNTIME_SHORT_LABELS = {
    "wasmedge-wasm": "Wasm",
    "wasmedge-aot": "AOT",
    "docker": "Docker",
}
RUNTIME_COLORS = {
    "wasmedge-wasm": "#DDD4F0",
    "wasmedge-aot": "#A9D0F3",
    "docker": "#AEDFDE",
}
RUNTIME_HATCHES = {
    "wasmedge-wasm": "",
    "wasmedge-aot": "///",
    "docker": "\\\\",
}
METRIC_TITLES = {
    "artifact_only": "Artifact Only",
    "full_deploy_size": "Full Deploy Size",
    "runtime_peak_rss": "Runtime Peak RSS",
}

BACKGROUND = "#FFFFFF"
PANEL_BACKGROUND = "#FFFFFF"
GRID_COLOR = "#E6E1D9"
TEXT_COLOR = "#1A1A1A"
MUTED_TEXT = "#625D57"
FRAME_COLOR = "#7A746D"
LEGEND_FACE = "#FFFFFF"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate combined artifact, deploy-size, and runtime-RSS comparison plots."
    )
    parser.add_argument(
        "--input-csv",
        default="target/bench-results/space-size.csv",
        help="Path to the size comparison CSV.",
    )
    parser.add_argument(
        "--output-dir",
        default="target/bench-results/plots",
        help="Directory to write the generated PDF plot into.",
    )
    parser.add_argument(
        "--output-name",
        default="space_size_overview",
        help="Output PDF basename without extension.",
    )
    return parser.parse_args()


def apply_style() -> None:
    plt.rcParams.update(
        {
            "figure.facecolor": BACKGROUND,
            "axes.facecolor": PANEL_BACKGROUND,
            "savefig.facecolor": BACKGROUND,
            "axes.edgecolor": FRAME_COLOR,
            "axes.labelcolor": TEXT_COLOR,
            "text.color": TEXT_COLOR,
            "xtick.color": MUTED_TEXT,
            "ytick.color": MUTED_TEXT,
            "font.family": "sans-serif",
            "font.sans-serif": ["DejaVu Sans Mono", "DejaVu Sans", "Arial"],
            "axes.titleweight": "bold",
            "axes.titlesize": 12,
            "axes.labelsize": 11,
            "font.size": 10,
            "legend.frameon": True,
            "legend.fontsize": 10,
            "axes.spines.top": True,
            "axes.spines.right": True,
            "axes.grid": False,
            "axes.linewidth": 1.0,
            "xtick.major.width": 0.9,
            "ytick.major.width": 0.9,
            "xtick.major.size": 4.0,
            "ytick.major.size": 4.0,
            "grid.linewidth": 0.8,
            "pdf.fonttype": 42,
            "ps.fonttype": 42,
        }
    )


def runtime_label(runtime: str) -> str:
    return RUNTIME_SHORT_LABELS.get(runtime, runtime)


def load_rows(path: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    with path.open(newline="") as handle:
        for row in csv.DictReader(handle):
            rows.append(
                {
                    "runtime": row["runtime"],
                    "metric": row["metric"],
                    "kind": row["kind"],
                    "source": row["source"],
                    "value_bytes": int(row["value_bytes"]),
                    "value_mib": float(row["value_mib"]),
                }
            )
    return rows


def rows_for_metric(rows: list[dict[str, object]], metric: str) -> list[dict[str, object]]:
    order_index = {runtime: index for index, runtime in enumerate(RUNTIME_ORDER)}
    filtered = [row for row in rows if row["metric"] == metric]
    return sorted(filtered, key=lambda row: order_index.get(str(row["runtime"]), 999))


def style_axis_frame(axis: plt.Axes) -> None:
    for side in ["left", "right", "top", "bottom"]:
        axis.spines[side].set_visible(True)
        axis.spines[side].set_color(FRAME_COLOR)
        axis.spines[side].set_linewidth(1.0)


def runtime_legend() -> list[Patch]:
    return [
        Patch(
            facecolor=RUNTIME_COLORS[runtime],
            hatch=RUNTIME_HATCHES[runtime],
            edgecolor=FRAME_COLOR,
            linewidth=0.9,
            label=runtime_label(runtime),
        )
        for runtime in RUNTIME_ORDER
    ]


def decorate_legend(legend: plt.Legend) -> None:
    frame = legend.get_frame()
    frame.set_facecolor(LEGEND_FACE)
    frame.set_edgecolor("#D6D1C8")
    frame.set_linewidth(0.9)
    frame.set_alpha(1.0)


def add_metric_panel(
    axis: plt.Axes,
    rows: list[dict[str, object]],
    metric: str,
    *,
    show_ylabel: bool,
) -> None:
    if not rows:
        axis.set_visible(False)
        return

    positions = list(range(len(rows)))
    labels = [runtime_label(str(row["runtime"])) for row in rows]
    values = [float(row["value_mib"]) for row in rows]
    colors = [RUNTIME_COLORS.get(str(row["runtime"]), "#CFCFCF") for row in rows]
    axis.bar(
        positions,
        values,
        width=0.58,
        color=colors,
        hatch=[RUNTIME_HATCHES.get(str(row["runtime"]), "") for row in rows],
        edgecolor=FRAME_COLOR,
        linewidth=0.9,
        zorder=3,
    )

    axis.set_title(METRIC_TITLES[metric], pad=8)
    if show_ylabel:
        axis.set_ylabel("Size (MiB)", labelpad=4)
        axis.yaxis.label.set_visible(True)
    else:
        axis.set_ylabel("")
        axis.yaxis.label.set_visible(False)
    axis.set_xticks(positions)
    axis.set_xticklabels(labels)
    axis.set_yscale("log")
    axis.grid(axis="y", linestyle=(0, (2, 2)), color=GRID_COLOR, zorder=0)
    axis.set_axisbelow(True)
    style_axis_frame(axis)


def plot_space_sizes(rows: list[dict[str, object]], output_path: Path) -> Path:
    if not rows:
        raise ValueError("size CSV is empty")

    fig, axes = plt.subplots(1, 3, figsize=(12.8, 4.4))
    add_metric_panel(
        axes[0],
        rows_for_metric(rows, "artifact_only"),
        "artifact_only",
        show_ylabel=True,
    )
    add_metric_panel(
        axes[1],
        rows_for_metric(rows, "full_deploy_size"),
        "full_deploy_size",
        show_ylabel=False,
    )
    add_metric_panel(
        axes[2],
        rows_for_metric(rows, "runtime_peak_rss"),
        "runtime_peak_rss",
        show_ylabel=False,
    )

    legend = fig.legend(
        handles=runtime_legend(),
        loc="upper center",
        bbox_to_anchor=(0.5, 0.965),
        ncol=3,
        columnspacing=1.8,
        handletextpad=0.7,
    )
    decorate_legend(legend)
    fig.subplots_adjust(left=0.07, right=0.985, bottom=0.14, top=0.81, wspace=0.19)

    pdf_path = output_path.with_suffix(".pdf")
    fig.savefig(pdf_path, bbox_inches="tight")
    plt.close(fig)
    return pdf_path


def main() -> None:
    args = parse_args()
    apply_style()

    input_csv = Path(args.input_csv)
    output_dir = Path(args.output_dir)
    if not input_csv.is_file():
        raise FileNotFoundError(f"size CSV not found: {input_csv}")

    rows = load_rows(input_csv)
    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = plot_space_sizes(rows, output_dir / args.output_name)
    print(f"Generated 1 PDF file: {output_path}")


if __name__ == "__main__":
    main()
