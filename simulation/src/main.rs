//! Ren et al. (2024) "Emergence of Social Norms in Generative Agent Societies
//! (CRSEC)" — 再現実験の CLI エントリポイント．
//!
//! `run`       : 単一設定で規範ライフサイクルを実行し，創発曲線・衝突時系列を記録する．
//! `sweep`     : 人口 × WS-β（× ネットワーク）を走査する．親 1 本 + 試行ごとの子 run．
//! `reproduce` : 論文の見出し的知見（社会規範の創発・統合・衝突 rise-then-fall・
//!               Fact 7 の injunctive→descriptive 順序）を一括再現する．
//!               親 1 本 + 試行ごとの子 run で，親が試行平均と論文の報告値を持つ．
//!
//! `run --mock` / `reproduce --mock` はライブ LLM を呼ばず決定論的 scripted mock で
//! 駆動する（サンドボックス・CI 用）．`--canonical-mode llm` は規範同定を LLM 意味判定へ
//! 切り替える（既定 rule は決定論的 canonical_key へ純委譲）．
//!
//! 出力の置き場と同一性は runvault が持つ．タイムスタンプ付きディレクトリも `latest`
//! シンボリックリンクもこちらでは作らず，`Run::start` が決めた run ディレクトリへ書く．

use std::fs;
use std::path::Path;

use clap::{Parser, Subcommand};
use runvault::{Lineage, Run, RunOptions};
use serde::Serialize;

use crsec_simulation::config::{
    parse_canonical_mode, parse_network, CanonicalMode, Config, LlmSettings, Network, HASH_EXCLUDE,
    SEED_POINTERS,
};
use crsec_simulation::llm::{build_live_client, CrsecClient};
use crsec_simulation::record::{self, DOMAIN, EXPERIMENT, REPO_ID, SWEEP_SCOPE};
use crsec_simulation::reproduce::{
    self, ReproCell, ReproTrial, ReproduceArgs as ReproduceParams, PAPER_VALUES,
};
use crsec_simulation::reproduce_mock::build_reproduce_client;
use crsec_simulation::simulation::{run_with_client, SimulationResult};

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

    /// Ollama 接続先 URL（指定時は環境変数 OLLAMA_HOST を上書きする）．
    #[arg(long, global = true)]
    ollama_host: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 単一設定で規範ライフサイクルを実行する．
    Run(RunArgs),
    /// 人口 × WS-β を走査する（親 1 本 + 試行ごとの子 run）．
    Sweep(SweepArgs),
    /// 論文の見出し的知見を一括再現する（親 1 本 + 試行ごとの子 run）．
    Reproduce(ReproduceArgs),
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

    /// LLM を呼ばず決定論的 scripted mock で駆動する（オフライン検証用）．
    /// サンドボックス・CI では `--mock` を付ける（ライブ LLM 不要）．
    #[arg(long, default_value_t = false)]
    mock: bool,

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

    /// runvault の results ルート．
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

    /// runvault の results ルート．
    #[arg(long, default_value = "results")]
    output_dir: String,
}

#[derive(Parser, Debug)]
struct ReproduceArgs {
    /// 人口（エージェント数 N）．
    #[arg(long, default_value_t = 12)]
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

    /// ラウンド数 T．
    #[arg(long, default_value_t = 48)]
    rounds: usize,

    /// 各条件あたりの独立試行数．
    #[arg(long, default_value_t = 3)]
    runs: usize,

    /// 採用率の創発しきい．
    #[arg(long, default_value_t = 0.9)]
    emergence_threshold: f64,

    /// 規範同定の方式（deterministic / llm）．
    #[arg(long, default_value = "deterministic")]
    canonical_mode: String,

    /// LLM を呼ばず決定論的 scripted mock で駆動する（オフライン検証用）．
    /// サンドボックス・CI では `--mock` を付ける（ライブ LLM 不要）．
    #[arg(long, default_value_t = false)]
    mock: bool,

    /// 軽量モード（N と rounds を縮小; 動作確認用）．
    #[arg(long, default_value_t = false)]
    quick: bool,

