#!/usr/bin/env python3
"""スイープの «1 行 1 (セル × 試行)» の表．

run ディレクトリの読み方そのものは `runvault.read` にある．ここに残るのは CRSEC 固有の
部分だけ — どの列を持つ表なのか（`final_adoption_rate` / `peak_conflicts` …）である．

runvault はこの表をディスクに持たない．sweep 親の子 run（`lineage.parent_run_uid` が
親の `run_uid`）を集め，各子の `config.json` の `parameters`・`run.json` の `rng`・
`metrics.csv` の最終ステップ / 全ステップの最大 / run スコープ指標から組み直す．列は
移行前の `sweep_summary.csv` と同じにしてある．

1 つだけ値の表し方が変わった: `time_to_emergence` は «創発しなかった» を旧 CSV では
`-1` という番兵で表していたが，runvault では **行そのものを書かない**（率や時刻は
欠測を 0 や -1 で埋めない）．ここでは `NaN` になる．
"""

from __future__ import annotations

import os

import pandas as pd
from runvault.read import (
    config_parameters,
    load_run_meta,
    metrics_wide,
    run_scope_metrics,
    sweep_children,
)

__all__ = ["sweep_summary_table"]

#: `parameters` から作る列．
_PARAMETER_COLUMNS = ["population", "ws_beta", "network"]

#: 最終ステップの値から作る列（列名 → metrics.csv の指標名）．
_FINAL_COLUMNS = {
    "final_adoption_rate": "adoption_rate",
    "final_compliance_rate": "compliance_rate",
    "final_n_distinct_norms": "n_distinct_norms",
}

#: run スコープ指標から作る列（列名 → 指標名）．
_SCOPE_COLUMNS = {
    "converged": "converged",
    "final_step": "final_step",
    "time_to_emergence": "time_to_emergence",
    "cache_hit_rate": "llm_cache_hit_rate",
}


def sweep_summary_table(sweep_dir: str | os.PathLike) -> pd.DataFrame:
    """1 行 1 (セル × 試行) のサマリ表を組み直す．

    どの行も `run_dir` を持つので，呼び出し側は条件からディレクトリ名を組み立てなくてよい．
    """
    children = sweep_children(sweep_dir)
    if not children:
        raise SystemExit(
            f"エラー: この sweep 親に紐づく子 run が見つかりません: {sweep_dir}\n"
            "  子 run は lineage.parent_run_uid で親を指します．"
            "親と子が同じ results ルートにあるか確認してください．"
        )

    rows: list[dict] = []
    for child in children:
        params = config_parameters(child) or {}
        rng = (load_run_meta(child) or {}).get("rng") or {}
        scoped = run_scope_metrics(child)
        steps = metrics_wide(os.path.join(child, "metrics.csv"))
        last = steps.iloc[-1]

        row: dict = {key: params.get(key) for key in _PARAMETER_COLUMNS}
        # 同一セルの何本目かは runvault の rng.replicate_index が持つ．
        row["run"] = rng.get("replicate_index")
        row["seed"] = rng.get("master_seed")
        row.update({column: scoped.get(name) for column, name in _SCOPE_COLUMNS.items()})
        row.update({column: float(last[name]) for column, name in _FINAL_COLUMNS.items()})
        # 衝突のピークは «全ステップの最大» なので，時系列そのものから採る（同じ数を
        # 指標としても持たせると 2 箇所に置くことになる）．
        row["peak_conflicts"] = float(steps["n_conflicts"].max())
        row["run_dir"] = child
        rows.append(row)

    return (
        pd.DataFrame(rows)
        .sort_values(["population", "ws_beta", "run"])
        .reset_index(drop=True)
    )
