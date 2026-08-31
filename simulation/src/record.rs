//! runvault への記録の共通部分．
//!
//! 論文メタデータ (research) は `run` でも `sweep` / `reproduce` の子でも同一なので，
//! ここ 1 箇所で組み立てる．ラウンドごとの指標，run 全体を 1 つの値で表す指標，
//! LLM ブロックと LLM 呼び出しの統計，終端の規範スナップショットもここに集める．
//!
//! # 規範をどこに置くか
//!
//! 旧 `norms.csv` は `agent_id,content,type,utility,s_act,s_val,qualified` で，時間軸を
//! 持たず 1 エージェントにつき複数行あった．runvault の `metrics.csv` は
//! `run_uid,step,step_unit,scope,name,value` で «どの主体か» を入れる列を持たないので，
//! エージェントごとの規範を並べると全行が同じ主キー `(name, step, step_unit, scope)`
//! を名乗って衝突する．しかも `content` は規範の本文，`type` は inj / des というラベル
//! であって数ではない．
//!
//! そこで実験固有の種別 [`NORM_EVENT`] に置く．コア語彙の `observation` ではないのは，
//! `observation` が «その単位をいつ見たか» という到達時間の観測 1 点を意味する行だから
//! である．ここに書くのは run が終わった時点の規範DB — 終端の状態スナップショットで，
//! knoll2013 の `x.knoll2013.agent` と同じ形になる．

use runvault::{Llm, Replication, Run, Target, Work};
use serde::Serialize;
use socsim_llm::MetadataCollector;

use crate::metrics::Metrics;
use crate::simulation::SimulationResult;

/// runvault 上の実験名．`runvault path --experiment` に渡す値でもある．
/// バイナリ名 (`crsec`) と揃える．
pub const EXPERIMENT: &str = "crsec";
/// リポジトリの安定 id．git remote の名前とは独立に固定する．
pub const REPO_ID: &str = "ren2024";
/// 分野．網生成・プロフィール割当・活性化順・相手サンプリングがいずれも乱数駆動で
/// `master_seed` が要るので `simulation`．規範の創出・伝播・評価・遵守は LLM が担うが，
/// 測っているのはモデルの安全性ではなく社会規範の創発なので `llm-safety` ではない．
/// LLM 側の同一性は `run.json` の `llm` ブロックが持つ．
pub const DOMAIN: &str = "simulation";

/// 時間軸の単位．
///
/// このモデルの 1 刻みは論文 Section 2 のラウンド — 全エージェントが創出・遵守・
/// 伝播・評価を一巡するまでの 1 周である．runvault の語彙では `round`．
const T_UNIT: &str = "round";

/// ラウンドごとの指標と run 全体の指標の粒度．いずれも集団全体の集約なので `run`．
const SCOPE: &str = "run";

/// `reproduce` の親が持つ «試行をまたいだ集約» の粒度．
pub const SWEEP_SCOPE: &str = "sweep";

/// 終端の規範スナップショットを書く実験固有のイベント種別．
pub const NORM_EVENT: &str = "x.ren2024.norm";

/// この再現実験が対象としている論文．
///
/// `run` も `sweep` / `reproduce` の子も同じ主張を対象とする — 掃引は人口 × WS-β を
/// 変えて規範創発の条件を見るためのもので，別の対象を持たない．
pub fn replication() -> Replication {
    let mut work = Work::doi("10.24963/ijcai.2024/874")
        .title(
            "Emergence of Social Norms in Generative Agent Societies: Principles and Architecture",
        )
        .year(2024)
        .source_version("ijcai-2024");
    // 同じ論文を arXiv と vault の paper-id からも引けるようにする (work_id は DOI 側)．
    work.arxiv_id = Some("2403.08251".to_string());
    work.paper_id = Some("P00001796".to_string());
    Replication::new(work)
        .target(Target::figure("fig2", "Figure 2"))
        .target(Target::claim(
            "shared-norm-emergence",
            "A norm seeded by a few norm entrepreneurs spreads through conversation and \
             observation until the whole society has adopted and complies with it",
        ))
        .target(Target::claim(
            "injunctive-precedes-descriptive",
            "Injunctive norms emerge earlier than descriptive ones (Fact 7)",
        ))
        .obsidian_note("研究/98_論文レポート/80-再現実験/実装完了/ren2024/設計書.md")
}

// --------------------------------------------------------------------------- //
// LLM ブロック
// --------------------------------------------------------------------------- //

