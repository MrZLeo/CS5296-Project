#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
from collections import defaultdict
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
WORKLOAD_ORDER = ["noop", "fib", "alloc_touch"]
METRIC_ORDER = ["mean", "p50", "p95"]

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
RUNTIME_SHORT_LABELS = {
    "wasmedge-wasm": "Wasm",
    "wasmedge-aot": "AOT",
    "docker": "Docker",
}
WORKLOAD_TITLES = {
    "noop": "Noop",
    "fib": "Fib",
    "alloc_touch": "AllocTouch",
}
WORKLOAD_PARAMETERS = {
    "noop": "baseline",
    "fib": "n=40",
    "alloc_touch": "64 MiB",
}
METRIC_TITLES = {
    "mean": "Mean",
    "p50": "P50",
    "p95": "P95",
}

BACKGROUND = "#FFFFFF"
PANEL_BACKGROUND = "#FFFFFF"
GRID_COLOR = "#E6E1D9"
TEXT_COLOR = "#1A1A1A"
MUTED_TEXT = "#625D57"
FRAME_COLOR = "#7A746D"
LEGEND_FACE = "#FFFFFF"
COMPONENT_COLORS = {
    "startup": "#E8DFD5",
    "compute": "#CDE5DF",
}
COMPONENT_HATCHES = {
    "startup": "///",
    "compute": "\\\\",
}

LEGACY_OUTPUTS = {
    "README.md",
    "e2e_mean_ms.pdf",
    "e2e_mean_ms.png",
    "e2e_percentiles_ms.pdf",
    "e2e_percentiles_ms.png",
    "startup_mean_ms.pdf",
    "startup_mean_ms.png",
    "runtime_breakdown_ms.pdf",
    "runtime_breakdown_ms.png",
    "e2e_boxplot_ms.pdf",
    "e2e_boxplot_ms.png",
    "startup_boxplot_ms.pdf",
    "startup_boxplot_ms.png",
    "e2e_cdf_ms.pdf",
    "e2e_cdf_ms.png",
}


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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate benchmark presentation-ready PDF plots."
    )
    parser.add_argument(
        "--detail-csv",
        default="target/bench-results/local-bench.csv",
        help="Path to the detailed benchmark CSV.",
    )
    parser.add_argument(
        "--summary-csv",
        default="target/bench-results/summary.csv",
        help="Path to the summary benchmark CSV. Kept for CLI compatibility.",
    )
    parser.add_argument(
        "--output-dir",
        default="target/bench-results/plots",
        help="Directory to write generated PDF plots into.",
    )
    return parser.parse_args()


def runtime_label(runtime: str) -> str:
    return RUNTIME_SHORT_LABELS.get(runtime, runtime)


def workload_label(workload: str) -> str:
    return WORKLOAD_TITLES.get(workload, workload)


