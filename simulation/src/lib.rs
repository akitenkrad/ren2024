//! Ren et al. (2024) "Emergence of Social Norms in Generative Agent Societies
//! (CRSEC)" の再現実装ライブラリ．
//!
//! socsim フレームワーク上に，LLM 駆動の生成エージェント社会で社会規範が創発する
//! 過程（CRSEC: Creation & Representation / Spreading / Evaluation / Compliance）を
//! `WorldState` + 6-phase `Mechanism` へ翻訳して構築する．設定（`config`）・規範型
//! （`norm`）・世界状態（`world`）・LLM クライアント層（`llm`）・プロンプト生成
//! （`prompts`）・応答パース（`parse`）・ライフサイクルメカニズム（`mechanisms`）・
//! 実行ドライバ（`simulation`）・集計メトリクス（`metrics`）をモジュールとして公開し，
//! バイナリ（`crsec`）と統合テストの双方から利用する．
//!
//! # 二層決定論
//!
//! socsim コア層（ネットワーク・活性化順・相手サンプリング・スケジューリング・
//! メトリクス・canonical-norm-identity）は seed から bit 単位で決定論的である．LLM
//! レイヤ（規範の創出・伝播・評価・遵守の生成）は socsim の bit 再現性の **外側** に
//! あり，`socsim-llm` のキャッシュ + `temperature=0` + `seed` 固定で擬似決定論化する．
//! 詳細は `crate::llm` を参照．

pub mod config;
pub mod llm;
pub mod mechanisms;
pub mod metrics;
pub mod norm;
pub mod parse;
pub mod prompts;
pub mod simulation;
pub mod world;
