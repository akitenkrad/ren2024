# CLI

[English](cli.md) | [日本語](cli.ja.md)

Rust バイナリは `crsec`（`cargo run --release -- <subcommand>`）．利用可能なサブコマンドは `run` / `sweep` / `reproduce` の 3 つ．

出力の置き場と同一性は [runvault](https://github.com/akitenkrad/rs-runvault) が持つ．タイムスタンプ付きディレクトリも `results/latest` シンボリックリンクもこちらでは作らず，`Run::start` が決めた run ディレクトリ（`<results-root>/crsec/<subcommand>_<timestamp>_<config_hash>_<uid>/`）へ書く．`--output-dir` は results ルートを指す．run ディレクトリのパスは次で取れる:

```bash
runvault path --experiment crsec --latest --subcommand run --standalone
runvault path --experiment crsec --latest --subcommand sweep
runvault path --experiment crsec --latest --subcommand reproduce
```

`run` と `reproduce` は `--mock` でパイプライン全体を決定論的 scripted クライアントで駆動できる（live LLM 不要）．`--canonical-mode {rule|llm}` で canonical-norm 同定の方式を選ぶ（`rule` 既定は決定論キーとバイト等価）．

## LLM 環境変数

| 変数 | 既定 | 用途 |
|------|------|------|
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama エンドポイント（第一候補） |
| `OLLAMA_MODEL` | `llama3.2:latest` | Ollama モデル |
| `OPENAI_API_KEY` | （未設定） | OpenAI フォールバックを有効化 |
| `OPENAI_MODEL` | `gpt-4o-mini` | OpenAI モデル |

プロバイダ順は **Ollama 第一 → OpenAI フォールバック**．live バックエンドに到達できない場合はオフライン mock スモーク（`cargo run --release --example mock_smoke -- results`）を使う．

## `run`

1 つの社会を規範ライフサイクルで実行する．

```bash
cargo run --release -- run \
    --population 10 --entrepreneurs 3 \
    --network ws --ws-k 4 --ws-beta 0.1 \
    --rounds 48 --synth-threshold 200 --seed 42
```

| フラグ | 既定 | 意味 |
|--------|------|------|
| `--population` | 10 | エージェント数 N |
| `--entrepreneurs` | 3 | 規範起業家の人数 |
| `--network` | ws | トポロジ: `ws` / `er` / `ba` |
| `--ws-k` | 4 | WS 初期次数 k（偶数） |
| `--ws-beta` | 0.1 | WS 再配線確率 β |
| `--er-p` | 0.3 | ER 辺確率 p |
| `--ba-m` | 2 | BA 新規ノードあたり結合数 m |
| `--rounds` | 48 | ラウンド数 T |
| `--synth-threshold` | 200 | 長期統合の有用性閾値 θ |
| `--convergence-window` | 3 | 適格集合が K ラウンド不変なら停止 |
| `--emergence-threshold` | 0.9 | 創発時刻判定の採用率しきい |
| `--canonical-mode` | deterministic | 規範同定: `deterministic`（別名 `rule`）/ `llm` |
| `--mock` | false | 決定論的 scripted クライアントで駆動（live LLM 不要） |
| `--seed` | ランダム | socsim コアシード |
| `--temperature` | 0.0 | LLM 生成温度 |
| `--llm-seed` | 0 | LLM バックエンドシード |
| `--cache-path` | `.llm_cache/cache.json` | プロンプト→応答キャッシュ |
| `--output-dir` | results | runvault の results ルート |

出力は run ディレクトリ 1 本（`subcommand=run`）:

- `config.json` — 実験条件（`parameters` の下）．`llm_cache_path` は置き場でしかないので `config_hash` から外してある．
- `metrics.csv` — long 形式 `run_uid,step,step_unit,scope,name,value`．`step` がラウンド `t`（`step_unit=round`），`scope=run`．ラウンドごとの指標は `adoption_rate` / `compliance_rate` / `n_conflicts` / `n_distinct_norms` / `n_qualified_holders` と型別の `adoption_injunctive` / `adoption_descriptive` / `n_distinct_injunctive` / `n_distinct_descriptive`．`step` を持たない run 全体の指標は `n_units` / `converged` / `final_step` / `time_to_emergence`（**創発しなかった run では行そのものが無い**）と `llm_calls` / `llm_cache_hits` / `llm_cache_hit_rate`．
- `events.jsonl` — 旧 `norms.csv`．終端の規範DB を `x.ren2024.norm` イベントとして 1 規範 1 行で書く（`unit_id` がエージェント，`t` が最終ラウンド，欄は `norm_type` / `content` / `utility` / `s_act` / `s_val` / `qualified`）．
- `run.json` — LLM の provider / モデル / 温度は `llm` ブロックが持つ（旧 `run_metadata.json` は書かれない）．

`--canonical-mode llm` では LLM 判定器（キャッシュ付き，`temperature=0`）が «二つの規範表現が同じ規範か» を判定し，語彙が重ならないパラフレーズも束ねる．rule-key 一致時は判定をショートカットする．`rule` 経路は変更なし．

## `sweep`

人口 × WS-β を走査，各セル複数の独立試行．

```bash
cargo run --release -- sweep \
    --population-values 6,10,20 \
    --ws-beta-min 0.0 --ws-beta-max 0.5 --ws-beta-step 0.1 \
    --rounds 48 --runs 3 --seed 42
```

| フラグ | 既定 | 意味 |
|--------|------|------|
| `--population-values` | 6,10,20 | カンマ区切りの人口リスト |
| `--ws-beta-min` / `--ws-beta-max` / `--ws-beta-step` | 0.0 / 0.5 / 0.1 | WS-β 格子 |
| `--network` | ws | トポロジ（単一固定） |
| `--entrepreneurs` | 3 | 起業家（固定） |
| `--ws-k` | 4 | WS 次数 k |
| `--runs` | 3 | セルあたり独立試行数 |
| `--rounds` | 48 | ラウンド数 T |
| `--seed` | 42 | シード基点（各試行は独立シードを派生） |
| `--cache-path` | `.llm_cache/cache.json` | sweep 全体で共有するキャッシュ |
| `--output-dir` | results | runvault の results ルート |

出力は **親 1 本 + 試行ごとの子 run**．親（`subcommand=sweep`）は格子の定義そのものを `parameters` に持ち，個別条件の指標は書かない（1 本のシミュレーションではないので `master_seed` も名乗らない）．子（`subcommand=sweep-point`）は (人口 × β × 試行) の 1 本ずつで，手で回した `run` と同じ形の `parameters`・`metrics.csv`・`events.jsonl` を持つ（同じ条件なら `config_hash` が一致する）．同一セルの繰り返しは `replicate_index` が分ける．

旧 `sweep_summary.csv` はディスクに無い．同じ値は子 run にあるので Python 側の `crsec_tools.sweep_summary` が組み直す（`crsec-tools visualize-sweep` が使う）．1 つだけ表し方が変わった: 未創発を旧 CSV は `time_to_emergence = -1` という番兵で表していたが，runvault では行そのものを書かない（NaN になる）．

## `reproduce`

論文の見出し的知見を一括再現する: 規範の **創発**（採用率が高水準へ上昇），**統合**（相異規範数がピークから縮約），社会的衝突の **rise-then-fall**，**Fact 7**（命令的規範が記述的規範より先に創発）．標準設定を複数シードで回してトラジェクトリを平均し，観測 vs 論文のアンカーを採点する．

```bash
# オフライン（live LLM 不要）: 決定論的 scripted mock
cargo run --release -- reproduce --mock

# live（Ollama→OpenAI），3 シード
cargo run --release -- reproduce --population 12 --runs 3 --rounds 48 --seed 42
```

| フラグ | 既定 | 意味 |
|--------|------|------|
| `--population` | 12 | エージェント数 N |
| `--entrepreneurs` | 3 | 規範起業家の人数 |
| `--network` | ws | トポロジ: `ws` / `er` / `ba` |
| `--ws-k` | 4 | WS 初期次数 k |
| `--ws-beta` | 0.1 | WS 再配線確率 β |
| `--rounds` | 48 | ラウンド数 T |
| `--runs` | 3 | 独立試行数（平均をとる） |
| `--emergence-threshold` | 0.9 | 創発時刻判定の採用率しきい |
| `--canonical-mode` | deterministic | 規範同定: `deterministic`（別名 `rule`）/ `llm` |
| `--mock` | false | 決定論的 scripted クライアントで駆動（live LLM 不要） |
| `--quick` | false | N とラウンドを縮小（動作確認用; 論文値検証には使わない） |
| `--temperature` / `--llm-seed` / `--cache-path` | 0.0 / 0 / `.llm_cache/cache.json` | LLM 設定（live 時のみ） |
| `--seed` | 42 | シード基点（各試行は独立シードを派生） |
| `--output-dir` | results | runvault の results ルート |

出力は **親 1 本 + 試行ごとの子 run**．親（`subcommand=reproduce`）の `metrics.csv` が試行平均（`scope=sweep` の `mean_final_adoption` / `mean_tte_injunctive` / `consolidation_gap` …）を持ち，`reference.csv` が **論文の報告値**（Fact 4 の «Day 2 末に injunctive 規範を 100% が受容・遵守»）を出典付きで持つ．子（`subcommand=reproduce-run`）は試行 1 本ずつで，`replicate_index` が試行番号．

許容帯（`target_lo` / `target_hi`）は論文の主張ではなく本実装が置いたものなので記録しない — 帯つきの PASS/OFF はコンソールにだけ出る．Python の `crsec-tools reproduce` は親の集約と `reference.csv` を読み，帯ではなく論文値との差を出す（同じ閾値を Python と Rust の 2 箇所に置くと食い違う余地ができる）．図は代表 run（`replicate_index = 0` の子）から描く．
