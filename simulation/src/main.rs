//! Ren et al. (2024) "Emergence of Social Norms in Generative Agent Societies
//! (CRSEC)" — 再現実験の CLI エントリポイント．
//!
//! `run`   : 単一設定で規範ライフサイクルを実行し，創発曲線・衝突時系列を出力する．
//! `sweep` : 人口 × WS-β（× ネットワーク）を走査し，創発時刻・最終採用率を集計する．
//!
//! Phase 3 の `reproduce`（Fig. 2 一括再現 + descriptive vs injunctive 深掘り）は未実装
//! （拡張点）．

use std::fs;
use std::path::Path;

use clap::{Parser, Subcommand};
use socsim_results::{refresh_latest_symlink, timestamp, write_csv, write_json};

use crsec_simulation::config::{
    parse_canonical_mode, parse_network, CanonicalMode, Config, LlmSettings, Network,
};
use crsec_simulation::simulation::{
    ensure_output_dir, run, save_metrics, save_norms, save_run_metadata,
};

// ---------------------------------------------------------------------------
// CLI 定義
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "crsec",
    about = "Ren et al. (2024) Emergence of Social Norms in Generative Agent Societies (CRSEC) — 再現実験"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 単一設定で規範ライフサイクルを実行する．
    Run(RunArgs),
    /// 人口 × WS-β を走査し，創発時刻・最終採用率を集計する．
    Sweep(SweepArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// 人口（エージェント数 N）．
    #[arg(long, default_value_t = 10)]
    population: usize,

    /// 規範起業家の人数．
    #[arg(long, default_value_t = 3)]
    entrepreneurs: usize,

    /// 社会接続トポロジ（ws / er / ba）．
    #[arg(long, default_value = "ws")]
    network: String,

    /// WS の各ノードの初期次数 k（偶数）．
    #[arg(long, default_value_t = 4)]
    ws_k: usize,

    /// WS の再配線確率 β．
    #[arg(long, default_value_t = 0.1)]
    ws_beta: f64,

    /// ER の辺生成確率 p．
    #[arg(long, default_value_t = 0.3)]
    er_p: f64,

    /// BA の新規ノードあたりの結合数 m．
    #[arg(long, default_value_t = 2)]
    ba_m: usize,

    /// ラウンド数 T．
    #[arg(long, default_value_t = 48)]
    rounds: usize,

    /// 長期統合の有用性閾値 θ．
    #[arg(long, default_value_t = 200.0)]
    synth_threshold: f64,

    /// 収束判定の安定ウィンドウ K．
    #[arg(long, default_value_t = 3)]
    convergence_window: usize,

    /// 採用率の創発しきい（time_to_emergence の判定）．
    #[arg(long, default_value_t = 0.9)]
    emergence_threshold: f64,

    /// 規範同定の方式（deterministic / llm）．
    #[arg(long, default_value = "deterministic")]
    canonical_mode: String,

    /// 乱数シード（省略時はランダム; socsim コア層のみ支配）．
    #[arg(long)]
    seed: Option<u64>,

    /// LLM 生成温度（既定 0.0; 再現性のため）．
    #[arg(long, default_value_t = 0.0)]
    temperature: f32,

    /// LLM 生成シード（バックエンドへ渡す）．
    #[arg(long, default_value_t = 0)]
    llm_seed: u64,

    /// プロンプト→応答キャッシュの保存先（既定 .llm_cache/cache.json）．
    #[arg(long, default_value = ".llm_cache/cache.json")]
    cache_path: String,

    /// 結果出力ディレクトリ．
    #[arg(long, default_value = "results")]
    output_dir: String,
}

#[derive(Parser, Debug)]
struct SweepArgs {
    /// カンマ区切りの人口リスト．
    #[arg(long, default_value = "6,10,20")]
    population_values: String,

    /// WS-β の最小値．
    #[arg(long, default_value_t = 0.0)]
    ws_beta_min: f64,

    /// WS-β の最大値．
    #[arg(long, default_value_t = 0.5)]
    ws_beta_max: f64,

    /// WS-β の刻み．
    #[arg(long, default_value_t = 0.1)]
    ws_beta_step: f64,

