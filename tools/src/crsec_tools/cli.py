"""crsec-tools — Ren et al. (2024) CRSEC 社会規範創発 ツール統合 CLI．

Usage:
    crsec-tools visualize [...]
    crsec-tools visualize-sweep [...]
    crsec-tools show-experiment-settings [...]

各サブコマンドに続く引数は，対応するモジュールの argparse がそのまま受け取る．
サブコマンドレベルで `--help` を付けると，そのサブコマンド自身のヘルプが表示される．

`reproduce`（論文 Fig. 2 の一括再現 / descriptive vs injunctive 深掘り）は Phase 3
で実装予定（未提供）．

dispatcher の組み立ては共有ヘルパ `socsim_tools.cli.build_dispatcher` に委譲する
（prog 名・サブコマンド・ヘルプ文・argv ルーティングは従来と同一）．可視化/設定表示の
実体（visualize / visualize_sweep / show_experiment_settings）は repo 固有のまま．
"""

from __future__ import annotations

from socsim_tools.cli import build_dispatcher

main = build_dispatcher(
    prog="crsec-tools",
    description="Ren et al. (2024) CRSEC 社会規範創発 可視化・分析ツール",
    subcommands={
        "visualize": (
            "単一実行結果（創発曲線・衝突時系列・規範数）の可視化",
            "crsec_tools.visualize:main",
        ),
        "visualize-sweep": (
            "スイープ結果（人口×WS-β の創発時刻・最終採用率）の可視化",
            "crsec_tools.visualize_sweep:main",
        ),
        "show-experiment-settings": (
            "実行結果ディレクトリの設定（config / sweep_config / run_metadata）の表示",
            "crsec_tools.show_experiment_settings:main",
        ),
    },
)


if __name__ == "__main__":
    main()
