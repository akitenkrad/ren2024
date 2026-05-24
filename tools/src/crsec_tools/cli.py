"""crsec-tools — Ren et al. (2024) CRSEC 社会規範創発 ツール統合 CLI．

Usage:
    crsec-tools visualize [...]
    crsec-tools visualize-sweep [...]
    crsec-tools show-experiment-settings [...]

各サブコマンドに続く引数は，対応するモジュールの argparse がそのまま受け取る．
サブコマンドレベルで `--help` を付けると，そのサブコマンド自身のヘルプが表示される．

`reproduce`（論文 Fig. 2 の一括再現 / descriptive vs injunctive 深掘り）は Phase 3
で実装予定（未提供）．
"""

from __future__ import annotations

import argparse
import sys


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="crsec-tools",
        description="Ren et al. (2024) CRSEC 社会規範創発 可視化・分析ツール",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser(
        "visualize",
        help="単一実行結果（創発曲線・衝突時系列・規範数）の可視化",
        add_help=False,
    )
    subparsers.add_parser(
        "visualize-sweep",
        help="スイープ結果（人口×WS-β の創発時刻・最終採用率）の可視化",
        add_help=False,
    )
    subparsers.add_parser(
        "show-experiment-settings",
        help="実行結果ディレクトリの設定（config / sweep_config / run_metadata）の表示",
        add_help=False,
    )

    argv = sys.argv[1:] if argv is None else argv
    if not argv or argv[0] in {"-h", "--help"}:
        parser.parse_args(argv)
        return

    command = argv[0]
    rest = argv[1:]
    if command == "visualize":
        from crsec_tools.visualize import main as run_main

        run_main(rest)
    elif command == "visualize-sweep":
        from crsec_tools.visualize_sweep import main as run_main

        run_main(rest)
    elif command == "show-experiment-settings":
        from crsec_tools.show_experiment_settings import main as run_main

        run_main(rest)
    else:
        parser.parse_args(argv)


if __name__ == "__main__":
    main()