    /// 規範起業家の人数（sweep では固定）．
    #[arg(long, default_value_t = 3)]
    entrepreneurs: usize,

    /// 社会接続トポロジ（ws / er / ba; sweep では単一固定）．
    #[arg(long, default_value = "ws")]
    network: String,

    /// WS の各ノードの初期次数 k．
    #[arg(long, default_value_t = 4)]
    ws_k: usize,

    /// 各条件あたりの独立試行数．
    #[arg(long, default_value_t = 3)]
    runs: usize,

    /// ラウンド数 T．
    #[arg(long, default_value_t = 48)]
    rounds: usize,

    /// 長期統合の有用性閾値 θ．
    #[arg(long, default_value_t = 200.0)]
    synth_threshold: f64,

    /// 収束判定の安定ウィンドウ K．
    #[arg(long, default_value_t = 3)]
    convergence_window: usize,

    /// 採用率の創発しきい．
    #[arg(long, default_value_t = 0.9)]
    emergence_threshold: f64,

    /// 乱数シード基点（各試行は derive により独立化する）．
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// LLM 生成温度．
    #[arg(long, default_value_t = 0.0)]
    temperature: f32,

    /// LLM 生成シード．
    #[arg(long, default_value_t = 0)]
    llm_seed: u64,

    /// プロンプト→応答キャッシュの保存先（sweep 全体で共有しヒット率を高める）．
    #[arg(long, default_value = ".llm_cache/cache.json")]
    cache_path: String,

    /// 結果出力ベースディレクトリ．
    #[arg(long, default_value = "results")]
    output_dir: String,
}

// ---------------------------------------------------------------------------
// 補助
// ---------------------------------------------------------------------------

/// `sweep_summary.csv` の 1 行．
#[derive(serde::Serialize)]
struct SweepRow {
    population: usize,
    ws_beta: f64,
    network: String,
    run: usize,
    seed: u64,
    converged: bool,
    final_step: usize,
    time_to_emergence: i64,
    final_adoption_rate: f64,
    final_compliance_rate: f64,
    final_n_distinct_norms: usize,
    peak_conflicts: usize,
    cache_hit_rate: f64,
}

/// `sweep_config.json` の構造体．
#[derive(serde::Serialize)]
struct SweepConfigJson {
    command: &'static str,
    population_values: Vec<usize>,
    ws_beta_values: Vec<f64>,
    network: String,
    entrepreneurs: usize,
    ws_k: usize,
    runs: usize,
    rounds: usize,
    synth_threshold: f64,
    convergence_window: usize,
    emergence_threshold: f64,
    seed: u64,
    llm_temperature: f32,
    llm_seed: u64,
}

