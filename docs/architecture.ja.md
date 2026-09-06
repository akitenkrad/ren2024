# アーキテクチャ

[English](architecture.md) | [日本語](architecture.ja.md)

## リポジトリ構成

```
ren2024/
├── simulation/                  Rust crate `crsec-simulation`（binary `crsec`）
│   ├── src/
│   │   ├── main.rs              CLI (clap): run / sweep / reproduce
│   │   ├── lib.rs               公開モジュール
│   │   ├── config.rs            Config, Network{ws|er|ba}, CanonicalMode, LlmSettings
│   │   ├── norm.rs              PersonalNorm 5つ組 ⟨c,u,α,s_act,s_val⟩, NormType
│   │   ├── world.rs            CrsecWorld (WorldState), AgentProfile, InteractionEvent, canonical_key, Canonicalizer
│   │   ├── llm.rs              CrsecClient = CachingClient<Box<dyn LlmClient>>（Ollama→OpenAI + キャッシュ）
│   │   ├── prompts.rs          CRSEC LLM 操作のプロンプト（KEY: value 契約）
│   │   ├── parse.rs            構造化 LLM 応答の寛容なパーサ
│   │   ├── mechanisms.rs       6 ライフサイクルメカニズム
│   │   ├── simulation.rs       init_world + run_with_client ドライバ + canonicalizer 配線
│   │   ├── record.rs           runvault への記録（実験名 / 論文メタ / llm ブロック / 指標 / 規範イベント）
│   │   ├── metrics.rs          採用率 / 遵守率 / 衝突 / 相異規範数（型別含む）/ 創発時刻
│   │   ├── reproduce.rs        reproduce の集約: 1 試行の抽出 + 試行平均セル + 観測 vs 論文アンカー + 論文値
│   │   └── reproduce_mock.rs   オフライン run / reproduce 用の決定論的 scripted クライアント
│   ├── examples/mock_smoke.rs  オフライン（live LLM 不要）スモーク
│   └── tests/integration_test.rs   mock 駆動の統合テスト
├── tools/                       Python パッケージ `crsec-tools`（module `crsec_tools`）
│   └── src/crsec_tools/{cli,visualize,visualize_sweep,sweep_summary,show_experiment_settings,reproduce_paper}.py
├── docs/                        bilingual ドキュメント（本ディレクトリ）
└── results/                     runvault の results ルート（gitignore 対象）
    └── crsec/<subcommand>_<timestamp>_<config_hash>_<uid>/   run ディレクトリ
```

## 二層決定論

socsim コアは決定論的で LLM を含まない．LLM 出力は bit 再現できない．本実装は二層を明示する:

- **決定論的 socsim コア（下層）**．単一 root シードから 2 ストリームを派生: `derive_seed(root,&[0])` が世界初期化（ネットワーク生成・プロフィール + 起業家割当）を，`derive_seed(root,&[1])` がエンジン（`RandomActivationScheduler` の活性化順・`ctx.rng` の会話/観察相手サンプリング）を駆動する．指標と canonical-norm 同定は状態の純関数なので決定論的．
- **非決定的 LLM レイヤ（上層）**．`CrsecClient = CachingClient<Box<dyn LlmClient>>` でメカニズム内に閉じ込める．バックエンドは `FallbackClient<OllamaClient, OpenAiClient>` を `Box<dyn LlmClient>` に型消去したもの（socsim-llm が `impl LlmClient for Box<T>`（issue #26）を提供するため自前 newtype は不要）．`temperature=0` + 固定 seed + `hash(prompt+model)` → 応答キャッシュで擬似決定論化する．

モデル・provider・温度は runvault の `run.json` の `llm` ブロックが，呼び出し数と cache-hit は run スコープの指標 `llm_calls` / `llm_cache_hits` / `llm_cache_hit_rate` が持つ（率は呼び出しが 1 本も無いときに «0» ではなく «定義できない» ので，そのときは行そのものを書かない）．収束・最終ステップ・創発時刻も run スコープの指標．`determinism_note` は数でも条件でもないので `simulation::DETERMINISM_NOTE` とこのドキュメントに残す．

`llm` ブロックのためにクライアントは `Run::start` の **前** に組む — モデル名と endpoint を知っているのはクライアントを組んだ側だけだからである（`simulation::run` / `run_mock` を消したのはこのため）．

## CRSEC ライフサイクル → メカニズム対応

CRSEC の規範ライフサイクルを socsim の 6-phase ループへ，1 規則 = 1 メカニズムで翻訳する．宣言順 = 同一フェーズ内の発火順．

