//! 初期化と実行ドライバ（SimulationBuilder 配線 + 二層 LLM レイヤ）．
//!
//! 二層決定論を配線する:
//! - **下層（決定論的 socsim コア）**: `derive_seed(root, &[0])` で網生成・プロフィール
//!   /規範起業家割当の init RNG を，`derive_seed(root, &[1])` で engine RNG
//!   （= 会話・観察相手のサンプリング・活性化順）を派生する．bit 単位で再現する．
//! - **上層（非決定的 LLM レイヤ）**: [`crate::llm`] のキャッシュ付き Ollama→OpenAI
//!   フォールバッククライアントに閉じ込め，`temperature=0`/`seed` 固定 + プロンプト→
//!   応答キャッシュで擬似決定論化する．モデル・endpoint・温度・seed・cache-hit を
//!   `run_metadata.json` に記録する．

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::rc::Rc;

use csv::Writer;
use serde::Serialize;

use socsim_core::{derive_seed, AgentId, SimRng};
use socsim_engine::{RandomActivationScheduler, SimulationBuilder};
use socsim_llm::MetadataCollector;

use crate::config::{Config, Network};
use crate::llm::{build_live_client, CrsecClient};
use crate::mechanisms::{
    empty_norm_db, ComplianceMechanism, ConvergenceMechanism, CreationMechanism,
    EvaluationMechanism, ResetInteractions, SharedClient, SharedMetadata, SpreadingMechanism,
    SCRATCH_COMPLIED, SCRATCH_CONFLICTS, SCRATCH_CONVERGED,
};
use crate::metrics::{time_to_emergence, Metrics};
use crate::norm::PersonalNorm;
use crate::world::{AgentProfile, CrsecWorld};

/// 網生成・プロフィール/規範起業家割当用 RNG ラベル．
const RNG_WORLD_INIT: u64 = 0;
/// socsim エンジン（= 活性化順・相手サンプリング）用 RNG ラベル．
const RNG_ENGINE: u64 = 1;

/// 初期プロフィールのテンプレート（ラウンドロビンで割当; 決定論的）．名前 + 性格．
const PERSONAS: [&str; 6] = [
    "Alex, an outgoing regular who dislikes loud noise",
    "Blair, a quiet patron who values tidiness",
    "Casey, a sociable visitor who enjoys lively chatter",
    "Devin, a thoughtful local who cares about fairness",
    "Erin, a busy professional who prefers calm spaces",
    "Frankie, a friendly newcomer still learning the norms",
];

/// 初期選好の候補（価値観対立の源; ラウンドロビンで割当）．
const PREFERENCES: [&[&str]; 6] = [
    &["dislikes smoking indoors", "prefers quiet"],
    &["wants a clean shared space", "dislikes loud phone calls"],
    &["enjoys music and chatter", "tolerant of smoking"],
    &["values tipping the staff", "dislikes queue-jumping"],
    &["prefers calm and order", "dislikes clutter"],
    &["easygoing", "open to any house rule"],
];

// 規範の創出は完全に LLM フェーズ（CreationMechanism）に委ねるため，init では
// norm_db を空にしておく（起業家は DB 空のとき CreateNorm を呼ぶ）．

/// シミュレーション全体の実行結果．
pub struct SimulationResult {
    /// 各ラウンド（t=0 を含む）のメトリクス履歴．
    pub metrics_history: Vec<Metrics>,
    /// 収束したか（適格集合が安定）．
    pub converged: bool,
    /// 収束（または最終）ラウンド番号．
    pub final_step: usize,
    /// 創発までのラウンド（採用率 ≥ threshold の最初; None なら未創発）．
    pub time_to_emergence: Option<usize>,
    /// 最終的な各エージェントの適格規範DB（出力用）．
    pub final_norm_db: BTreeMap<AgentId, Vec<PersonalNorm>>,
    /// LLM 呼び出しメタデータの集計．
    pub metadata: MetadataCollector,
    /// LLM モデル名（run_metadata 用）．
    pub llm_model: String,
    /// LLM endpoint（run_metadata 用; primary）．
    pub llm_endpoint: String,
}

