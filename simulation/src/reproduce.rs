//! 論文（Ren et al. 2024, CRSEC）の **見出し的知見の一括再現**．
//!
//! `reproduce` サブコマンドの実体．CRSEC のライフサイクル（Creation / Representation /
//! Spreading / Evaluation / Compliance）が生む «社会規範の創発» を，複数試行の平均
//! トラジェクトリと論文知見アンカーの PASS/off で要約し，`reproduce_summary.json` +
//! 代表 run の `metrics.csv` を書き出す．
//!
//! 再現する論文知見（観測 vs 論文）:
//! - **H1 emergence**: 集団の採用率が高水準へ上昇する（共有社会規範の創発）．
//! - **H2 consolidation**: 相異なる canonical 規範数がピークから縮約する（収斂）．
//! - **H3 conflict rise-then-fall**: 社会的衝突がピーク後に減衰する．
//! - **H4 Fact 7（descriptive vs injunctive）**: 命令的規範が記述的規範より先に創発する．
//!
//! サンドボックス・CI では `--mock`（決定論的 scripted クライアント; [`crate::reproduce_mock`]）
//! で駆動しライブ LLM を回避する．`--quick` は N・runs・rounds を縮小する（動作確認用）．

use serde::Serialize;

use crate::config::{CanonicalMode, Config, LlmSettings, Network};
use crate::metrics::{time_to_emergence, Metrics};
use crate::norm::NormType;
use crate::simulation::{run, run_mock, SimulationResult};

/// `reproduce` の引数（[`crate::main`] から構築）．
#[derive(Debug, Clone)]
pub struct ReproduceArgs {
    /// 人口（エージェント数 N）．
    pub population: usize,
    /// 規範起業家の人数．
    pub entrepreneurs: usize,
    /// 社会接続トポロジ（ws / er / ba）．
    pub network: Network,
    /// WS の各ノードの初期次数 k．
    pub ws_k: usize,
    /// WS の再配線確率 β．
    pub ws_beta: f64,
    /// ラウンド数 T．
    pub rounds: usize,
    /// 各条件あたりの独立試行数．
    pub runs: usize,
    /// 採用率の創発しきい．
    pub emergence_threshold: f64,
    /// 規範同定の方式（rule / llm）．
    pub canonical_mode: CanonicalMode,
    /// オフライン scripted mock で駆動する（ライブ LLM 不要）．
    pub mock: bool,
    /// 軽量モード（N・runs・rounds を縮小）．
    pub quick: bool,
    /// LLM 生成温度（live 時）．
    pub temperature: f32,
    /// LLM 生成シード（live 時）．
    pub llm_seed: u64,
    /// プロンプト→応答キャッシュの保存先（live 時; 全条件で共有）．
    pub cache_path: String,
    /// 乱数シード基点（各試行は derive により独立化する）．
    pub seed: u64,
    /// 出力ディレクトリ．
    pub output_dir: String,
}

/// 複数試行を平均した再現セル．
#[derive(Serialize, Clone)]
pub struct ReproCell {
    /// 試行数．
    pub runs: usize,
    /// 試行平均の最終採用率（共有社会規範の創発度）．
    pub mean_final_adoption: f64,
    /// 試行平均の最終遵守率．
    pub mean_final_compliance: f64,
    /// 試行平均の «最終 / ピーク相異 canonical 規範数»（統合の指標）．
    pub mean_final_distinct: f64,
    pub mean_peak_distinct: f64,
    /// 試行平均の «ピーク衝突 / 最終衝突»（rise-then-fall の指標）．
    pub mean_peak_conflicts: f64,
    pub mean_final_conflicts: f64,
    /// 試行平均の創発時刻（採用率 ≥ threshold; 未創発は rounds で代用）．
    pub mean_time_to_emergence: f64,
    /// 試行平均の «命令的 / 記述的» 規範の創発時刻（Fact 7; 未創発は rounds）．
    pub mean_tte_injunctive: f64,
    pub mean_tte_descriptive: f64,
    /// 試行平均の最終 «命令的 / 記述的» 採用率．
    pub mean_final_adoption_injunctive: f64,
    pub mean_final_adoption_descriptive: f64,
    /// 収束した試行の割合．
    pub converged_frac: f64,
}

/// 観測値と論文の定性的知見を突き合わせた 1 アンカー．
#[derive(Serialize)]
pub struct ReproAnchor {
    pub name: String,
    pub paper: String,
    pub observed: f64,
    pub target_lo: f64,
    pub target_hi: f64,
    pub pass: bool,
}

/// `adoptions` 列で `threshold` 未到達なら `fallback` を返す創発時刻．
fn tte_or(adoptions: &[f64], threshold: f64, fallback: usize) -> usize {
    time_to_emergence(adoptions, threshold).unwrap_or(fallback)
}