    /// LLM 生成温度（live 時のみ）．
    #[arg(long, default_value_t = 0.0)]
    temperature: f32,

    /// LLM 生成シード（live 時のみ）．
    #[arg(long, default_value_t = 0)]
    llm_seed: u64,

    /// プロンプト→応答キャッシュの保存先（live 時のみ; 全条件で共有）．
    #[arg(long, default_value = ".llm_cache/cache.json")]
    cache_path: String,

    /// 乱数シード基点（各試行は derive により独立化する）．
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// runvault の results ルート．
    #[arg(long, default_value = "results")]
    output_dir: String,
}

// ---------------------------------------------------------------------------
// 親 run の parameters
// ---------------------------------------------------------------------------

/// `sweep` 親 run の `parameters`．掃引の格子そのものを持つ．
///
/// 旧 `sweep_config.json` に無かった `er_p` / `ba_m` / `canonical_mode` / `mock` /
/// `llm_cache_path` を足した．`--network er` / `ba` では `er_p` / `ba_m` が結果を決め，
/// `canonical_mode` は sweep が `deterministic` に固定している値である — 条件に入って
/// いないと `config_hash` が結果を決める値に盲目になる．
#[derive(Serialize)]
struct SweepConfigJson {
    population_values: Vec<usize>,
    ws_beta_values: Vec<f64>,
    network: String,
    entrepreneurs: usize,
    ws_k: usize,
    er_p: f64,
    ba_m: usize,
    runs: usize,
    rounds: usize,
    synth_threshold: f64,
    convergence_window: usize,
    emergence_threshold: f64,
    canonical_mode: String,
    seed: u64,
    mock: bool,
    llm_temperature: f32,
    llm_seed: u64,
    llm_cache_path: Option<String>,
}

/// `reproduce` 親 run の `parameters`．条件と試行数を持つ．
///
/// `population` / `rounds` は `--quick` の縮小を適用した **実際に回した** 値．
/// `synth_threshold` / `convergence_window` は `reproduce` が独自に固定する値で，旧
/// `reproduce_summary.json` の `config` ブロックには無かった（結果を決めるので入れる）．
#[derive(Serialize)]
struct ReproduceConfigJson {
    population: usize,
    entrepreneurs: usize,
    network: String,
    ws_k: usize,
    ws_beta: f64,
    er_p: f64,
    ba_m: usize,
    rounds: usize,
    runs: usize,
    synth_threshold: f64,
    convergence_window: usize,
    emergence_threshold: f64,
    canonical_mode: String,
    seed: u64,
    mock: bool,
    llm_temperature: f32,
    llm_seed: u64,
    llm_cache_path: Option<String>,
}

// ---------------------------------------------------------------------------
// 補助
// ---------------------------------------------------------------------------

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

/// LLM レイヤ設定を組む．
///
/// mock は in-memory キャッシュなので永続保存先を持たない（`cache_path = None`）．
fn llm_settings(temperature: f32, seed: u64, cache_path: &str, mock: bool) -> LlmSettings {
    LlmSettings {
        temperature,
        seed,
        cache_path: if mock {
            None
        } else {
            Some(cache_path.to_string())
        },
    }
}

/// キャッシュファイルの親ディレクトリを作る（live 時のみ）．
fn ensure_cache_dir(settings: &LlmSettings) {
    if let Some(path) = settings.cache_path.as_deref() {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
    }
}

/// LLM クライアントを組む．
///
/// `run.json` の `llm` ブロックに書くモデル名と endpoint は，実際に応答するバックエンドから
/// 採らないと意味を持たない．知っているのはクライアントを組んだ側だけなので，組み立ては
/// `Run::start` より前に行う（`simulation::run` / `run_mock` を消したのはこのため —
/// 中でクライアントを組む入口が残っていると `llm` ブロックを埋めないまま記録できてしまう）．
fn build_client(settings: &LlmSettings, mock: bool) -> CrsecClient {
    if mock {
        build_reproduce_client()
    } else {
        build_live_client(settings).unwrap_or_else(|e| panic!("LLM クライアント構築に失敗: {e}"))
    }
}

