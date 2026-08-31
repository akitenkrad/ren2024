//! 論文（Ren et al. 2024, CRSEC）の **見出し的知見の一括再現**．
//!
//! `reproduce` サブコマンドの計算部分．CRSEC のライフサイクル（Creation / Representation /
//! Spreading / Evaluation / Compliance）が生む «社会規範の創発» を，複数試行の平均
//! トラジェクトリと論文知見アンカーの PASS/off で要約する．
//!
//! 再現する論文知見（観測 vs 論文）:
//! - **H1 emergence**: 集団の採用率が高水準へ上昇する（共有社会規範の創発）．
//! - **H2 consolidation**: 相異なる canonical 規範数がピークから縮約する（収斂）．
//! - **H3 conflict rise-then-fall**: 社会的衝突がピーク後に減衰する．
//! - **H4 Fact 7（descriptive vs injunctive）**: 命令的規範が記述的規範より先に創発する．
//!
//! サンドボックス・CI では `--mock`（決定論的 scripted クライアント; [`crate::reproduce_mock`]）
//! で駆動しライブ LLM を回避する．`--quick` は N・rounds を縮小する（動作確認用）．
//!
//! # 試行はどこへ記録されるか
//!
//! 試行 1 本ずつが模型の **別々の実行** なので，runvault では sweep 親（`reproduce`）+
//! 試行ごとの子 run（`reproduce-run`，`replicate_index` = 試行番号）になる．試行を回す
//! ループは `main` にあり，このモジュールは «1 試行から何を取り出すか»（[`ReproTrial`]）と
//! «試行群をどう集約するか»（[`ReproCell`]）だけを持つ．クライアントを組むのは呼び出し側の
//! 仕事である（`run.json` の `llm` ブロックを埋めるため）．

use serde::Serialize;

use crate::config::{CanonicalMode, Config, LlmSettings, Network};
use crate::metrics::time_to_emergence;
use crate::simulation::SimulationResult;

/// `reproduce` の引数（`main` から構築）．
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
    /// 軽量モード（N・rounds を縮小）．
    pub quick: bool,
    /// LLM 生成温度（live 時）．
    pub temperature: f32,
    /// LLM 生成シード（live 時）．
    pub llm_seed: u64,
    /// プロンプト→応答キャッシュの保存先（live 時; 全試行で共有）．
    pub cache_path: String,
    /// 乱数シード基点（各試行は derive により独立化する）．
    pub seed: u64,
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
///
/// `target_lo` / `target_hi` は論文が印字した数ではなく **本実装が置いた許容帯** なので，
/// `reference.csv` にも指標にも入らない．PASS/OFF はコンソールにだけ残す．
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

/// `--quick` の縮小を適用した «実際に回す» 人口とラウンド数．
///
/// `quick` そのものは記録しない — 結果を決めるのは縮小後の値で，それは `parameters` の
/// `population` / `rounds` に入る．
pub fn effective(args: &ReproduceArgs) -> (usize, usize) {
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
    (population, rounds)
}

/// 試行 1 本の設定（試行ごとに seed だけが変わる）．
///
/// `synth_threshold` と `convergence_window` は `reproduce` が独自に固定する — 各試行を
/// 同じ T まで回して創発曲線を比較するため，長期統合も収束による早期停止もさせない．
/// 旧 `reproduce_summary.json` の `config` ブロックはこの 2 つを持っていなかったが，
/// 結果を決める値なので `parameters` に入れる．
pub fn trial_config(args: &ReproduceArgs, rounds: usize, population: usize, seed: u64) -> Config {
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
        seed: Some(seed),
        llm: LlmSettings {
            temperature: args.temperature,
            seed: args.llm_seed,
            cache_path: if args.mock {
                // mock は in-memory キャッシュ（永続保存しない）．
                None
            } else {
                Some(args.cache_path.clone())
            },
        },
    }
}