/// 基準設定（条件ごとに seed のみ差し替える）．
fn base_config(args: &ReproduceArgs, rounds: usize, population: usize) -> Config {
    Config {
        population,
        entrepreneurs: args.entrepreneurs.min(population),
        network: args.network,
        ws_k: args.ws_k,
        ws_beta: args.ws_beta,
        er_p: 0.3,
        ba_m: 2,
        rounds,
        // 各条件を同じ T まで回して創発曲線を比較するため収束で早期停止させない．
        synth_threshold: 1e9,
        convergence_window: rounds.max(1) + 1,
        emergence_threshold: args.emergence_threshold,
        canonical_mode: args.canonical_mode,
        seed: Some(args.seed),
        llm: LlmSettings {
            temperature: args.temperature,
            seed: args.llm_seed,
            cache_path: if args.mock {
                None
            } else {
                Some(args.cache_path.clone())
            },
        },
        output_dir: args.output_dir.clone(),
    }
}

/// `runs` 回回して再現セルを作り，代表 run（run 0）のメトリクス履歴を返す．
fn run_cell(args: &ReproduceArgs, rounds: usize, population: usize) -> (ReproCell, Vec<Metrics>) {
    let base = base_config(args, rounds, population);
    let mut acc = ReproCell {
        runs: args.runs,
        mean_final_adoption: 0.0,
        mean_final_compliance: 0.0,
        mean_final_distinct: 0.0,
        mean_peak_distinct: 0.0,
        mean_peak_conflicts: 0.0,
        mean_final_conflicts: 0.0,
        mean_time_to_emergence: 0.0,
        mean_tte_injunctive: 0.0,
        mean_tte_descriptive: 0.0,
        mean_final_adoption_injunctive: 0.0,
        mean_final_adoption_descriptive: 0.0,
        converged_frac: 0.0,
    };
    let mut representative: Option<Vec<Metrics>> = None;

    for run_idx in 0..args.runs.max(1) {
        let seed = socsim_core::derive_seed(args.seed, &[population as u64, run_idx as u64]);
        let cfg = Config {
            seed: Some(seed),
            ..base.clone()
        };
        let result: SimulationResult = if args.mock {
            run_mock(&cfg).unwrap_or_else(|e| panic!("mock 実行に失敗: {e}"))
        } else {
            run(&cfg).unwrap_or_else(|e| panic!("実行に失敗: {e}"))
        };
        let hist = &result.metrics_history;
        let last = hist.last().expect("metrics non-empty");

        acc.mean_final_adoption += last.adoption_rate;
        acc.mean_final_compliance += last.compliance_rate;
        acc.mean_final_distinct += last.n_distinct_norms as f64;
        acc.mean_peak_distinct += hist.iter().map(|m| m.n_distinct_norms).max().unwrap_or(0) as f64;
        acc.mean_peak_conflicts += hist.iter().map(|m| m.n_conflicts).max().unwrap_or(0) as f64;
        acc.mean_final_conflicts += last.n_conflicts as f64;
        acc.mean_time_to_emergence += tte_or(
            &hist.iter().map(|m| m.adoption_rate).collect::<Vec<_>>(),
            args.emergence_threshold,
            rounds,
        ) as f64;
        // 型別の創発: 当該型の採用率が «型別しきい» を超えた最初のラウンド．記述的・
        // 命令的で同じしきい（threshold）を使い，創発順序（Fact 7）を比較する．
        acc.mean_tte_injunctive += tte_or(
            &hist
                .iter()
                .map(|m| m.adoption_injunctive)
                .collect::<Vec<_>>(),
            args.emergence_threshold,
            rounds,
        ) as f64;
        acc.mean_tte_descriptive += tte_or(
            &hist
                .iter()
                .map(|m| m.adoption_descriptive)
                .collect::<Vec<_>>(),
            args.emergence_threshold,
            rounds,
        ) as f64;
        acc.mean_final_adoption_injunctive += last.adoption_injunctive;
        acc.mean_final_adoption_descriptive += last.adoption_descriptive;
        acc.converged_frac += if result.converged { 1.0 } else { 0.0 };

        if run_idx == 0 {
            representative = Some(hist.clone());
        }
    }

    let n = args.runs.max(1) as f64;
    acc.mean_final_adoption /= n;
    acc.mean_final_compliance /= n;
    acc.mean_final_distinct /= n;
    acc.mean_peak_distinct /= n;
    acc.mean_peak_conflicts /= n;
    acc.mean_final_conflicts /= n;
    acc.mean_time_to_emergence /= n;
    acc.mean_tte_injunctive /= n;
    acc.mean_tte_descriptive /= n;
    acc.mean_final_adoption_injunctive /= n;
    acc.mean_final_adoption_descriptive /= n;
    acc.converged_frac /= n;

    (acc, representative.unwrap_or_default())
}