| Mechanism | Phase | 役割 | LLM 操作 / エージェント / ラウンド |
|-----------|-------|------|-----------------------------------|
| `ResetInteractions` | PreStep | 当ラウンドの会話・観察ログ + カウンタをクリア | 0 |
| `CreationMechanism` | Environment | 起業家が DB 空のとき初期規範を創出（`CreateNorm`） | ≤ 1（DB 空のときのみ; 定常 0） |
| `ComplianceMechanism` | Decision | 適格集合に沿う行動を生成し遵守を記録 | 適格規範を持てば 1 |
| `SpreadingMechanism` | Interaction | 送信者ごとに近傍 1 名と会話 + 1 名を観察; **1 回の構造化呼び出し** で衝突検出 + 会話判断 + 規範識別; 識別規範は受信者DBへ未適格で格納 | 最大 2（相手ごと 1） |
| `EvaluationMechanism` | PostStep | **1 回の構造化呼び出し** で 4 サニティ検査（整合性 / 重複 / 型 / 衝突）→ 適格へ昇格; 合計有用性 > θ で長期統合（抽象規範 + 元規範非活性化） | 未評価候補ごと 1 |
| `ConvergenceMechanism` | PostStep | 適格 canonical 集合が `K` ラウンド不変なら停止（`request_stop`） | 0 |

**LLM 呼び出しの統合（明示した実用的判断）**．論文は伝播・評価で複数の LLM 操作を挙げるが，呼び出しを抑えるため (a) `DetectConflict` + `DecideToTalk` + `IdentifyNormativeInformation` を 1 伝播プロンプトに，(b) 4 評価検査を 1 評価プロンプトに統合する．プロンプトは行ベースの `KEY: value` 出力を求め，`parse.rs` が寛容にパースする．

## 更新セマンティクス

1 tick = 全エージェントが創出 / 遵守 / 伝播 / 評価を一巡する 1 ラウンド．伝播で識別された規範はバッファに溜め Interaction フェーズ末に一括適用するため，同一ラウンド内の他者の識別に mid-round の DB 変化が波及しない（スナップショットイディオム）．活性化順は毎ラウンドランダム化（順序効果を平均化）；固定順にしたい場合は sequential scheduler に切替可能．

## canonical-norm 同定

`world::Canonicalizer` が «二つの規範表現が同じ規範か» を判定する．方式は `--canonical-mode` で選ぶ:

- **`rule`**（既定）— `world::canonical_key` への純委譲．記述を決定論的なキーワード集合へ縮約する（小文字化 → 英数字以外で分割 → ストップワード除去 → 重複除去 → ソート → 連結）．採用率・相異規範数・伝播 dedup・収束集合はすべてこのキーで束ね，**追加の LLM 呼び出しなし**；この経路は `Canonicalizer` 導入前のコードとバイト等価．
- **`llm`** — キャッシュ付き LLM 判定器（`prompts::same_norm_prompt`，`parse::same_norm` でパース）が既出代表のリストに対し «同じ規範か» を答え，語彙が重ならないパラフレーズも束ねる．rule-key 一致時は判定をショートカット；判定は共有キャッシュ付きクライアントを通るため擬似決定論的．canonicalizer は伝播 dedup と収束メカニズムで共有（`Rc`）する．

## 指標

| 指標 | 定義 |
|------|------|
| `adoption_rate` | 最も共有された canonical 規範を適格として持つエージェント割合 |
| `compliance_rate` | 当ラウンドに遵守（COMPLY=yes）したエージェント割合 |
| `n_conflicts` | 当ラウンドに検出された衝突数（DetectConflict=T） |
| `n_distinct_norms` | 相異 canonical 規範数（適格のみ） |
| `adoption_injunctive` / `adoption_descriptive` | 型別採用率（命令的 vs 記述的） |
| `n_distinct_injunctive` / `n_distinct_descriptive` | 型別の相異 canonical 規範数 |
| `time_to_emergence` | `adoption_rate ≥ emergence_threshold`（既定 0.9）の最初のラウンド |

型別の列が descriptive vs injunctive 深掘りを支える: `reproduce` サブコマンドが型別の創発時刻（命令的が記述的より先）を比較し，Python `reproduce` ツールが 2 本のトラジェクトリを描く．

## 定性的な再現目標

ローカルモデル（`llama3.2:latest`）は論文の GPT-3.5/4 と異なるため，**数値**でなく **パターン** を狙う: 採用率は 1.0 へ上昇，相異規範数は統合，衝突数は初期急増後に減少，injunctive が descriptive より先に創発．`reproduce` サブコマンドはこれらを観測 vs 論文のアンカーとして採点し `reproduce_summary.json` に書く．

## 参考文献

- Ren, S., Cui, Z., Song, R., Wang, Z., & Hu, S. (2024). Emergence of Social Norms in Generative Agent Societies: Principles and Architecture. *IJCAI-24*, 7895–7903. arXiv:2403.08251.
- Park, J. S., et al. (2023). Generative Agents: Interactive Simulacra of Human Behavior. *UIST '23*. arXiv:2304.03442.
- Cialdini, R. B., Kallgren, C. A., & Reno, R. R. (1991). A Focus Theory of Normative Conduct.
- [socsim](https://github.com/akitenkrad/rs-social-simulation-tools) — `socsim-core` / `socsim-engine` / `socsim-net` / `socsim-llm`.