/// 試行 1 本のシードを base seed から決定的に派生させる．
pub fn trial_seed(base: u64, population: usize, run_idx: usize) -> u64 {
    socsim_core::derive_seed(base, &[population as u64, run_idx as u64])
}

/// 試行 1 本から集約に必要な値だけを取り出したもの．
///
/// 試行の全ステップは子 run の `metrics.csv` が持つ．ここにあるのは «集約の材料» で，
/// 親の `scope=sweep` 指標を組むためだけに使う．
pub struct ReproTrial {
    pub final_adoption: f64,
    pub final_compliance: f64,
    pub final_distinct: f64,
    pub peak_distinct: f64,
    pub peak_conflicts: f64,
    pub final_conflicts: f64,
    pub tte: f64,
    pub tte_injunctive: f64,
    pub tte_descriptive: f64,
    pub final_adoption_injunctive: f64,
    pub final_adoption_descriptive: f64,
    pub converged: bool,
}

impl ReproTrial {
    /// [`SimulationResult`] から取り出す．
    ///
    /// 型別の創発は «当該型の採用率がしきいを超えた最初のラウンド»．記述的・命令的で
    /// 同じしきい（`threshold`）を使い，創発順序（Fact 7）を比較する．未創発は `rounds`
    /// で代用する（平均を取るため）．
    pub fn from_result(result: &SimulationResult, threshold: f64, rounds: usize) -> Self {
        let hist = &result.metrics_history;
        let last = hist.last().expect("metrics non-empty");
        ReproTrial {
            final_adoption: last.adoption_rate,
            final_compliance: last.compliance_rate,
            final_distinct: last.n_distinct_norms as f64,
            peak_distinct: hist.iter().map(|m| m.n_distinct_norms).max().unwrap_or(0) as f64,
            peak_conflicts: hist.iter().map(|m| m.n_conflicts).max().unwrap_or(0) as f64,
            final_conflicts: last.n_conflicts as f64,
            tte: tte_or(
                &hist.iter().map(|m| m.adoption_rate).collect::<Vec<_>>(),
                threshold,
                rounds,
            ) as f64,
            tte_injunctive: tte_or(
                &hist
                    .iter()
                    .map(|m| m.adoption_injunctive)
                    .collect::<Vec<_>>(),
                threshold,
                rounds,
            ) as f64,
            tte_descriptive: tte_or(
                &hist
                    .iter()
                    .map(|m| m.adoption_descriptive)
                    .collect::<Vec<_>>(),
                threshold,
                rounds,
            ) as f64,
            final_adoption_injunctive: last.adoption_injunctive,
            final_adoption_descriptive: last.adoption_descriptive,
            converged: result.converged,
        }
    }
}