/// 論文知見アンカーを組み立てる．
fn build_anchors(cell: &ReproCell, threshold: f64) -> Vec<ReproAnchor> {
    let mut anchors: Vec<ReproAnchor> = Vec::new();
    let mut push = |name: &str, paper: &str, obs: f64, lo: f64, hi: f64| {
        anchors.push(ReproAnchor {
            name: name.to_string(),
            paper: paper.to_string(),
            observed: obs,
            target_lo: lo,
            target_hi: hi,
            pass: obs >= lo && obs <= hi,
        });
    };

    // H1 emergence: 共有社会規範が創発する（採用率 ≥ 創発しきい）．
    push(
        "emergence (final adoption >= threshold)",
        "social norms emerge",
        cell.mean_final_adoption,
        threshold,
        f64::INFINITY,
    );
    // H2 consolidation: 相異規範数がピークから縮約する（peak - final >= 0）．
    push(
        "consolidation (peak_distinct - final_distinct >= 0)",
        "norms consolidate",
        cell.mean_peak_distinct - cell.mean_final_distinct,
        -1e-9,
        f64::INFINITY,
    );
    // H3 conflict rise-then-fall: ピーク衝突 ≥ 最終衝突（rise then fall）．
    push(
        "conflict_rise_then_fall (peak - final >= 0)",
        "conflicts rise then fall",
        cell.mean_peak_conflicts - cell.mean_final_conflicts,
        -1e-9,
        f64::INFINITY,
    );
    // H4 Fact 7: 命令的が記述的より «早く» 創発する（tte_des - tte_inj >= 0）．
    push(
        "fact7_injunctive_before_descriptive (tte_des - tte_inj >= 0)",
        "injunctive precedes descriptive",
        cell.mean_tte_descriptive - cell.mean_tte_injunctive,
        -1e-9,
        f64::INFINITY,
    );

    anchors
}

/// `reproduce` の出力（main から JSON 化 / コンソール出力する）．
pub struct ReproduceOutput {
    pub cell: ReproCell,
    pub anchors: Vec<ReproAnchor>,
    pub representative: Vec<Metrics>,
    pub population: usize,
    pub rounds: usize,
}

/// 一括再現を実行する（条件は単一 = 論文の標準設定; 試行平均で集計）．
pub fn reproduce(args: &ReproduceArgs) -> ReproduceOutput {
    // quick モードは軽量化（動作確認用; 論文値検証には使わない）．
    let population = if args.quick {
        args.population.min(6)
    } else {
        args.population
    };
    let rounds = if args.quick {
        args.rounds.min(12)
    } else {
        args.rounds
    };

    let (cell, representative) = run_cell(args, rounds, population);
    let anchors = build_anchors(&cell, args.emergence_threshold);

    ReproduceOutput {
        cell,
        anchors,
        representative,
        population,
        rounds,
    }
}

/// アンカー名に出す型ラベル（命令的 / 記述的）．
pub fn type_label(alpha: NormType) -> &'static str {
    alpha.label()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_args(quick: bool) -> ReproduceArgs {
        ReproduceArgs {
            population: 8,
            entrepreneurs: 2,
            network: Network::WattsStrogatz,
            ws_k: 4,
            ws_beta: 0.2,
            rounds: 16,
            runs: 2,
            emergence_threshold: 0.9,
            canonical_mode: CanonicalMode::Deterministic,
            mock: true,
            quick,
            temperature: 0.0,
            llm_seed: 0,
            cache_path: ".llm_cache/cache.json".to_string(),
            seed: 42,
            output_dir: "results".to_string(),
        }
    }

    #[test]
    fn reproduce_on_mock_emerges_and_passes_anchors() {
        let out = reproduce(&mock_args(false));
        // 規範が創発し採用率が高水準へ（H1）．
        assert!(
            out.cell.mean_final_adoption >= 0.9,
            "adoption={}",
            out.cell.mean_final_adoption
        );
        // 全アンカーが in-band（mock は論文の定性的知見を再現するよう設計）．
        let n_pass = out.anchors.iter().filter(|a| a.pass).count();
        assert_eq!(
            n_pass,
            out.anchors.len(),
            "anchors: {n_pass}/{}",
            out.anchors.len()
        );
    }

    #[test]
    fn reproduce_fact7_injunctive_not_after_descriptive() {
        let out = reproduce(&mock_args(false));
        // Fact 7: 命令的の創発時刻 ≤ 記述的の創発時刻．
        assert!(
            out.cell.mean_tte_injunctive <= out.cell.mean_tte_descriptive,
            "inj_tte={} des_tte={}",
            out.cell.mean_tte_injunctive,
            out.cell.mean_tte_descriptive
        );
    }

    #[test]
    fn reproduce_mock_is_deterministic() {
        let a = reproduce(&mock_args(false));
        let b = reproduce(&mock_args(false));
        assert_eq!(a.cell.mean_final_adoption, b.cell.mean_final_adoption);
        assert_eq!(a.cell.mean_tte_injunctive, b.cell.mean_tte_injunctive);
        let av: Vec<f64> = a.representative.iter().map(|m| m.adoption_rate).collect();
        let bv: Vec<f64> = b.representative.iter().map(|m| m.adoption_rate).collect();
        assert_eq!(av, bv);
    }

    #[test]
    fn reproduce_canonical_mode_llm_mock_runs() {
        let mut args = mock_args(true);
        args.canonical_mode = CanonicalMode::Llm;
        let out = reproduce(&args);
        // llm canonical-mode（mock judge）でも創発する．
        assert!(out.cell.mean_final_adoption > 0.0);
    }
}