/// 世界状態を初期化する（網生成 + プロフィール/規範起業家割当 + 空の規範DB）．
///
/// トポロジに応じて `socsim-net` の生成器を使う．先頭 `entrepreneurs` 名を起業家と
/// する（決定論的）．規範の創出は LLM に委ねるため norm_db は空で開始する．
pub fn init_world(cfg: &Config, rng: &mut SimRng) -> CrsecWorld {
    let ids: Vec<AgentId> = (0..cfg.population as u64).map(AgentId).collect();

    let network = match cfg.network {
        Network::WattsStrogatz => {
            socsim_net::SocialNetwork::watts_strogatz(&ids, cfg.ws_k, cfg.ws_beta, rng)
        }
        Network::ErdosRenyi => socsim_net::SocialNetwork::erdos_renyi(&ids, cfg.er_p, rng),
        Network::BarabasiAlbert => socsim_net::SocialNetwork::barabasi_albert(&ids, cfg.ba_m, rng),
    };

    let n_entre = cfg.entrepreneurs.min(cfg.population);
    let mut agents: BTreeMap<AgentId, AgentProfile> = BTreeMap::new();
    for (idx, &id) in ids.iter().enumerate() {
        let description = PERSONAS[idx % PERSONAS.len()].to_string();
        let preferences: Vec<String> = PREFERENCES[idx % PREFERENCES.len()]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let is_entrepreneur = idx < n_entre;
        agents.insert(
            id,
            AgentProfile::new(description, is_entrepreneur, preferences),
        );
    }

    let norm_db = empty_norm_db(&ids);

    CrsecWorld::new(
        network,
        agents,
        norm_db,
        cfg.synth_threshold,
        cfg.rounds as u64,
    )
}

/// シミュレーションを実行する（本番 LLM クライアントを構築して駆動）．
pub fn run(cfg: &Config) -> Result<SimulationResult, String> {
    let client =
        build_live_client(&cfg.llm).map_err(|e| format!("LLM クライアント構築に失敗: {e}"))?;
    run_with_client(cfg, client)
}

/// 与えられた [`CrsecClient`] でシミュレーションを実行する．
///
/// 本番は [`build_live_client`] の結果を，テストは [`crate::llm::wrap_client`] で
/// ラップした `mock::ScriptedClient` を渡す．
pub fn run_with_client(cfg: &Config, client: CrsecClient) -> Result<SimulationResult, String> {
    let root = cfg.seed.unwrap_or_else(rand::random);

    let mut init_rng = SimRng::from_seed(derive_seed(root, &[RNG_WORLD_INIT]));
    let world = init_world(cfg, &mut init_rng);

    let llm_model = client.inner().model().to_string();
    let llm_endpoint = client.inner().endpoint().to_string();

    let shared_client: SharedClient = Rc::new(RefCell::new(client));
    let shared_meta: SharedMetadata = Rc::new(RefCell::new(MetadataCollector::new()));

    let mut sim = SimulationBuilder::new(world)
        .scheduler(Box::new(RandomActivationScheduler))
        .seed(derive_seed(root, &[RNG_ENGINE]))
        .add_mechanism(Box::new(ResetInteractions))
        .add_mechanism(Box::new(CreationMechanism::new(
            Rc::clone(&shared_client),
            Rc::clone(&shared_meta),
            cfg.llm.clone(),
        )))
        .add_mechanism(Box::new(ComplianceMechanism::new(
            Rc::clone(&shared_client),
            Rc::clone(&shared_meta),
            cfg.llm.clone(),
        )))
        .add_mechanism(Box::new(SpreadingMechanism::new(
            Rc::clone(&shared_client),
            Rc::clone(&shared_meta),
            cfg.llm.clone(),
        )))
        .add_mechanism(Box::new(EvaluationMechanism::new(
            Rc::clone(&shared_client),
            Rc::clone(&shared_meta),
            cfg.llm.clone(),
        )))
        .add_mechanism(Box::new(ConvergenceMechanism::new(cfg.convergence_window)))
        .build();

    let mut metrics_history: Vec<Metrics> = Vec::new();

    // 初期状態（t=0）を記録（規範皆無; 採用率 0）．
    metrics_history.push(Metrics::compute(sim.world().norm_db_ref(), 0, 0, 0));

    let mut converged = false;
    let mut final_step = 0usize;

    sim.run_observed(|report| {
        let t = report.t as usize;
        let complied = *report.scratch.get::<usize>(SCRATCH_COMPLIED).unwrap_or(&0);
        let conflicts = *report.scratch.get::<usize>(SCRATCH_CONFLICTS).unwrap_or(&0);
        metrics_history.push(Metrics::compute(
            &report.world.norm_db,
            complied,
            conflicts,
            t,
        ));
        converged = *report
            .scratch
            .get::<bool>(SCRATCH_CONVERGED)
            .unwrap_or(&false);
        final_step = t;
    })
    .map_err(|e| format!("シミュレーションの実行に失敗: {e}"))?;

    // キャッシュを保存（cache_path 指定時; in-memory はスキップ）．
    if cfg.llm.cache_path.is_some() {
        let client = shared_client.borrow();
        client
            .cache()
            .save()
            .map_err(|e| format!("キャッシュ保存に失敗: {e}"))?;
    }

    let adoptions: Vec<f64> = metrics_history.iter().map(|m| m.adoption_rate).collect();
    let tte = time_to_emergence(&adoptions, cfg.emergence_threshold);

    let final_norm_db = sim.world().norm_db.clone();
    let metadata = shared_meta.borrow().clone();

    Ok(SimulationResult {
        metrics_history,
        converged,
        final_step,
        time_to_emergence: tte,
        final_norm_db,
        metadata,
        llm_model,
        llm_endpoint,
    })
}