def load_detail(path: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    with path.open(newline="") as handle:
        for row in csv.DictReader(handle):
            rows.append(
                {
                    "runtime": row["runtime"],
                    "workload": row["workload"],
                    "parameter": row["parameter"],
                    "sample": int(row["sample"]),
                    "e2e_ms": float(row["e2e_ms"]),
                    "internal_compute_ms": float(row["internal_compute_ms"]),
                    "startup_overhead_ms": float(row["startup_overhead_ms"]),
                    "exit_code": int(row["exit_code"]),
                }
            )
    return rows


def build_detail_groups(
    detail_rows: list[dict[str, object]]
) -> dict[tuple[str, str], list[dict[str, object]]]:
    grouped: dict[tuple[str, str], list[dict[str, object]]] = defaultdict(list)
    for row in detail_rows:
        grouped[(str(row["runtime"]), str(row["workload"]))].append(row)
    return grouped


def detail_workloads(detail_groups: dict[tuple[str, str], list[dict[str, object]]]) -> list[str]:
    return [
        workload
        for workload in WORKLOAD_ORDER
        if any((runtime, workload) in detail_groups for runtime in RUNTIME_ORDER)
    ]


def mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def percentile_nearest_rank(values: list[float], percentile: int) -> float:
    if not values:
        return 0.0
    sorted_values = sorted(values)
    rank = ((percentile * len(sorted_values)) + 99) // 100
    index = max(0, min(rank - 1, len(sorted_values) - 1))
    return sorted_values[index]


def compute_metric_stats(
    detail_groups: dict[tuple[str, str], list[dict[str, object]]],
    metric_key: str,
) -> dict[tuple[str, str], dict[str, float]]:
    stats: dict[tuple[str, str], dict[str, float]] = {}
    for key, rows in detail_groups.items():
        values = [float(row[metric_key]) for row in rows]
        stats[key] = {
            "mean": mean(values),
            "p50": percentile_nearest_rank(values, 50),
            "p95": percentile_nearest_rank(values, 95),
        }
    return stats


def style_axis_frame(axis: plt.Axes) -> None:
    for side in ["left", "right", "top", "bottom"]:
        axis.spines[side].set_visible(True)
        axis.spines[side].set_color(FRAME_COLOR)
        axis.spines[side].set_linewidth(1.0)


def save_pdf(fig: plt.Figure, output_path: Path) -> str:
    pdf_path = output_path.with_suffix(".pdf")
    fig.savefig(pdf_path, bbox_inches="tight")
    return pdf_path.name


def clean_legacy_outputs(output_dir: Path) -> None:
    for name in LEGACY_OUTPUTS:
        (output_dir / name).unlink(missing_ok=True)


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


def component_legend() -> list[Patch]:
    return [
        Patch(
            facecolor=COMPONENT_COLORS["startup"],
            hatch=COMPONENT_HATCHES["startup"],
            edgecolor=FRAME_COLOR,
            linewidth=0.9,
            label="Startup",
        ),
        Patch(
            facecolor=COMPONENT_COLORS["compute"],
            hatch=COMPONENT_HATCHES["compute"],
            edgecolor=FRAME_COLOR,
            linewidth=0.9,
            label="Compute",
        ),
    ]


def decorate_legend(legend: plt.Legend) -> None:
    frame = legend.get_frame()
    frame.set_facecolor(LEGEND_FACE)
    frame.set_edgecolor("#D6D1C8")
    frame.set_linewidth(0.9)
    frame.set_alpha(1.0)


def plot_latency_overview(
    detail_groups: dict[tuple[str, str], list[dict[str, object]]],
    output_path: Path,
    *,
    metric_key: str,
    ylabel: str,
    use_log_scale: bool,
) -> list[str]:
    workloads = detail_workloads(detail_groups)
    if not workloads:
        return []

    stats = compute_metric_stats(detail_groups, metric_key)
    fig, axes = plt.subplots(1, len(METRIC_ORDER), figsize=(12.8, 4.4), sharey=True)
    if len(METRIC_ORDER) == 1:
        axes = [axes]

    bar_width = 0.22
    group_centers = list(range(len(workloads)))
    runtime_offset = (len(RUNTIME_ORDER) - 1) / 2

    for axis, metric_name in zip(axes, METRIC_ORDER, strict=True):
        for runtime_index, runtime in enumerate(RUNTIME_ORDER):
            positions: list[float] = []
            values: list[float] = []
            for center, workload in zip(group_centers, workloads, strict=True):
                key = (runtime, workload)
                if key not in stats:
                    continue
                positions.append(center + ((runtime_index - runtime_offset) * bar_width))
                values.append(stats[key][metric_name])

            axis.bar(
                positions,
                values,
                width=bar_width * 0.9,
                color=RUNTIME_COLORS[runtime],
                hatch=RUNTIME_HATCHES[runtime],
                edgecolor=FRAME_COLOR,
                linewidth=0.9,
                zorder=3,
            )

        axis.set_title(METRIC_TITLES[metric_name], pad=4)
        axis.set_xticks(group_centers)
        axis.set_xticklabels([workload_label(workload) for workload in workloads])
        axis.grid(axis="y", linestyle=(0, (2, 2)), color=GRID_COLOR, zorder=0)
        axis.set_axisbelow(True)
        if use_log_scale:
            axis.set_yscale("log")
        style_axis_frame(axis)

    axes[0].set_ylabel(ylabel)
    legend = fig.legend(
        handles=runtime_legend(),
        loc="upper center",
        bbox_to_anchor=(0.5, 0.955),
        ncol=3,
        columnspacing=1.8,
        handletextpad=0.7,
    )
    decorate_legend(legend)
    fig.subplots_adjust(left=0.07, right=0.985, bottom=0.14, top=0.81, wspace=0.16)
    output_name = save_pdf(fig, output_path)
    plt.close(fig)
    return [output_name]


def plot_breakdown(
    detail_groups: dict[tuple[str, str], list[dict[str, object]]],
    output_path: Path,
) -> list[str]:
    workloads = detail_workloads(detail_groups)
    if not workloads:
        return []

    fig, axes = plt.subplots(1, len(workloads), figsize=(12.8, 4.5))
    if len(workloads) == 1:
        axes = [axes]

    for index, (axis, workload) in enumerate(zip(axes, workloads, strict=True)):
        runtimes = [runtime for runtime in RUNTIME_ORDER if (runtime, workload) in detail_groups]
        positions = list(range(len(runtimes)))
        startup_values = [
            mean([float(row["startup_overhead_ms"]) for row in detail_groups[(runtime, workload)]])
            for runtime in runtimes
        ]
        compute_values = [
            mean([float(row["internal_compute_ms"]) for row in detail_groups[(runtime, workload)]])
            for runtime in runtimes
        ]

        axis.bar(
            positions,
            startup_values,
            width=0.52,
            color=COMPONENT_COLORS["startup"],
            hatch=COMPONENT_HATCHES["startup"],
            edgecolor=FRAME_COLOR,
            linewidth=0.9,
            zorder=3,
        )
        axis.bar(
            positions,
            compute_values,
            width=0.52,
            bottom=startup_values,
            color=COMPONENT_COLORS["compute"],
            hatch=COMPONENT_HATCHES["compute"],
            edgecolor=FRAME_COLOR,
            linewidth=0.9,
            zorder=3,
        )

        axis.set_title(workload_label(workload), pad=4)
        axis.set_xticks(positions)
        axis.set_xticklabels([runtime_label(runtime) for runtime in runtimes])
        axis.grid(axis="y", linestyle=(0, (2, 2)), color=GRID_COLOR, zorder=0)
        axis.set_axisbelow(True)
        if index == 0:
            axis.set_ylabel("Latency (ms)")
        style_axis_frame(axis)

    legend = fig.legend(
        handles=component_legend(),
        loc="upper center",
        bbox_to_anchor=(0.5, 0.955),
        ncol=2,
        columnspacing=1.8,
        handletextpad=0.7,
    )
    decorate_legend(legend)
    fig.subplots_adjust(left=0.07, right=0.985, bottom=0.14, top=0.81, wspace=0.2)
    output_name = save_pdf(fig, output_path)
    plt.close(fig)
    return [output_name]


def main() -> None:
    args = parse_args()
    apply_style()

    detail_csv = Path(args.detail_csv)
    summary_csv = Path(args.summary_csv)
    output_dir = Path(args.output_dir)

    if not detail_csv.is_file():
        raise FileNotFoundError(f"detail CSV not found: {detail_csv}")
    if summary_csv.exists() and not summary_csv.is_file():
        raise FileNotFoundError(f"summary CSV is not a file: {summary_csv}")

    detail_rows = load_detail(detail_csv)
    detail_groups = build_detail_groups(detail_rows)

    output_dir.mkdir(parents=True, exist_ok=True)
    clean_legacy_outputs(output_dir)

    generated: list[str] = []
    generated.extend(
        plot_latency_overview(
            detail_groups,
            output_dir / "e2e_overview",
            metric_key="e2e_ms",
            ylabel="Latency (ms)",
            use_log_scale=True,
        )
    )
    generated.extend(
        plot_latency_overview(
            detail_groups,
            output_dir / "startup_overview",
            metric_key="startup_overhead_ms",
            ylabel="Latency (ms)",
            use_log_scale=False,
        )
    )
    generated.extend(plot_breakdown(detail_groups, output_dir / "breakdown"))

    print(f"Generated {len(generated)} PDF files in {output_dir}")
    for filename in generated:
        print(f"- {output_dir / filename}")


if __name__ == "__main__":
    main()