impl ReproCell {
    /// 試行群を平均する．
    ///
    /// `runs` は CLI で指定された試行数（`trials` の本数は `runs.max(1)`）．合計してから
    /// 割る順序は移行前と同じにしてある — 浮動小数の和は順序で変わる．
    pub fn from_trials(trials: &[ReproTrial], runs: usize) -> Self {
        let mut acc = ReproCell {
            runs,
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
        for trial in trials {
            acc.mean_final_adoption += trial.final_adoption;
            acc.mean_final_compliance += trial.final_compliance;
            acc.mean_final_distinct += trial.final_distinct;
            acc.mean_peak_distinct += trial.peak_distinct;
            acc.mean_peak_conflicts += trial.peak_conflicts;
            acc.mean_final_conflicts += trial.final_conflicts;
            acc.mean_time_to_emergence += trial.tte;
            acc.mean_tte_injunctive += trial.tte_injunctive;
            acc.mean_tte_descriptive += trial.tte_descriptive;
            acc.mean_final_adoption_injunctive += trial.final_adoption_injunctive;
            acc.mean_final_adoption_descriptive += trial.final_adoption_descriptive;
            acc.converged_frac += if trial.converged { 1.0 } else { 0.0 };
        }

        let n = runs.max(1) as f64;
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
        acc
    }

    /// 親 run の `scope=sweep` 指標として書く «観測値»．
    ///
    /// アンカーの観測値のうち差の 3 つ（`consolidation_gap` / `conflict_gap` /
    /// `fact7_gap`）もここに入れる — 論文が報告した数ではないので `reference.csv` には
    /// 入らないが，主張が «その差が非負か» なので測っている量そのものである．H1 の観測値は
    /// `mean_final_adoption` そのものなので重複させない．
    pub fn observed(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("mean_final_adoption", self.mean_final_adoption),
            ("mean_final_compliance", self.mean_final_compliance),
            ("mean_final_distinct", self.mean_final_distinct),
            ("mean_peak_distinct", self.mean_peak_distinct),
            ("mean_peak_conflicts", self.mean_peak_conflicts),
            ("mean_final_conflicts", self.mean_final_conflicts),
            ("mean_time_to_emergence", self.mean_time_to_emergence),
            ("mean_tte_injunctive", self.mean_tte_injunctive),
            ("mean_tte_descriptive", self.mean_tte_descriptive),
            (
                "mean_final_adoption_injunctive",
                self.mean_final_adoption_injunctive,
            ),
            (
                "mean_final_adoption_descriptive",
                self.mean_final_adoption_descriptive,
            ),
            ("converged_frac", self.converged_frac),
            (
                "consolidation_gap",
                self.mean_peak_distinct - self.mean_final_distinct,
            ),
            (
                "conflict_gap",
                self.mean_peak_conflicts - self.mean_final_conflicts,
            ),
            (
                "fact7_gap",
                self.mean_tte_descriptive - self.mean_tte_injunctive,
            ),
        ]
    }
}