// `Metrics::compute` は &BTreeMap を取るので世界からの借用ヘルパを足す．
impl CrsecWorld {
    /// 規範DB への参照（t=0 メトリクス用）．
    pub fn norm_db_ref(&self) -> &BTreeMap<AgentId, Vec<PersonalNorm>> {
        &self.norm_db
    }
}

// --------------------------------------------------------------------------- //
// 出力
// --------------------------------------------------------------------------- //

/// メトリクス履歴を CSV に保存する．
pub fn save_metrics(metrics: &[Metrics], output_dir: &str) {
    let path = format!("{}/metrics.csv", output_dir);
    let file = File::create(&path).expect("metrics.csv の作成に失敗");
    let mut wtr = Writer::from_writer(BufWriter::new(file));
    for m in metrics {
        wtr.serialize(m).expect("メトリクス書き込みに失敗");
    }
    wtr.flush().expect("フラッシュに失敗");
}

/// 最終的な適格規範をエージェント別に long-format CSV に保存する．
pub fn save_norms(result: &SimulationResult, output_dir: &str) {
    let path = format!("{}/norms.csv", output_dir);
    let file = File::create(&path).expect("norms.csv の作成に失敗");
    let mut wtr = Writer::from_writer(BufWriter::new(file));
    wtr.write_record([
        "agent_id",
        "content",
        "type",
        "utility",
        "s_act",
        "s_val",
        "qualified",
    ])
    .expect("ヘッダ書き込みに失敗");
    for (&AgentId(id), norms) in &result.final_norm_db {
        for n in norms {
            wtr.write_record(&[
                id.to_string(),
                n.content.replace(['\n', '\r'], " "),
                n.alpha.label().to_string(),
                n.utility.to_string(),
                n.s_act.to_string(),
                n.s_val.to_string(),
                n.qualified().to_string(),
            ])
            .expect("レコード書き込みに失敗");
        }
    }
    wtr.flush().expect("フラッシュに失敗");
}

/// `run_metadata.json` の構造体（LLM モデル・endpoint・温度・seed・cache 統計）．
#[derive(Serialize)]
pub struct RunMetadataJson {
    pub llm_model: String,
    pub llm_endpoint: String,
    pub llm_temperature: f32,
    pub llm_seed: u64,
    pub total_calls: usize,
    pub cache_hits: usize,
    pub cache_hit_rate: f64,
    pub converged: bool,
    pub final_step: usize,
    pub time_to_emergence: Option<usize>,
    pub determinism_note: &'static str,
}