/// 子 run を 1 本起こしてシミュレーションを回す（`sweep` / `reproduce` 共通）．
///
/// 子の `parameters` は手で回した `run` と同じ形なので，同じ条件なら `config_hash` が
/// 一致する．`master_seed` は `derive_seed` が作った実際に使われるシードで，同一条件の
/// 繰り返しは `replicate_index` で分ける．
#[allow(clippy::too_many_arguments)]
fn run_child(
    subcommand: &str,
    cfg: &Config,
    mock: bool,
    seed: u64,
    replicate_index: usize,
    results_root: &str,
    sweep_id: &str,
    parent_run_uid: &str,
) -> SimulationResult {
    let client = build_client(&cfg.llm, mock);
    let llm = record::llm_block(
        client.inner().model(),
        client.inner().endpoint(),
        cfg.llm.temperature,
    );
    let parameters = cfg.to_parameters(mock);
    let mut child = Run::start(
        RunOptions::new(EXPERIMENT, subcommand)
            .repo_id(REPO_ID)
            .domain(DOMAIN)
            .results_root(results_root)
            .parameters(&parameters)
            .expect("runvault: 子 run の parameters の組み立てに失敗")
            .hash_exclude(HASH_EXCLUDE)
            .seed_pointers(SEED_POINTERS)
            .master_seed(seed)
            .replicate_index(replicate_index as u64)
            .lineage(Lineage {
                sweep_id: Some(sweep_id.to_string()),
                parent_run_uid: Some(parent_run_uid.to_string()),
                ..Default::default()
            })
            .llm(llm)
            .replication(record::replication()),
    )
    .expect("runvault: 子 run の開始に失敗");

    let result = run_with_client(cfg, client).unwrap_or_else(|e| panic!("実行に失敗: {e}"));
    record::log_simulation(&mut child, &result);
    record::log_norms(&mut child, &result);
    child.finish().expect("runvault: 子 run の完了に失敗");
    result
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn cmd_run(args: RunArgs) {
    let network = parse_network(&args.network).unwrap_or_else(|e| panic!("{}", e));
    let canonical_mode =
        parse_canonical_mode(&args.canonical_mode).unwrap_or_else(|e| panic!("{}", e));

    // シードを実体化してから記録する．--seed 省略時にシミュレーション側で rand::random
    // に落とすと，実際に使われたシードがどこにも残らない．
    let seed = args.seed.unwrap_or_else(rand::random::<u64>);

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
        seed: Some(seed),
        llm: llm_settings(args.temperature, args.llm_seed, &args.cache_path, args.mock),
    };
    ensure_cache_dir(&cfg.llm);

    // クライアントは run を開始する前に組む（`llm` ブロックのため）．
    let client = build_client(&cfg.llm, args.mock);
    let llm = record::llm_block(
        client.inner().model(),
        client.inner().endpoint(),
        cfg.llm.temperature,
    );

    let parameters = cfg.to_parameters(args.mock);
    let mut rv = Run::start(
        RunOptions::new(EXPERIMENT, "run")
            .repo_id(REPO_ID)
            .domain(DOMAIN)
            .results_root(&args.output_dir)
            .parameters(&parameters)
            .expect("runvault: parameters の組み立てに失敗")
            .hash_exclude(HASH_EXCLUDE)
            .seed_pointers(SEED_POINTERS)
            .master_seed(seed)
            .llm(llm)
            .replication(record::replication()),
    )
    .expect("runvault: run の開始に失敗");

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
        "rounds: {} | θ: {} | K: {} | canonical: {} | seed: {}",
        cfg.rounds,
        cfg.synth_threshold,
        cfg.convergence_window,
        cfg.canonical_mode.label(),
        seed,
    );
    println!(
        "LLM: temp={} llm_seed={} cache={}",
        cfg.llm.temperature,
        cfg.llm.seed,
        cfg.llm.cache_path.as_deref().unwrap_or("(in-memory)"),
    );
    println!(
        "出力先: {}{}",
        rv.dir().display(),
        if args.mock { " | MOCK" } else { "" }
    );
    println!("-------------------------------------------------");

    let result = run_with_client(&cfg, client).unwrap_or_else(|e| panic!("実行に失敗: {}", e));

    record::log_simulation(&mut rv, &result);
    record::log_norms(&mut rv, &result);

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

    let dir = rv.finish().expect("runvault: run の完了に失敗");
    println!("メトリクス → {}/metrics.csv", dir.display());
    println!("規範       → {}/events.jsonl", dir.display());
    println!("設定       → {}/config.json", dir.display());
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
    let n_total = populations.len() * betas.len() * args.runs;

    // sweep はライブ LLM 専用（`--mock` を持たない）．
    let settings = llm_settings(args.temperature, args.llm_seed, &args.cache_path, false);
    ensure_cache_dir(&settings);

    // 親 run: 格子の定義そのものを parameters に持つ．個別条件の指標は書かない．
    // 親は 1 本のシミュレーションではないので master_seed を名乗らない — base seed は
    // /parameters.seed と seed_pointers 経由で execution_hash に残る．
    let sweep_parameters = SweepConfigJson {
        population_values: populations.clone(),
        ws_beta_values: betas.clone(),
        network: network.label().to_string(),
        entrepreneurs: args.entrepreneurs,
        ws_k: args.ws_k,
        er_p: 0.3,
        ba_m: 2,
        runs: args.runs,
        rounds: args.rounds,
        synth_threshold: args.synth_threshold,
        convergence_window: args.convergence_window,
        emergence_threshold: args.emergence_threshold,
        canonical_mode: CanonicalMode::Deterministic.label().to_string(),
        seed: args.seed,
        mock: false,
        llm_temperature: args.temperature,
        llm_seed: args.llm_seed,
        llm_cache_path: settings.cache_path.clone(),
    };
    let parent = Run::start(
        RunOptions::new(EXPERIMENT, "sweep")
            .repo_id(REPO_ID)
            .domain(DOMAIN)
            .results_root(&args.output_dir)
            .parameters(&sweep_parameters)
            .expect("runvault: sweep の parameters の組み立てに失敗")
            .hash_exclude(HASH_EXCLUDE)
            .seed_pointers(SEED_POINTERS)
            .sweep_parent()
            .replication(record::replication()),
    )
    .expect("runvault: sweep 親 run の開始に失敗");

    let sweep_id = parent
        .sweep_id()
        .expect("runvault: sweep 親に sweep_id がありません")
        .to_string();
    let parent_run_uid = parent.run_uid().to_string();

    println!("=== Ren et al. (2024) CRSEC パラメータスイープ ===");
    println!(
        "人口: {:?} | WS-β: {:?} | network: {} | 試行: {} | 合計: {} 実行",
        populations,
        betas,
        network.label(),
        args.runs,
        n_total,
    );
    println!("シード (base): {}", args.seed);
    println!("出力先: {}", parent.dir().display());
    println!("-----------------------------------------------------------");

    let mut done = 0usize;
    // 人口別の平均 最終採用率（コンソール要約用）．
    let mut adoption_by_population: Vec<(usize, Vec<f64>)> =
        populations.iter().map(|&p| (p, Vec::new())).collect();

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
                    llm: settings.clone(),
                };

                let result = run_child(
                    "sweep-point",
                    &cfg,
                    false,
                    seed,
                    run_idx,
                    &args.output_dir,
                    &sweep_id,
                    &parent_run_uid,
                );
                let last = result.metrics_history.last().unwrap();
                if let Some((_, values)) = adoption_by_population
                    .iter_mut()
                    .find(|(p, _)| *p == population)
                {
                    values.push(last.adoption_rate);
                }

                done += 1;
            }
            println!(
                "[{}/{}] population={} ws_beta={:.3} 完了 ({} 試行)",
                done, n_total, population, beta, args.runs,
            );
        }
    }

    let dir = parent
        .finish()
        .expect("runvault: sweep 親 run の完了に失敗");

    println!("===========================================================");
    println!("スイープ完了: {} 実行", n_total);
    println!("-----------------------------------------------------------");
    println!("人口別の平均 最終採用率:");
    for (population, values) in &adoption_by_population {
        if values.is_empty() {
            continue;
        }
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        println!("  N={:<4} → 採用率̄ = {:.3}", population, avg);
    }
    println!("-----------------------------------------------------------");
    println!("親 run → {}", dir.display());
    println!(
        "子 run → {} 本（subcommand=sweep-point; lineage.parent_run_uid={}）",
        n_total, parent_run_uid,
    );
}