/// 実際に応答したバックエンドを `llm` ブロックに落とす．
///
/// `model` / `endpoint` はクライアントが名乗った値をそのまま使う．`provider` は runvault
/// の語彙ではなく自由記述なので，endpoint から «どのゲートウェイが答えたか» を決める．
/// 推測しているのは分類だけで，値そのものは記録から採る．
///
/// `model_snapshot` に入るのは `llama3.2:latest` のような動くエイリアスであることが多い．
/// socsim-llm はスナップショット id を持たないので，持っていない値を作らずに名乗られた
/// 名前を書く．
pub fn llm_block(model: &str, endpoint: &str, temperature: f32) -> Llm {
    let provider = if endpoint.starts_with("mock://") {
        "mock"
    } else if endpoint.contains("openai") {
        "openai"
    } else {
        "ollama"
    };
    Llm {
        provider: provider.to_string(),
        model_snapshot: model.to_string(),
        temperature: Some(temperature as f64),
        // 創出・伝播・評価・遵守のプロンプトは相手とラウンドごとに組み立てられ，固定の
        // system prompt を持たない．無いものを hash しない．
        system_prompt_hash: None,
    }
}

// --------------------------------------------------------------------------- //
// シミュレーション 1 本ぶんの記録
// --------------------------------------------------------------------------- //

/// シミュレーション 1 本ぶんを run へ書く (`run` サブコマンドと掃引・再現の子で共通)．
pub fn log_simulation(run: &mut Run, result: &SimulationResult) {
    for m in &result.metrics_history {
        log_step(run, m);
    }
    log_run_scope(run, result);
    log_llm(run, &result.metadata);
}

/// `Metrics` の数値フィールドを 1 ラウンドぶんまとめて書く．
///
/// `Metrics::t` はここに来ない — 時間軸そのものなので `step` が持つ．名前は旧
/// `metrics.csv` の列名をそのまま使う．型別の 4 つ (`adoption_injunctive` /
/// `adoption_descriptive` / `n_distinct_injunctive` / `n_distinct_descriptive`) は
/// «inj / des というカテゴリに番号を振った» ものではなく，«その型の規範が占める割合»
/// と «その型の相異規範の個数» というラウンドごとの数である．
fn log_step(run: &mut Run, m: &Metrics) {
    run.log_metrics_at(
        m.t as u64,
        T_UNIT,
        SCOPE,
        &[
            ("adoption_rate", m.adoption_rate),
            ("compliance_rate", m.compliance_rate),
            ("n_conflicts", m.n_conflicts as f64),
            ("n_distinct_norms", m.n_distinct_norms as f64),
            ("n_qualified_holders", m.n_qualified_holders as f64),
            ("adoption_injunctive", m.adoption_injunctive),
            ("adoption_descriptive", m.adoption_descriptive),
            ("n_distinct_injunctive", m.n_distinct_injunctive as f64),
            ("n_distinct_descriptive", m.n_distinct_descriptive as f64),
        ],
    )
    .unwrap_or_else(|e| panic!("round {} の指標の記録に失敗: {e}", m.t));
}

/// run 全体を 1 つの値で表す指標．
///
/// 観測主体の数は予約指標名の `n_units` (= エージェント数)．`time_to_emergence` は
/// 採用率がしきいに達しなかった run では «0» ではなく «定義できない» ので，行そのものを
/// 書かない (旧 `sweep_summary.csv` の `-1` という番兵をやめた)．実行時間は
/// `status.json` の `duration_sec` が正本なので指標にしない．
fn log_run_scope(run: &mut Run, result: &SimulationResult) {
    let mut values: Vec<(&str, f64)> = vec![
        ("n_units", result.final_norm_db.len() as f64),
        ("converged", if result.converged { 1.0 } else { 0.0 }),
        ("final_step", result.final_step as f64),
    ];
    if let Some(t) = result.time_to_emergence {
        values.push(("time_to_emergence", t as f64));
    }
    run.log_metrics(SCOPE, &values)
        .expect("run スコープの指標の記録に失敗");
}

/// LLM 呼び出しの統計．
///
/// モデル・endpoint・温度は `run.json` の `llm` ブロックが持つので，ここには数だけを
/// 書く．cache-hit 率は呼び出しが 1 本も無いときに «0» ではなく «定義できない» ので，
/// そのときは行そのものを書かない．
fn log_llm(run: &mut Run, metadata: &MetadataCollector) {
    let total = metadata.total();
    if total == 0 {
        return;
    }
    run.log_metrics(
        SCOPE,
        &[
            ("llm_calls", total as f64),
            ("llm_cache_hits", metadata.cache_hits() as f64),
            ("llm_cache_hit_rate", metadata.cache_hit_rate()),
        ],
    )
    .expect("LLM 統計の記録に失敗");
}

// --------------------------------------------------------------------------- //
// 終端の規範スナップショット
// --------------------------------------------------------------------------- //