/// `run_metadata.json` を保存する．
pub fn save_run_metadata(result: &SimulationResult, cfg: &Config, output_dir: &str) {
    let meta = RunMetadataJson {
        llm_model: result.llm_model.clone(),
        llm_endpoint: result.llm_endpoint.clone(),
        llm_temperature: cfg.llm.temperature,
        llm_seed: cfg.llm.seed,
        total_calls: result.metadata.total(),
        cache_hits: result.metadata.cache_hits(),
        cache_hit_rate: result.metadata.cache_hit_rate(),
        converged: result.converged,
        final_step: result.final_step,
        time_to_emergence: result.time_to_emergence,
        determinism_note: "LLM output is outside socsim bit-reproducibility; the prompt->response \
                           cache (with temperature=0 and fixed seed) is the reproducibility \
                           mechanism. The socsim core (network, activation order, partner \
                           sampling, scheduling, metrics, canonical-norm-identity) is \
                           deterministic given the seed.",
    };
    let path = format!("{}/run_metadata.json", output_dir);
    let file = File::create(&path).expect("run_metadata.json の作成に失敗");
    serde_json::to_writer_pretty(BufWriter::new(file), &meta)
        .expect("run_metadata.json の書き込みに失敗");
}

/// 出力ディレクトリを作成する．
pub fn ensure_output_dir(output_dir: &str) {
    fs::create_dir_all(output_dir).expect("出力ディレクトリの作成に失敗");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::wrap_client;
    use socsim_llm::mock::ScriptedClient;
    use socsim_llm::PromptCache;

    /// 規範ライフサイクルを駆動する mock: 創出・伝播・評価で一貫した規範を返す．
    fn scripted_client() -> CrsecClient {
        let backend = ScriptedClient::new("mock-crsec", |prompt: &str| {
            if prompt.contains("propose ONE social norm") {
                // CreateNorm
                "CONTENT: no smoking indoors\nTYPE: injunctive\nUTILITY: 80".to_string()
            } else if prompt.contains("Decide on your next action") {
                // Compliance
                "COMPLY: yes\nACTION: I refrain from smoking indoors.".to_string()
            } else if prompt.contains("Analyse the interaction") {
                // Spreading: 会話・観察で常に同じ規範を識別させる．
                "CONFLICT: yes\nTALK: yes\nNORM: no smoking indoors\nTYPE: injunctive\nUTILITY: 80"
                    .to_string()
            } else if prompt.contains("Run four sanity checks") {
                // Evaluation: 昇格させる．
                "CONSISTENT: yes\nDUPLICATE: no\nTYPE_OK: yes\nCONFLICTS: no\nPROMOTE: yes"
                    .to_string()
            } else {
                "none".to_string()
            }
        });
        wrap_client(backend, PromptCache::in_memory())
    }

    fn test_config() -> Config {
        Config {
            population: 6,
            entrepreneurs: 2,
            network: Network::WattsStrogatz,
            ws_k: 2,
            rounds: 10,
            convergence_window: 100, // 収束で早期停止しないよう大きく
            synth_threshold: 1e9,    // 統合させない
            seed: Some(42),
            ..Config::default()
        }
    }

    #[test]
    fn scripted_run_reaches_high_adoption() {
        let cfg = test_config();
        let result = run_with_client(&cfg, scripted_client()).unwrap();
        let last = result.metrics_history.last().unwrap();
        // 伝播 + 昇格で規範が集団へ広がり採用率が上昇する．
        assert!(last.adoption_rate > 0.5, "adoption={}", last.adoption_rate);
        assert_eq!(result.metrics_history[0].t, 0);
    }

    #[test]
    fn core_is_deterministic_given_mock() {
        let cfg = test_config();
        let a = run_with_client(&cfg, scripted_client()).unwrap();
        let b = run_with_client(&cfg, scripted_client()).unwrap();
        let av: Vec<f64> = a.metrics_history.iter().map(|m| m.adoption_rate).collect();
        let bv: Vec<f64> = b.metrics_history.iter().map(|m| m.adoption_rate).collect();
        assert_eq!(av, bv);
        assert_eq!(a.final_step, b.final_step);
    }
}
