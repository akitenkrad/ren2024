#!/usr/bin/env python3
"""
visualize_sweep.py — Ren et al. (2024) CRSEC スイープ結果 可視化スクリプト

results/latest（または --sweep_dir 指定先）の sweep_summary.csv を読み，
人口 N × WS-β の格子について «創発時刻 (time_to_emergence)» と «最終採用率» を集計し，
ヒートマップと折れ線で可視化する．

Usage:
    uv run crsec-tools visualize-sweep
    uv run crsec-tools visualize-sweep --sweep_dir results/20260524_160000_sweep

Outputs:
    output_dir/
    ├── sweep_time_to_emergence_heatmap.png ← 創発時刻 (N × β) ヒートマップ
    ├── sweep_adoption_heatmap.png          ← 最終採用率 (N × β) ヒートマップ
    └── sweep_curves.png                    ← N / β に対する創発時刻・採用率の折れ線
"""

from __future__ import annotations

import argparse
import os

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

plt.rcParams["font.family"] = "Hiragino Sans"

COLOR_BG = "#FAFAF8"


def load_summary(sweep_dir: str) -> pd.DataFrame:
    """sweep_summary.csv を読み込む．"""
    path = os.path.join(sweep_dir, "sweep_summary.csv")
    if not os.path.exists(path):
        raise FileNotFoundError(f"sweep_summary.csv が見つかりません: {path}")
    return pd.read_csv(path)


def pivot_metric(df: pd.DataFrame, metric: str, agg: str = "mean") -> pd.DataFrame:
    """(population, ws_beta) ごとに metric を集計してピボットする（行=N, 列=β）．"""
    grouped = df.groupby(["population", "ws_beta"])[metric].agg(agg).reset_index()
    table = grouped.pivot(index="population", columns="ws_beta", values=metric)
    return table.sort_index().sort_index(axis=1)


def save_heatmap(table: pd.DataFrame, title: str, out_path: str, cmap: str, fmt: str = "{:.2f}") -> None:
    """population × ws_beta のヒートマップを保存する．"""
    fig, ax = plt.subplots(
        figsize=(1.8 + 1.2 * table.shape[1], 1.6 + 0.8 * table.shape[0]),
        facecolor=COLOR_BG,
    )
    ax.set_facecolor(COLOR_BG)
    data = table.to_numpy(dtype=float)
    im = ax.imshow(data, cmap=cmap, aspect="auto")

    ax.set_xticks(range(table.shape[1]))
    ax.set_xticklabels([f"{b:.2f}" for b in table.columns])
    ax.set_yticks(range(table.shape[0]))
    ax.set_yticklabels([str(p) for p in table.index])
    ax.set_xlabel("WS 再配線確率 β")
    ax.set_ylabel("人口 N")
    ax.set_title(title, fontsize=12)

    for i in range(table.shape[0]):
        for j in range(table.shape[1]):
            v = data[i, j]
            if not np.isnan(v):
                ax.text(j, i, fmt.format(v), ha="center", va="center", fontsize=9, color="black")

    fig.colorbar(im, ax=ax, fraction=0.046, pad=0.04)
    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def save_curves(df: pd.DataFrame, out_path: str) -> None:
    """人口別・β別に創発時刻 / 最終採用率の傾向を折れ線で示す．"""
    fig, axes = plt.subplots(1, 2, figsize=(12, 4.5), facecolor=COLOR_BG)

    # 左: β に対する最終採用率（人口ごとの系列）．
    ax = axes[0]
    ax.set_facecolor(COLOR_BG)
    for pop, sub in df.groupby("population"):
        agg = sub.groupby("ws_beta")["final_adoption_rate"].mean()
        ax.plot(agg.index, agg.to_numpy(), marker="o", label=f"N={pop}")
    ax.set_xlabel("WS 再配線確率 β")
    ax.set_ylabel("最終採用率 (平均)")
    ax.set_ylim(-0.03, 1.05)
    ax.set_title("β ↑ → 創発しやすさ（最終採用率）")
    ax.legend(loc="best")
    ax.grid(True, alpha=0.3)

    # 右: 人口に対する創発時刻（未創発 -1 は除外）．
    ax = axes[1]
    ax.set_facecolor(COLOR_BG)
    valid = df[df["time_to_emergence"] >= 0]
    if not valid.empty:
        agg = valid.groupby("population")["time_to_emergence"].mean()
        ax.plot(agg.index, agg.to_numpy(), color="#2196F3", marker="o")
    ax.set_xlabel("人口 N")
    ax.set_ylabel("創発までのラウンド (平均; 創発した試行のみ)")
    ax.set_title("人口 ↑ → 創発までのラウンド数の傾向")
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="crsec-tools visualize-sweep",
        description="Ren et al. (2024) CRSEC スイープ結果 可視化スクリプト",
    )
    p.add_argument(
        "--sweep_dir",
        "--sweep-dir",
        default="results/latest",
        help="スイープ出力ディレクトリ (default: results/latest)",
    )
    p.add_argument(
        "--output_dir",
        "--output-dir",
        default=None,
        help="図の保存先ディレクトリ (default: {sweep_dir}/figures)",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)

    out_dir = args.output_dir if args.output_dir else os.path.join(args.sweep_dir, "figures")
    os.makedirs(out_dir, exist_ok=True)

    print("=== Ren et al. (2024) CRSEC スイープ可視化 ===")
    print(f"スイープ: {args.sweep_dir}")
    print(f"出力先:   {out_dir}")
    print("-------------------------------------------------")

    print("[1/3] sweep_summary.csv を読み込み中 ...")
    df = load_summary(args.sweep_dir)
    print(f"      人口 {df['population'].nunique()} 種 × β {df['ws_beta'].nunique()} 種")

    print("[2/3] ヒートマップを保存中 ...")
    # 未創発 (-1) は NaN 扱いにして平均から除く．
    df_tte = df.copy()
    df_tte["time_to_emergence"] = df_tte["time_to_emergence"].where(df_tte["time_to_emergence"] >= 0, np.nan)
    save_heatmap(
        pivot_metric(df_tte, "time_to_emergence"),
        "創発時刻 time_to_emergence (人口 × β)",
        os.path.join(out_dir, "sweep_time_to_emergence_heatmap.png"),
        cmap="YlGnBu",
        fmt="{:.1f}",
    )
    save_heatmap(
        pivot_metric(df, "final_adoption_rate"),
        "最終採用率 (人口 × β)",
        os.path.join(out_dir, "sweep_adoption_heatmap.png"),
        cmap="YlOrRd",
    )

    print("[3/3] 折れ線図を保存中 ...")
    save_curves(df, os.path.join(out_dir, "sweep_curves.png"))

    print("-------------------------------------------------")
    print("人口別の平均 最終採用率:")
    for pop, sub in df.groupby("population"):
        print(f"  N={pop:<4} → 採用率̄ = {sub['final_adoption_rate'].mean():.3f}")

    print("-------------------------------------------------")
    print("完了．出力ファイル一覧:")
    for f in sorted(os.listdir(out_dir)):
        size_kb = os.path.getsize(os.path.join(out_dir, f)) / 1024
        print(f"  {f:40s} ({size_kb:6.1f} KB)")


if __name__ == "__main__":
    main()