/// `events.jsonl` に書く規範 1 件．
///
/// `unit_id` はエージェント，`t` は run が終わったラウンド．1 エージェントが複数の規範を
/// 持つので `unit_id` は行をまたいで繰り返す (コア語彙の `observation` / `terminal` とは
/// 違い，実験固有の種別に主体ごと 1 行という制約は無い)．
///
/// 欄名は掃引パラメータ (population / ws_beta / network / …) と重ならないようにしてある
/// — `sweep_events_table` は同名のパラメータ列でイベント列を上書きするので，衝突すると
/// 黙って消える．旧 CSV の `type` は runvault の予約語 (`schema`) と紛らわしいので
/// `norm_type` に改名した．
#[derive(Serialize)]
struct NormEvent<'a> {
    unit_id: String,
    t: u64,
    t_unit: &'static str,
    /// α: 記述的 / 命令的．ラベルであって数ではないのでここにしか置けない．
    norm_type: &'a str,
    /// c: 規範の自然言語記述．
    content: String,
    /// u: 有用性 ∈ [1, 100]．
    utility: u8,
    /// s_act: 活性．
    s_act: bool,
    /// s_val: 有効．
    s_val: bool,
    /// s_act ∧ s_val (適格)．旧 CSV の列をそのまま残す — 集計はこの欄で絞る．
    qualified: bool,
}

/// 最終的な規範DB をエージェント別に `events.jsonl` へ書く (旧 `norms.csv`)．
pub fn log_norms(run: &mut Run, result: &SimulationResult) {
    let t = result.final_step as u64;
    for (&socsim_core::AgentId(id), norms) in &result.final_norm_db {
        for n in norms {
            let event = NormEvent {
                unit_id: id.to_string(),
                t,
                t_unit: T_UNIT,
                norm_type: n.alpha.label(),
                // 旧 CSV と同じく改行を空白に潰す (1 規範 1 行という粒度を保つ)．
                content: n.content.replace(['\n', '\r'], " "),
                utility: n.utility,
                s_act: n.s_act,
                s_val: n.s_val,
                qualified: n.qualified(),
            };
            run.log_event(NORM_EVENT, &event)
                .unwrap_or_else(|e| panic!("agent {id} の規範の記録に失敗: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runvault::meta::TargetKind;

    #[test]
    fn the_work_id_agrees_with_the_doi() {
        let research: runvault::meta::Research = replication().into();
        let work = research.work.expect("再現実験なので work がある");
        assert_eq!(work.work_id, "doi:10.24963/ijcai.2024/874");
        assert_eq!(work.doi.as_deref(), Some("10.24963/ijcai.2024/874"));
        assert_eq!(work.arxiv_id.as_deref(), Some("2403.08251"));
        assert_eq!(work.paper_id.as_deref(), Some("P00001796"));
    }

    #[test]
    fn the_targets_cover_the_figure_and_both_claims() {
        let research: runvault::meta::Research = replication().into();
        let kinds: Vec<TargetKind> = research.targets.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![TargetKind::Figure, TargetKind::Claim, TargetKind::Claim]
        );
    }

    #[test]
    fn the_provider_follows_the_endpoint() {
        assert_eq!(llm_block("m", "mock://scripted", 0.0).provider, "mock");
        assert_eq!(
            llm_block("m", "https://api.openai.com/v1/chat/completions", 0.0).provider,
            "openai"
        );
        assert_eq!(
            llm_block("m", "http://localhost:11434/api/chat", 0.0).provider,
            "ollama"
        );
    }

    /// イベントの欄名が掃引パラメータと重ならないことを固定する．
    ///
    /// `sweep_events_table` は同名のパラメータ列でイベント列を上書きするので，衝突すると
    /// 黙って消える．掃引パラメータ側の名前が変わったらこのテストが落ちる．
    #[test]
    fn the_norm_event_fields_do_not_collide_with_sweep_parameters() {
        let event = NormEvent {
            unit_id: "0".to_string(),
            t: 3,
            t_unit: T_UNIT,
            norm_type: "inj",
            content: "no smoking indoors".to_string(),
            utility: 80,
            s_act: true,
            s_val: true,
            qualified: true,
        };
        let value = serde_json::to_value(&event).expect("イベントの serialize に失敗");
        let fields: Vec<&String> = value.as_object().expect("object").keys().collect();
        for parameter in [
            "population",
            "entrepreneurs",
            "network",
            "ws_k",
            "ws_beta",
            "er_p",
            "ba_m",
            "rounds",
            "synth_threshold",
            "convergence_window",
            "emergence_threshold",
            "canonical_mode",
            "seed",
            "mock",
            "runs",
            "llm_temperature",
            "llm_seed",
            "llm_cache_path",
        ] {
            assert!(
                !fields.iter().any(|f| f.as_str() == parameter),
                "イベントの欄 {parameter} が掃引パラメータと衝突している"
            );
        }
    }
}