// ---------------------------------------------------------------------------
// reproduce
// ---------------------------------------------------------------------------

fn cmd_reproduce(args: ReproduceArgs) {
    let network = parse_network(&args.network).unwrap_or_else(|e| panic!("{}", e));
    let canonical_mode =
        parse_canonical_mode(&args.canonical_mode).unwrap_or_else(|e| panic!("{}", e));

    let params = ReproduceParams {
        population: args.population,
        entrepreneurs: args.entrepreneurs,
        network,
        ws_k: args.ws_k,
        ws_beta: args.ws_beta,
        rounds: args.rounds,
        runs: args.runs,
        emergence_threshold: args.emergence_threshold,
        canonical_mode,
        mock: args.mock,
        quick: args.quick,
        temperature: args.temperature,
        llm_seed: args.llm_seed,
        cache_path: args.cache_path.clone(),
        seed: args.seed,
    };
    let (population, rounds) = reproduce::effective(&params);

    // 試行の設定は seed だけが違う．親の parameters は試行 0 のものから採る．
    let template = reproduce::trial_config(&params, rounds, population, args.seed);
    ensure_cache_dir(&template.llm);

    // 親 run: 条件と試行数を parameters に持ち，試行をまたいだ集約を sweep スコープの
    // 指標として書く．論文の報告値は reference.csv に入る．親は単一の master_seed を
    // 持たない（試行ごとの子が派生シードを持つ）．
    let parent_parameters = ReproduceConfigJson {
        population,
        entrepreneurs: template.entrepreneurs,
        network: network.label().to_string(),
        ws_k: args.ws_k,
        ws_beta: args.ws_beta,
        er_p: template.er_p,
        ba_m: template.ba_m,
        rounds,
        runs: args.runs,
        synth_threshold: template.synth_threshold,
        convergence_window: template.convergence_window,
        emergence_threshold: args.emergence_threshold,
        canonical_mode: canonical_mode.label().to_string(),
        seed: args.seed,
        mock: args.mock,
        llm_temperature: args.temperature,
        llm_seed: args.llm_seed,
        llm_cache_path: template.llm.cache_path.clone(),
    };
    let mut parent = Run::start(
        RunOptions::new(EXPERIMENT, "reproduce")
            .repo_id(REPO_ID)
            .domain(DOMAIN)
            .results_root(&args.output_dir)
            .parameters(&parent_parameters)
            .expect("runvault: reproduce の parameters の組み立てに失敗")
            .hash_exclude(HASH_EXCLUDE)
            .seed_pointers(SEED_POINTERS)
            .sweep_parent()
            .replication(record::replication()),
    )
    .expect("runvault: reproduce 親 run の開始に失敗");

    let sweep_id = parent
        .sweep_id()
        .expect("runvault: reproduce 親に sweep_id がありません")
        .to_string();
    let parent_run_uid = parent.run_uid().to_string();

    println!("=== Ren et al. (2024) CRSEC 見出し的知見 一括再現 ===");
    println!(
        "N: {} | 起業家: {} | network: {} | runs: {} | T: {} | canonical: {} | mode: {}",
        population,
        template.entrepreneurs,
        network.label(),
        args.runs,
        rounds,
        canonical_mode.label(),
        if args.mock { "MOCK" } else { "LIVE" },
    );
    println!("出力先: {}", parent.dir().display());
    println!("-------------------------------------------------");

    // 試行 1 本ずつが模型の別々の実行なので，それぞれを子 run にする．
    let mut trials: Vec<ReproTrial> = Vec::with_capacity(args.runs.max(1));
    for run_idx in 0..args.runs.max(1) {
        let seed = reproduce::trial_seed(args.seed, population, run_idx);
        let cfg = reproduce::trial_config(&params, rounds, population, seed);
        let result = run_child(
            "reproduce-run",
            &cfg,
            args.mock,
            seed,
            run_idx,
            &args.output_dir,
            &sweep_id,
            &parent_run_uid,
        );
        trials.push(ReproTrial::from_result(
            &result,
            args.emergence_threshold,
            rounds,
        ));
    }

    let cell = ReproCell::from_trials(&trials, args.runs);
    let anchors = reproduce::build_anchors(&cell, args.emergence_threshold);

    // --- 親の sweep スコープ指標（観測値） ---
    let observed = cell.observed();
    parent
        .log_metrics(SWEEP_SCOPE, &observed)
        .expect("reproduce 親の集約指標の記録に失敗");

    // --- 論文の報告値 (reference.csv) ---
    for pv in &PAPER_VALUES {
        parent
            .log_reference(pv.name, pv.value)
            .scope(SWEEP_SCOPE)
            .target(pv.target_id)
            .source(pv.source)
            .send()
            .unwrap_or_else(|e| panic!("論文値 {} の記録に失敗: {e}", pv.name));
    }

    // --- コンソール出力 ---
    println!("--- 集計（試行平均; N={} T={}）---", population, rounds);
    println!("最終 採用率̄        : {:.3}", cell.mean_final_adoption);
    println!("最終 遵守率̄        : {:.3}", cell.mean_final_compliance);
    println!(
        "相異規範数 ピーク→最終: {:.2} → {:.2}",
        cell.mean_peak_distinct, cell.mean_final_distinct
    );
    println!(
        "衝突 ピーク→最終     : {:.2} → {:.2}",
        cell.mean_peak_conflicts, cell.mean_final_conflicts
    );
    println!("創発時刻̄ (採用率)   : {:.2}", cell.mean_time_to_emergence);
    println!(
        "創発時刻̄ inj / des   : {:.2} / {:.2}  (Fact 7: inj が先)",
        cell.mean_tte_injunctive, cell.mean_tte_descriptive
    );
    println!(
        "最終採用率̄ inj / des : {:.3} / {:.3}",
        cell.mean_final_adoption_injunctive, cell.mean_final_adoption_descriptive
    );
    println!("収束した試行割合     : {:.2}", cell.converged_frac);

    // 許容帯は論文の主張ではなく本実装が置いたものなので記録しない（コンソールだけ）．
    println!("--- 論文知見アンカー（観測 vs 論文）---");
    for a in &anchors {
        let hi = if a.target_hi.is_infinite() {
            "∞".to_string()
        } else {
            format!("{:.3}", a.target_hi)
        };
        println!(
            "[{}] {:<56} obs={:.4} target=[{:.3},{}]",
            if a.pass { "PASS" } else { "OFF " },
            a.name,
            a.observed,
            a.target_lo,
            hi,
        );
    }
    let n_pass = anchors.iter().filter(|a| a.pass).count();
    println!("-------------------------------------------------");
    println!("{}/{} アンカーが in-band", n_pass, anchors.len());

    let dir = parent
        .finish()
        .expect("runvault: reproduce 親 run の完了に失敗");
    println!("集約     → {}/metrics.csv", dir.display());
    println!("論文値   → {}/reference.csv", dir.display());
    println!(
        "試行     → 子 run {} 本（subcommand=reproduce-run）",
        args.runs.max(1),
    );
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();
    if let Some(host) = cli.ollama_host.as_deref() {
        std::env::set_var("OLLAMA_HOST", host);
    }
    match cli.command {
        Commands::Run(args) => cmd_run(args),
        Commands::Sweep(args) => cmd_sweep(args),
        Commands::Reproduce(args) => cmd_reproduce(args),
    }
}
