"""crsec-tools — Ren et al. (2024) CRSEC 社会規範創発 ツール統合 CLI．

Usage:
    crsec-tools visualize [...]
    crsec-tools visualize-sweep [...]
    crsec-tools show-experiment-settings [...]
    crsec-tools reproduce [...]

各サブコマンドに続く引数は，対応するモジュールの argparse がそのまま受け取る．
サブコマンドレベルで `--help` を付けると，そのサブコマンド自身のヘルプが表示される．

`reproduce` は論文の見出し的知見（社会規範の創発・統合・衝突 rise-then-fall・Fact 7 の
命令的→記述的の創発順序）を一括再現し，観測 vs 論文の PASS/off と図を生成する．

dispatcher の組み立ては共有ヘルパ `socsim_tools.cli.build_dispatcher` に委譲する
（prog 名・サブコマンド・ヘルプ文・argv ルーティングは従来と同一）．可視化/設定表示/
再現の実体（visualize / visualize_sweep / show_experiment_settings / reproduce_paper）は
repo 固有のまま．
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
        "reproduce": (
            "論文の見出し的知見（規範の創発・統合・衝突 rise-then-fall・Fact 7）の一括再現 + 図",
            "crsec_tools.reproduce_paper:main",
        ),
    },
)


if __name__ == "__main__":
    main()