/// 論文知見アンカーを組み立てる．
pub fn build_anchors(cell: &ReproCell, threshold: f64) -> Vec<ReproAnchor> {
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

/// 論文が報告した値 1 つ（`reference.csv` の 1 行になる）．
///
/// **論文が印字した数だけ**を入れる．本実装が置いた許容帯（[`build_anchors`] の
/// `target_lo` / `target_hi`）は誰の数でもないので入らない．
pub struct PaperValue {
    /// 対応する親の指標名．
    pub name: &'static str,
    /// 論文の値．
    pub value: f64,
    /// どの Target に対する数か．
    pub target_id: &'static str,
    /// 出典．
    pub source: &'static str,
}

/// 論文 Section 3（Fact 4）の報告値．
///
/// > "at the end of Day 2 in the Smallville environment, 100% of agents have adopted and
/// > adhered to the injunctive norms 'no smoking indoors' and 'be quiet in public'."
///
/// 採用（adopted）と遵守（adhered）がどちらも 100% なので，命令的規範の最終採用率と
/// 最終遵守率の 2 つに対応する．Fact 7 の «injunctive は Day 1，descriptive は Day 2 末»
/// は Smallville の «日» で数えた値で，本実装の «ラウンド» とは単位が違うため入れない
/// （順序そのものは `Target::claim` が持つ）．
pub const PAPER_VALUES: [PaperValue; 2] = [
    PaperValue {
        name: "mean_final_adoption_injunctive",
        value: 1.0,
        target_id: "shared-norm-emergence",
        source: "Ren et al. (2024) IJCAI-24, Section 3 (Fact 4): 100% of agents have adopted \
                 the injunctive norms at the end of Day 2",
    },
    PaperValue {
        name: "mean_final_compliance",
        value: 1.0,
        target_id: "shared-norm-emergence",
        source: "Ren et al. (2024) IJCAI-24, Section 3 (Fact 4): 100% of agents have adhered \
                 to the injunctive norms at the end of Day 2",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(final_adoption: f64, peak_distinct: f64, tte_inj: f64, tte_des: f64) -> ReproTrial {
        ReproTrial {
            final_adoption,
            final_compliance: 1.0,
            final_distinct: 1.0,
            peak_distinct,
            peak_conflicts: 5.0,
            final_conflicts: 0.0,
            tte: 3.0,
            tte_injunctive: tte_inj,
            tte_descriptive: tte_des,
            final_adoption_injunctive: final_adoption,
            final_adoption_descriptive: 0.0,
            converged: true,
        }
    }

    fn args(quick: bool) -> ReproduceArgs {
        ReproduceArgs {
            population: 12,
            entrepreneurs: 3,
            network: Network::WattsStrogatz,
            ws_k: 4,
            ws_beta: 0.1,
            rounds: 48,
            runs: 3,
            emergence_threshold: 0.9,
            canonical_mode: CanonicalMode::Deterministic,
            mock: true,
            quick,
            temperature: 0.0,
            llm_seed: 0,
            cache_path: ".llm_cache/cache.json".to_string(),
            seed: 42,
        }
    }

    #[test]
    fn the_cell_averages_over_the_trials() {
        let trials = vec![trial(1.0, 4.0, 1.0, 3.0), trial(0.5, 2.0, 2.0, 4.0)];
        let cell = ReproCell::from_trials(&trials, 2);
        assert_eq!(cell.runs, 2);
        assert!((cell.mean_final_adoption - 0.75).abs() < 1e-12);
        assert!((cell.mean_peak_distinct - 3.0).abs() < 1e-12);
        assert!((cell.mean_tte_injunctive - 1.5).abs() < 1e-12);
        assert!((cell.mean_tte_descriptive - 3.5).abs() < 1e-12);
        assert!((cell.converged_frac - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_anchors_read_the_cell() {
        let cell = ReproCell::from_trials(&[trial(1.0, 4.0, 1.0, 3.0)], 1);
        let anchors = build_anchors(&cell, 0.9);
        assert_eq!(anchors.len(), 4);
        assert!(anchors.iter().all(|a| a.pass), "全アンカーが in-band");
    }

    /// 親の観測値がアンカーの観測値をすべて含むことを固定する．
    #[test]
    fn the_observed_metrics_cover_every_anchor() {
        let cell = ReproCell::from_trials(&[trial(1.0, 4.0, 1.0, 3.0)], 1);
        let observed = cell.observed();
        for anchor in build_anchors(&cell, 0.9) {
            assert!(
                observed.iter().any(|(_, v)| *v == anchor.observed),
                "アンカー {} の観測値が指標にない",
                anchor.name
            );
        }
    }

    /// 論文値の指標名が親の観測値に実在する．
    #[test]
    fn every_paper_value_points_at_a_metric() {
        let cell = ReproCell::from_trials(&[trial(1.0, 4.0, 1.0, 3.0)], 1);
        let observed = cell.observed();
        for pv in &PAPER_VALUES {
            assert!(
                observed.iter().any(|(name, _)| *name == pv.name),
                "論文値 {} に対応する指標がない",
                pv.name
            );
        }
    }

    /// 派生シードは `(base, population, run_idx)` で決まる．
    #[test]
    fn each_coordinate_changes_the_trial_seed() {
        let base = trial_seed(42, 12, 0);
        assert_eq!(base, trial_seed(42, 12, 0));
        assert_ne!(base, trial_seed(43, 12, 0), "base が効いていない");
        assert_ne!(base, trial_seed(42, 10, 0), "population が効いていない");
        assert_ne!(base, trial_seed(42, 12, 1), "run_idx が効いていない");
    }

    /// `--quick` は人口とラウンド数だけを縮小する．
    #[test]
    fn quick_shrinks_population_and_rounds() {
        assert_eq!(effective(&args(false)), (12, 48));
        assert_eq!(effective(&args(true)), (6, 12));
    }

    /// `reproduce` は長期統合も収束による早期停止もさせない．
    #[test]
    fn the_trial_config_disables_synthesis_and_early_stop() {
        let cfg = trial_config(&args(false), 16, 8, 7);
        assert_eq!(cfg.synth_threshold, 1e9);
        assert_eq!(cfg.convergence_window, 17);
        // mock は永続キャッシュを持たない．
        assert!(cfg.llm.cache_path.is_none());
    }
}
