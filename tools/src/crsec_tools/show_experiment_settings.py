"""crsec-tools show-experiment-settings — run ディレクトリの設定表示．

runvault の run ディレクトリの `config.json`（封筒．条件は `parameters` の下）を読み，
実行時に使われた全パラメータを整形表示する．`run` か `sweep` / `reproduce` かは
`run.json` の `subcommand` で判別する（`sweep_config.json` はもう書かれない）．
LLM 情報（provider・モデル・温度）は `run.json` の `llm` ブロック，呼び出し数と
cache-hit 率は `metrics.csv` の run スコープ指標から採る（`run_metadata.json` は
書かれない）．

run ディレクトリのパスは次で取れる:
    runvault path --experiment crsec --latest --subcommand run --standalone
    runvault path --experiment crsec --latest --subcommand sweep
    runvault path --experiment crsec --latest --subcommand reproduce

移行前の `results/<timestamp>/` も `--results-dir` に直接渡せば従来どおり読める
（`run.json` を持たないディレクトリは legacy として扱い，`run_metadata.json` があれば
そこから LLM 情報を出す）．

Usage:
    crsec-tools show-experiment-settings
    crsec-tools show-experiment-settings --results-dir "$(runvault path --experiment crsec --latest --subcommand sweep)"
    crsec-tools show-experiment-settings --json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from runvault.read import config_parameters, load_run_meta, run_scope_metrics, runvault_path

# runvault の experiment 名（Rust 側 record::EXPERIMENT と揃える）．
EXPERIMENT = "crsec"

# config キー → 表示ラベル（右コロン位置を揃えるため空白パディング済み）．
FIELD_LABELS = {
    "population": "人口 N           ",
    "population_values": "人口リスト       ",
    "entrepreneurs": "規範起業家       ",
    "network": "ネットワーク     ",
    "ws_k": "WS k             ",
    "ws_beta": "WS β             ",
    "ws_beta_values": "WS-β リスト      ",
    "er_p": "ER p             ",
    "ba_m": "BA m             ",
    "runs": "試行数 runs      ",
    "rounds": "ラウンド T       ",
    "synth_threshold": "統合閾値 θ       ",
    "convergence_window": "収束ウィンドウ K ",
    "emergence_threshold": "創発しきい       ",
    "canonical_mode": "規範同定         ",
    "seed": "シード (コア)    ",
    "mock": "mock             ",
    "llm_temperature": "LLM 温度         ",
    "llm_seed": "LLM seed         ",
    "llm_cache_path": "LLM cache_path   ",
}


def render_config(cfg: dict, source: Path, kind: str) -> str:
    """条件テーブルを整形する（キーの並びは FIELD_LABELS の順）．"""
    lines: list[str] = []
    lines.append("=" * 70)
    lines.append(f"実行設定 ({kind})")
    lines.append("=" * 70)
    lines.append(f"設定ファイル: {source}")
    lines.append("-" * 70)
    for field, label in FIELD_LABELS.items():
        if field in cfg:
            lines.append(f"{label}: {cfg[field]}")
    lines.append("=" * 70)
    return "\n".join(lines)


def render_llm(meta: dict | None, scoped: dict[str, float], legacy: dict | None) -> str | None:
    """LLM 由来情報．

    移行前は `run_metadata.json` が持っていた．provider・モデル・温度は `run.json` の
    `llm` ブロック，呼び出し数と cache-hit 率は run スコープの指標が正本になった．
    legacy ディレクトリでは `run_metadata.json` をそのまま読む．
    """
    lines: list[str] = ["LLM provenance", "-" * 70]
    if meta is not None and meta.get("llm") is not None:
        llm = meta["llm"]
        lines.append(f"provider         : {llm.get('provider', '-')}")
        lines.append(f"model            : {llm.get('model_snapshot', '-')}")
        lines.append(f"temperature      : {llm.get('temperature', '-')}")
        calls = scoped.get("llm_calls")
        if calls is not None:
            hits = scoped.get("llm_cache_hits", 0.0)
            rate = scoped.get("llm_cache_hit_rate")
            rate_text = "-" if rate is None else f"{rate * 100:.1f}%"
            lines.append(f"calls / cache-hit: {int(calls)} / {int(hits)} ({rate_text})")
    elif legacy is not None:
        lines.append(f"model            : {legacy.get('llm_model', '-')}")
        lines.append(f"endpoint         : {legacy.get('llm_endpoint', '-')}")
        lines.append(f"temperature      : {legacy.get('llm_temperature', '-')}")
        rate = legacy.get("cache_hit_rate")
        rate_text = "-" if rate is None else f"{rate * 100:.1f}%"
        lines.append(
            f"calls / cache-hit: {legacy.get('total_calls', '-')} /"
            f" {legacy.get('cache_hits', '-')} ({rate_text})"
        )
    else:
        return None
    lines.append("=" * 70)
    return "\n".join(lines)


def render_run_scope(scoped: dict[str, float]) -> str | None:
    """run 全体を 1 つの値で表す指標（収束・最終ステップ・創発時刻）．

    `time_to_emergence` は創発しなかった run では指標そのものが無い（欠測を -1 で
    埋めない）ので，そのときは «未創発» と出す．
    """
    if not scoped:
        return None
    lines: list[str] = ["run スコープ指標", "-" * 70]
    for name, label in (
        ("n_units", "観測主体数 n_units"),
        ("converged", "収束             "),
        ("final_step", "最終ステップ     "),
    ):
        if name in scoped:
            lines.append(f"{label}: {scoped[name]:g}")
    tte = scoped.get("time_to_emergence")
    lines.append(f"創発時刻         : {'未創発' if tte is None else f'{tte:g}'}")
    lines.append("=" * 70)
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="crsec-tools show-experiment-settings",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--results-dir",
        "--results_dir",
        default=None,
        help="run ディレクトリ (省略時は runvault path が返す直近の run)",
    )
    parser.add_argument(
        "--results-root",
        "--results_root",
        default="results",
        help="runvault の results ルート (default: results)",
    )
    parser.add_argument(
        "--subcommand",
        default="run",
        help="--results-dir 省略時に探すサブコマンド (run / sweep / reproduce)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="表ではなく JSON 形式で出力する．",
    )
    args = parser.parse_args(argv)

    if args.results_dir is None:
        results_dir = Path(
            runvault_path(
                EXPERIMENT,
                args.results_root,
                subcommand=args.subcommand,
                standalone=args.subcommand == "run",
            )
        )
    else:
        results_dir = Path(args.results_dir)
    if not results_dir.exists():
        print(f"エラー: ディレクトリが存在しません: {results_dir}", file=sys.stderr)
        return 1

    cfg = config_parameters(results_dir, required=False)
    if cfg is None:
        print(f"エラー: config.json が見つかりません: {results_dir}", file=sys.stderr)
        return 1
    meta = load_run_meta(results_dir, required=False)
    # legacy の wide な metrics.csv は `step` 列を持たないので run スコープ指標は無い．
    # 同じ値は run_metadata.json 側にある．
    scoped = run_scope_metrics(results_dir) if meta is not None else {}
    kind = meta["subcommand"] if meta is not None else "legacy"

    legacy_meta = None
    legacy_path = results_dir / "run_metadata.json"
    if meta is None and legacy_path.exists():
        legacy_meta = json.loads(legacy_path.read_text())

    if args.json:
        payload = {
            "source": str(results_dir),
            "kind": kind,
            "config": cfg,
            "llm": (meta or {}).get("llm") if meta is not None else legacy_meta,
            "run_scope_metrics": scoped,
        }
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return 0

    print(render_config(cfg, results_dir / "config.json", kind))
    block = render_run_scope(scoped)
    if block is not None:
        print(block)
    block = render_llm(meta, scoped, legacy_meta)
    if block is not None:
        print(block)
    return 0


if __name__ == "__main__":
    sys.exit(main())
