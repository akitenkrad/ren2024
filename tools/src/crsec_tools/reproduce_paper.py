#!/usr/bin/env python3
"""reproduce_paper.py — Ren et al. (2024) CRSEC 見出し的知見の一括再現レポート + 図．

Rust の `crsec reproduce` が書き出す `reproduce_summary.json`（試行平均セル・論文知見
アンカー）と代表 run の `metrics.csv` を読み，CRSEC の中心的知見を 2 つの図で可視化
しつつ PASS/off テーブルを表示する:

    1. emergence_trajectory.png
       代表 run の «採用率・遵守率»，«相異 canonical 規範数»，«社会的衝突数» のラウンド
       推移．社会規範の創発（採用率↑）・統合（相異規範数の縮約）・衝突の rise-then-fall
       を一望する（論文 Section 3 / Fig. 2 風）．
    2. descriptive_vs_injunctive.png
       descriptive vs injunctive 深掘り．命令的（injunctive）規範と記述的（descriptive）
       規範の «型別採用率トラジェクトリ» を重ね描きし，論文 Fact 7「命令的規範が記述的
       規範より先に創発する」を可視化する．型別の相異規範数も併記する．

`--run` を付けると先に Rust バイナリ（`cargo run --release -- reproduce`）を実行して
最新結果を生成する．サンドボックス・CI では `--mock` も付けてライブ LLM を回避する．

Usage:
    uv run crsec-tools reproduce --run --mock          # mock で一括再現 + 図
    uv run crsec-tools reproduce --run --mock --quick  # 軽量版（動作確認用）
    uv run crsec-tools reproduce                        # 既存 results/latest を可視化
    uv run crsec-tools reproduce --results-dir results/reproduce_20260530_000000
    uv run crsec-tools reproduce --json

Outputs:
    {results_dir}/figures/{emergence_trajectory,descriptive_vs_injunctive}.png
    stdout: アンカーごとの PASS / OFF．
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd

from socsim_tools.io import resolve_results_dir

# --------------------------------------------------------------------------- #
# 表示設定（CJK フォントが利用不能でも落ちないように try）
# --------------------------------------------------------------------------- #
try:
    plt.rcParams["font.family"] = "Hiragino Sans"
except Exception:  # pragma: no cover - フォント未インストール環境用フォールバック
    pass

COLOR_BG = "#FAFAF8"
COLOR_ADOPT = "#2196F3"
COLOR_COMPLY = "#4CAF50"
COLOR_CONFLICT = "#F44336"
COLOR_NORMS = "#9C27B0"
COLOR_INJ = "#E64A19"
COLOR_DES = "#1565C0"


# --------------------------------------------------------------------------- #
# Rust バイナリ実行
# --------------------------------------------------------------------------- #


def _run_binary(*, mock: bool, quick: bool, seed: int, output_dir: str) -> None:
    """`cargo run --release -- reproduce ...` を実行して最新結果を生成する．"""
    cmd = ["cargo", "run", "--release", "--", "reproduce",
           "--seed", str(seed), "--output-dir", output_dir]
    if mock:
        cmd.append("--mock")
    if quick:
        cmd.append("--quick")
    print(f"$ {' '.join(cmd)}")
    subprocess.run(cmd, check=True)


def _load_summary(results_dir: Path) -> dict:
    path = results_dir / "reproduce_summary.json"
    if not path.exists():
        raise FileNotFoundError(
            f"reproduce_summary.json が見つかりません: {path}\n"
            f"  先に `crsec-tools reproduce --run --mock` を実行してください．"
        )
    with path.open(encoding="utf-8") as f:
        return json.load(f)


# --------------------------------------------------------------------------- #
# 描画
# --------------------------------------------------------------------------- #


def _emergence_trajectory(results_dir: Path, out_path: Path) -> None:
    """代表 run の創発曲線・規範統合・衝突 rise-then-fall（3 段）．"""
    path = results_dir / "metrics.csv"
    if not path.exists():
        print(f"  警告: metrics.csv が無いため emergence_trajectory をスキップ ({path})")
        return
    df = pd.read_csv(path)
    t = df["t"]

    fig, axes = plt.subplots(3, 1, figsize=(9, 10), facecolor=COLOR_BG, sharex=True)
    fig.suptitle(
        "Ren et al. (2024) CRSEC — 社会規範の創発・統合・衝突（代表 run）",
        fontsize=13,
    )

    ax = axes[0]
    ax.set_facecolor(COLOR_BG)
    ax.plot(t, df["adoption_rate"], color=COLOR_ADOPT, lw=2.2, marker="o", ms=3,
            label="採用率 (adoption)")
    ax.plot(t, df["compliance_rate"], color=COLOR_COMPLY, lw=2.2, marker="s", ms=3,
            label="遵守率 (compliance)")
    ax.axhline(0.9, color="#888888", lw=0.9, ls="--", label="創発しきい 0.9")
    ax.set_ylabel("割合")
    ax.set_ylim(-0.05, 1.05)
    ax.set_title("創発: 採用率が高水準へ上昇", fontsize=11)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)

    ax = axes[1]
    ax.set_facecolor(COLOR_BG)
    ax.plot(t, df["n_distinct_norms"], color=COLOR_NORMS, lw=2.2, marker="d", ms=3,
            label="相異 canonical 規範数")
    ax.set_ylabel("規範数")
    ax.set_title("統合: 相異規範数がピークから縮約", fontsize=11)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)

    ax = axes[2]
    ax.set_facecolor(COLOR_BG)
    ax.plot(t, df["n_conflicts"], color=COLOR_CONFLICT, lw=2.2, marker="^", ms=3,
            label="社会的衝突数")
    ax.set_xlabel("時刻 t（ラウンド）")
    ax.set_ylabel("衝突数")
    ax.set_title("衝突: rise-then-fall（初期急増→減衰）", fontsize=11)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


def _descriptive_vs_injunctive(summary: dict, results_dir: Path, out_path: Path) -> None:
    """型別採用率トラジェクトリ + 型別相異規範数（Fact 7 の深掘り）．"""
    path = results_dir / "metrics.csv"
    if not path.exists():
        print(f"  警告: metrics.csv が無いため descriptive_vs_injunctive をスキップ ({path})")
        return
    df = pd.read_csv(path)
    t = df["t"]
    cell = summary.get("cell", {})
    tte_inj = cell.get("mean_tte_injunctive")
    tte_des = cell.get("mean_tte_descriptive")

    fig, axes = plt.subplots(1, 2, figsize=(13, 5), facecolor=COLOR_BG)
    fig.suptitle(
        "Ren et al. (2024) CRSEC — descriptive vs injunctive 深掘り（Fact 7）",
        fontsize=13,
    )

    ax = axes[0]
    ax.set_facecolor(COLOR_BG)
    ax.plot(t, df["adoption_injunctive"], color=COLOR_INJ, lw=2.4, marker="o", ms=3,
            label="命令的 (injunctive)")
    ax.plot(t, df["adoption_descriptive"], color=COLOR_DES, lw=2.4, marker="s", ms=3,
            label="記述的 (descriptive)")
    ax.axhline(0.9, color="#888888", lw=0.9, ls="--", label="創発しきい 0.9")
    if tte_inj is not None:
        ax.axvline(tte_inj, color=COLOR_INJ, lw=1.2, ls=":", alpha=0.8)
    if tte_des is not None:
        ax.axvline(tte_des, color=COLOR_DES, lw=1.2, ls=":", alpha=0.8)
    ax.set_xlabel("時刻 t（ラウンド）")
    ax.set_ylabel("型別 採用率")
    ax.set_ylim(-0.05, 1.05)
    ax.set_title("命令的規範が記述的規範より先に創発する", fontsize=11)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)

    ax = axes[1]
    ax.set_facecolor(COLOR_BG)
    ax.plot(t, df["n_distinct_injunctive"], color=COLOR_INJ, lw=2.2, marker="o", ms=3,
            label="命令的 相異規範数")
    ax.plot(t, df["n_distinct_descriptive"], color=COLOR_DES, lw=2.2, marker="s", ms=3,
            label="記述的 相異規範数")
    ax.set_xlabel("時刻 t（ラウンド）")
    ax.set_ylabel("型別 相異 canonical 規範数")
    ax.set_title("型別の規範多様性の推移", fontsize=11)
    ax.legend(fontsize=9)
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(out_path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"  保存: {out_path}")


# --------------------------------------------------------------------------- #
# レポート出力
# --------------------------------------------------------------------------- #


def _print_report(summary: dict, results_dir: Path) -> None:
    print("=" * 78)
    print("Ren et al. (2024) CRSEC — 見出し的知見 一括再現レポート")
    print(f"  source: {results_dir}  (mode={summary.get('mode', '?')})")
    print("=" * 78)

    cell = summary.get("cell", {})
    print("\n[集計（試行平均）]")
    print(f"  最終 採用率̄         : {cell.get('mean_final_adoption', 0):.3f}")
    print(f"  最終 遵守率̄         : {cell.get('mean_final_compliance', 0):.3f}")
    print(f"  相異規範数 ピーク→最終 : {cell.get('mean_peak_distinct', 0):.2f}"
          f" → {cell.get('mean_final_distinct', 0):.2f}")
    print(f"  衝突 ピーク→最終      : {cell.get('mean_peak_conflicts', 0):.2f}"
          f" → {cell.get('mean_final_conflicts', 0):.2f}")
    print(f"  創発時刻̄ inj / des    : {cell.get('mean_tte_injunctive', 0):.2f}"
          f" / {cell.get('mean_tte_descriptive', 0):.2f}  (Fact 7: inj が先)")
    print(f"  最終採用率̄ inj / des  : {cell.get('mean_final_adoption_injunctive', 0):.3f}"
          f" / {cell.get('mean_final_adoption_descriptive', 0):.3f}")

    print("\n[論文知見アンカー（観測 vs 論文）]")
    n_pass = 0
    for a in summary["anchors"]:
        hi = a["target_hi"]
        hi_str = "∞" if hi is None or hi > 1e30 else f"{hi:.3f}"
        status = "PASS" if a["pass"] else "OFF "
        if a["pass"]:
            n_pass += 1
        print(f"  [{status}] {a['name']:<56} obs={a['observed']:.4f} "
              f"target=[{a['target_lo']:.3f},{hi_str}] paper={a['paper']}")
    print("-" * 78)
    print(f"{n_pass}/{len(summary['anchors'])} アンカーが in-band")
    print("(中核知見: 社会規範の創発 / 規範の統合 / 衝突 rise-then-fall / "
          "Fact 7 命令的→記述的の創発順序)")


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="crsec-tools reproduce",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--results-dir", "--results_dir", default=None,
                        help="reproduce_summary.json のあるディレクトリ（既定: results/latest）")
    parser.add_argument("--output-dir", "--output_dir", default=None,
                        help="図の保存先（既定: {results_dir}/figures）")
    parser.add_argument("--run", action="store_true",
                        help="先に Rust バイナリ（reproduce）を実行する．")
    parser.add_argument("--mock", action="store_true",
                        help="--run 時にライブ LLM を使わず mock で駆動する．")
    parser.add_argument("--quick", action="store_true",
                        help="--run 時に軽量モードで実行する（動作確認用）．")
    parser.add_argument("--seed", type=int, default=42, help="--run 時のシード基点．")
    parser.add_argument("--cargo-output-dir", "--cargo_output_dir", default="results",
                        help="--run 時に cargo の --output-dir へ渡すパス（既定: results）．")
    parser.add_argument("--json", action="store_true", help="JSON 形式で要約を出力する．")
    args = parser.parse_args(argv)

    if args.run:
        _run_binary(mock=args.mock, quick=args.quick, seed=args.seed,
                    output_dir=args.cargo_output_dir)

    results_dir = resolve_results_dir(args.results_dir)
    try:
        summary = _load_summary(results_dir)
    except FileNotFoundError as exc:
        print(f"エラー: {exc}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(summary, indent=2, ensure_ascii=False))
        return 0

    _print_report(summary, results_dir)

    out_dir = Path(args.output_dir) if args.output_dir else results_dir / "figures"
    os.makedirs(out_dir, exist_ok=True)
    print(f"\n[図] 出力先: {out_dir}")
    _emergence_trajectory(results_dir, out_dir / "emergence_trajectory.png")
    _descriptive_vs_injunctive(summary, results_dir, out_dir / "descriptive_vs_injunctive.png")

    print("-" * 78)
    return 0


if __name__ == "__main__":
    sys.exit(main())
