#!/usr/bin/env python3
"""
visualize.py — Ren et al. (2024) CRSEC 社会規範創発 再現実験 可視化スクリプト

results/latest（または --results_dir 指定先）の metrics.csv を読み，
(1) 創発曲線（採用率・遵守率のラウンド推移; 論文 Fig. 2 風），
(2) 社会的衝突数の時系列（初期急増→減少を期待），
(3) 相異 canonical 規範数の時系列（多様性・収束の指標）
を生成する．

Usage:
    uv run crsec-tools visualize
    uv run crsec-tools visualize --results_dir results/20260524_153000
    uv run crsec-tools visualize --output_dir out

Outputs:
    output_dir/
    ├── emergence_curves.png  ← 採用率・遵守率のラウンド推移
    ├── conflicts_timeseries.png ← 社会的衝突数の時系列
    └── distinct_norms.png    ← 相異 canonical 規範数の時系列
"""

from __future__ import annotations

import argparse
import os

import matplotlib.pyplot as plt
import pandas as pd

# --------------------------------------------------------------------------- #
# 日本語フォント設定
# --------------------------------------------------------------------------- #
plt.rcParams["font.family"] = "Hiragino Sans"

# --------------------------------------------------------------------------- #
# カラー設定
# --------------------------------------------------------------------------- #
COLOR_BG = "#FAFAF8"
COLOR_ADOPT = "#2196F3"
COLOR_COMPLY = "#4CAF50"
COLOR_CONFLICT = "#F44336"
COLOR_NORMS = "#9C27B0"


def load_metrics(path: str) -> pd.DataFrame:
    """metrics.csv を読み込む．"""
    if not os.path.exists(path):
        raise FileNotFoundError(f"metrics.csv が見つかりません: {path}")
    return pd.read_csv(path)


def save_emergence_curves(df: pd.DataFrame, out_path: str) -> None:
    """採用率・遵守率のラウンド推移（創発曲線; 論文 Fig. 2 風）を保存する．"""
    fig, ax = plt.subplots(figsize=(9, 5.5), facecolor=COLOR_BG)
    ax.set_facecolor(COLOR_BG)
    t = df["t"]
    ax.plot(t, df["adoption_rate"], color=COLOR_ADOPT, lw=2.2, marker="o", ms=4, label="採用率 (adoption)")
    ax.plot(t, df["compliance_rate"], color=COLOR_COMPLY, lw=2.2, marker="s", ms=4, label="遵守率 (compliance)")
    ax.axhline(0.9, color="#888888", lw=0.9, linestyle="--", label="創発しきい 0.9")
    ax.set_xlabel("ラウンド t")
    ax.set_ylabel("割合 ∈ [0, 1]")
    ax.set_ylim(-0.03, 1.05)
    ax.set_title("社会規範の創発曲線 (採用率・遵守率のラウンド推移)", fontsize=12)
    ax.legend(loc="lower right")
    ax.grid(True, alpha=0.3)
    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def save_conflicts_timeseries(df: pd.DataFrame, out_path: str) -> None:
    """社会的衝突数の時系列（初期急増→減少を期待）を保存する．"""
    fig, ax = plt.subplots(figsize=(9, 4.5), facecolor=COLOR_BG)
    ax.set_facecolor(COLOR_BG)
    ax.plot(df["t"], df["n_conflicts"], color=COLOR_CONFLICT, lw=2.2, marker=".")
    ax.fill_between(df["t"], df["n_conflicts"], color=COLOR_CONFLICT, alpha=0.15)
    ax.set_xlabel("ラウンド t")
    ax.set_ylabel("社会的衝突数 (DetectConflict=T)")
    ax.set_title("社会的衝突数の時系列 (初期急増 → 減少が論文の知見)", fontsize=12)
    ax.grid(True, alpha=0.3)
    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def save_distinct_norms(df: pd.DataFrame, out_path: str) -> None:
    """相異 canonical 規範数の時系列（多様性・収束の指標）を保存する．"""
    fig, ax = plt.subplots(figsize=(9, 4.5), facecolor=COLOR_BG)
    ax.set_facecolor(COLOR_BG)
    ax.plot(df["t"], df["n_distinct_norms"], color=COLOR_NORMS, lw=2.2, marker=".", label="相異規範数")
    if "n_qualified_holders" in df.columns:
        ax.plot(
            df["t"],
            df["n_qualified_holders"],
            color="#FF9800",
            lw=1.6,
            marker=".",
            linestyle="--",
            label="適格保有者数",
        )
    ax.set_xlabel("ラウンド t")
    ax.set_ylabel("件数 / 人数")
    ax.set_title("相異 canonical 規範数と適格保有者数の時系列", fontsize=12)
    ax.legend(loc="best")
    ax.grid(True, alpha=0.3)
    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="crsec-tools visualize",
        description="Ren et al. (2024) CRSEC 社会規範創発 可視化スクリプト",
    )
    p.add_argument(
        "--results_dir",
        "--results-dir",
        default="results/latest",
        help="Rust シミュレーションの出力ディレクトリ (default: results/latest)",
    )
    p.add_argument(
        "--output_dir",
        "--output-dir",
        default=None,
        help="図の保存先ディレクトリ (default: {results_dir}/figures)",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)

    metrics_path = os.path.join(args.results_dir, "metrics.csv")
    out_dir = args.output_dir if args.output_dir else os.path.join(args.results_dir, "figures")
    os.makedirs(out_dir, exist_ok=True)

    print("=== Ren et al. (2024) CRSEC 社会規範創発 可視化 ===")
    print(f"メトリクス: {metrics_path}")
    print(f"出力先:     {out_dir}")
    print("-----------------------------------------")

    df = load_metrics(metrics_path)
    print(f"      {len(df)} ラウンド分のメトリクス")

    print("[1/3] 創発曲線（採用率・遵守率）を保存中 ...")
    save_emergence_curves(df, os.path.join(out_dir, "emergence_curves.png"))

    print("[2/3] 社会的衝突数の時系列を保存中 ...")
    save_conflicts_timeseries(df, os.path.join(out_dir, "conflicts_timeseries.png"))

    print("[3/3] 相異規範数の時系列を保存中 ...")
    save_distinct_norms(df, os.path.join(out_dir, "distinct_norms.png"))

    print("-----------------------------------------")
    print("完了．出力ファイル一覧:")
    for f in sorted(os.listdir(out_dir)):
        size_kb = os.path.getsize(os.path.join(out_dir, f)) / 1024
        print(f"  {f:35s} ({size_kb:6.1f} KB)")


if __name__ == "__main__":
    main()