/// カンマ区切り文字列を trim 済みの非空リストへ．
fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// `ws_beta_min..=max` を step 刻みで列挙する（浮動小数の累積誤差を避け整数倍で生成）．
fn beta_range(min: f64, max: f64, step: f64) -> Vec<f64> {
    if step <= 0.0 || max < min {
        return vec![min];
    }
    let n = ((max - min) / step).round() as i64;
    (0..=n)
        .map(|i| (min + step * i as f64 * 1e6).round() / 1e6)
        .collect()
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn cmd_run(args: RunArgs) {
    let network = parse_network(&args.network).unwrap_or_else(|e| panic!("{}", e));
    let canonical_mode =
        parse_canonical_mode(&args.canonical_mode).unwrap_or_else(|e| panic!("{}", e));

    let timestamp = timestamp();
    let output_dir = format!("{}/{}", args.output_dir, timestamp);

    let cfg = Config {
        population: args.population,
        entrepreneurs: args.entrepreneurs,
        network,
        ws_k: args.ws_k,
        ws_beta: args.ws_beta,
        er_p: args.er_p,
        ba_m: args.ba_m,
        rounds: args.rounds,
        synth_threshold: args.synth_threshold,
        convergence_window: args.convergence_window,
        emergence_threshold: args.emergence_threshold,
        canonical_mode,
        seed: args.seed,
        llm: LlmSettings {
            temperature: args.temperature,
            seed: args.llm_seed,
            cache_path: Some(args.cache_path.clone()),
        },
        output_dir: output_dir.clone(),
    };

    if let Some(parent) = Path::new(&args.cache_path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    ensure_output_dir(&cfg.output_dir);

    println!("=== Ren et al. (2024) CRSEC 社会規範の創発 再現実験 ===");
    println!(
        "N: {} | 起業家: {} | network: {} | ws_k: {} | ws_beta: {}",
        cfg.population,
        cfg.entrepreneurs,
        cfg.network.label(),
        cfg.ws_k,
        cfg.ws_beta,
    );
    println!(
        "rounds: {} | θ: {} | K: {} | canonical: {} | seed: {:?}",
        cfg.rounds,
        cfg.synth_threshold,
        cfg.convergence_window,
        cfg.canonical_mode.label(),
        cfg.seed,
    );
    println!(
        "LLM: temp={} llm_seed={} cache={}",
        cfg.llm.temperature, cfg.llm.seed, args.cache_path
    );
    println!("出力先: {}", cfg.output_dir);
    println!("-------------------------------------------------");

    let result = run(&cfg).unwrap_or_else(|e| panic!("実行に失敗: {}", e));

    save_metrics(&result.metrics_history, &cfg.output_dir);
    save_norms(&result, &cfg.output_dir);
    save_run_metadata(&result, &cfg, &cfg.output_dir);

    // config.json (pretty-print JSON; socsim_results::write_json に委譲)．
    {
        let path = format!("{}/config.json", cfg.output_dir);
        write_json(&cfg.to_run_config_json(), &path).expect("config.json の書き込みに失敗");
    }

    // latest シンボリックリンクを再作成する (best-effort; 従来同様エラーは無視)．
    let _ = refresh_latest_symlink(&args.output_dir, &timestamp);

    let last = result.metrics_history.last().unwrap();
    let peak_conflicts = result
        .metrics_history
        .iter()
        .map(|m| m.n_conflicts)
        .max()
        .unwrap_or(0);
    println!(
        "収束: {} | ステップ: {} | 創発: {}",
        if result.converged { "Yes" } else { "No" },
        result.final_step,
        result
            .time_to_emergence
            .map(|t| format!("t={t}"))
            .unwrap_or_else(|| "未創発".to_string()),
    );
    println!(
        "最終 採用率: {:.3} | 遵守率: {:.3} | 相異規範数: {} | 衝突ピーク: {}",
        last.adoption_rate, last.compliance_rate, last.n_distinct_norms, peak_conflicts,
    );
    println!(
        "LLM 呼び出し: {} 回 | cache-hit: {} ({:.1}%) | model: {}",
        result.metadata.total(),
        result.metadata.cache_hits(),
        result.metadata.cache_hit_rate() * 100.0,
        result.llm_model,
    );
    println!("メトリクス → {}/metrics.csv", cfg.output_dir);
    println!("規範       → {}/norms.csv", cfg.output_dir);
    println!("LLM メタ   → {}/run_metadata.json", cfg.output_dir);
    println!("設定       → {}/config.json", cfg.output_dir);
}

// ---------------------------------------------------------------------------
// sweep
// ---------------------------------------------------------------------------

fn cmd_sweep(args: SweepArgs) {
    let network: Network = parse_network(&args.network).unwrap_or_else(|e| panic!("{}", e));
    let populations: Vec<usize> = split_csv(&args.population_values)
        .iter()
        .map(|s| {
            s.parse::<usize>()
                .unwrap_or_else(|_| panic!("不正な人口: {s}"))
        })
        .collect();
    let betas = beta_range(args.ws_beta_min, args.ws_beta_max, args.ws_beta_step);

    let timestamp = timestamp();
    let sweep_dir = format!("{}/{}_sweep", args.output_dir, timestamp);
    fs::create_dir_all(&sweep_dir).expect("sweep ディレクトリの作成に失敗");
    if let Some(parent) = Path::new(&args.cache_path).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let n_total = populations.len() * betas.len() * args.runs;

    println!("=== Ren et al. (2024) CRSEC パラメータスイープ ===");
    println!(
        "人口: {:?} | WS-β: {:?} | network: {} | 試行: {} | 合計: {} 実行",
        populations,
        betas,
        network.label(),
        args.runs,
        n_total,
    );
    println!("出力先: {}", sweep_dir);
    println!("-----------------------------------------------------------");

    let mut summary_rows: Vec<SweepRow> = Vec::with_capacity(n_total);
    let mut done = 0usize;

    for &population in &populations {
        for &beta in &betas {
            for run_idx in 0..args.runs {
                // 各条件に独立なシードを派生（explicit identity）．
                let seed = socsim_core::derive_seed(
                    args.seed,
                    &[population as u64, (beta * 1e6) as u64, run_idx as u64],
                );

                let cfg = Config {
                    population,
                    entrepreneurs: args.entrepreneurs,
                    network,
                    ws_k: args.ws_k,
                    ws_beta: beta,
                    er_p: 0.3,
                    ba_m: 2,
                    rounds: args.rounds,
                    synth_threshold: args.synth_threshold,
                    convergence_window: args.convergence_window,
                    emergence_threshold: args.emergence_threshold,
                    canonical_mode: CanonicalMode::Deterministic,
                    seed: Some(seed),
                    llm: LlmSettings {
                        temperature: args.temperature,
                        seed: args.llm_seed,
                        cache_path: Some(args.cache_path.clone()),
                    },
                    output_dir: sweep_dir.clone(),
                };

                let result = run(&cfg).unwrap_or_else(|e| panic!("実行に失敗: {}", e));
                let last = result.metrics_history.last().unwrap();
                let peak = result
                    .metrics_history
                    .iter()
                    .map(|m| m.n_conflicts)
                    .max()
                    .unwrap_or(0);

                summary_rows.push(SweepRow {
                    population,
                    ws_beta: beta,
                    network: network.label().to_string(),
                    run: run_idx,
                    seed,
                    converged: result.converged,
                    final_step: result.final_step,
                    time_to_emergence: result.time_to_emergence.map(|t| t as i64).unwrap_or(-1),
                    final_adoption_rate: last.adoption_rate,
                    final_compliance_rate: last.compliance_rate,
                    final_n_distinct_norms: last.n_distinct_norms,
                    peak_conflicts: peak,
                    cache_hit_rate: result.metadata.cache_hit_rate(),
                });

                done += 1;
            }
            println!(
                "[{}/{}] population={} ws_beta={:.3} 完了 ({} 試行)",
                done, n_total, population, beta, args.runs,
            );
        }
    }

    // sweep_summary.csv (各行を serialize; socsim_results::write_csv に委譲)．
    {
        let path = format!("{}/sweep_summary.csv", sweep_dir);
        write_csv(&summary_rows, &path).expect("sweep_summary.csv の書き込みに失敗");
    }

    // sweep_config.json
    {
        let config_json = SweepConfigJson {
            command: "sweep",
            population_values: populations.clone(),
            ws_beta_values: betas.clone(),
            network: network.label().to_string(),
            entrepreneurs: args.entrepreneurs,
            ws_k: args.ws_k,
            runs: args.runs,
            rounds: args.rounds,
            synth_threshold: args.synth_threshold,
            convergence_window: args.convergence_window,
            emergence_threshold: args.emergence_threshold,
            seed: args.seed,
            llm_temperature: args.temperature,
            llm_seed: args.llm_seed,
        };
        let path = format!("{}/sweep_config.json", sweep_dir);
        write_json(&config_json, &path).expect("sweep_config.json の書き込みに失敗");
    }

    let _ = refresh_latest_symlink(&args.output_dir, &format!("{}_sweep", timestamp));

    println!("===========================================================");
    println!("スイープ完了: {} 実行", n_total);
    println!("-----------------------------------------------------------");
    println!("人口別の平均 最終採用率:");
    for &population in &populations {
        let rows: Vec<&SweepRow> = summary_rows
            .iter()
            .filter(|r| r.population == population)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let avg = rows.iter().map(|r| r.final_adoption_rate).sum::<f64>() / rows.len() as f64;
        println!("  N={:<4} → 採用率̄ = {:.3}", population, avg);
    }
    println!("-----------------------------------------------------------");
    println!("サマリ → {}/sweep_summary.csv", sweep_dir);
    println!("設定   → {}/sweep_config.json", sweep_dir);
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run(args) => cmd_run(args),
        Commands::Sweep(args) => cmd_sweep(args),
    }
}
