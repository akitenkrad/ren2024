//! Mock 駆動のスモーク実行（ライブ LLM 不要）．
//!
//! ライブ Ollama/OpenAI が使えない環境（CI・ネットワーク遮断サンドボックス）で
//! 記録パイプライン（runvault の run ディレクトリ・`metrics.csv`・`events.jsonl`）と
//! Python 可視化を検証するための補助バイナリ．`socsim-llm::mock::ScriptedClient` で
//! 決定論的に規範ライフサイクルを駆動し，本番 `run` と同じ記録経路を通す．
//!
//! 規範起業家が "no smoking indoors"（injunctive）を創出し，伝播・評価を経て集団へ
//! 広がる小社会を再現する．採用率が 1.0 へ上昇し，衝突が立ち上がってから減るさまを
//! 確認できる．
//!
//! `run --mock` とは別の scripted 応答（衝突の rise-then-fall を «相手が規範を持つか»
//! で近似する）を持つので，サブコマンド名も `mock-smoke` と分けてある．
//!
//! ```bash
//! cargo run --release --example mock_smoke -- results
//! ```

use std::env;

use runvault::{Run, RunOptions};

use crsec_simulation::config::{Config, Network, HASH_EXCLUDE, SEED_POINTERS};
use crsec_simulation::llm::wrap_client;
use crsec_simulation::record::{self, DOMAIN, EXPERIMENT, REPO_ID};
use crsec_simulation::simulation::run_with_client;
use socsim_llm::mock::ScriptedClient;
use socsim_llm::{LlmClient, PromptCache};

fn main() {
    let results_root = env::args().nth(1).unwrap_or_else(|| "results".to_string());
    let seed = 42u64;

    let cfg = Config {
        population: 8,
        entrepreneurs: 2,
        network: Network::WattsStrogatz,
        ws_k: 4,
        ws_beta: 0.2,
        rounds: 12,
        convergence_window: 4,
        synth_threshold: 1e9, // 統合させず単純な創発を観察
        seed: Some(seed),
        ..Config::default()
    };

    // 規範ライフサイクルを駆動する mock．
    // - 序盤は衝突を立て（CONFLICT: yes），規範を伝播・昇格させる．
    // - 規範が普及すると衝突は自然に減る（適格保有者が増え DetectConflict の頻度が
    //   下がる挙動を，会話相手がすでに同じ規範を持つかで近似）．
    let backend = ScriptedClient::new("mock-llama3.2", |prompt: &str| {
        if prompt.contains("propose ONE social norm") {
            "CONTENT: no smoking indoors\nTYPE: injunctive\nUTILITY: 85".to_string()
        } else if prompt.contains("Decide on your next action") {
            "COMPLY: yes\nACTION: I refrain from smoking indoors.".to_string()
        } else if prompt.contains("Analyse the interaction") {
            // 送信者がまだ規範を持たないとき（norms 行が "(none yet)"）は衝突あり，
            // 持っているときは衝突なしとして «初期急増→減少» を表現する．
            let conflict = if prompt.contains("(none yet)") {
                "yes"
            } else {
                "no"
            };
            format!(
                "CONFLICT: {conflict}\nTALK: yes\nNORM: no smoking indoors\nTYPE: injunctive\nUTILITY: 85"
            )
        } else if prompt.contains("Run four sanity checks") {
            "CONSISTENT: yes\nDUPLICATE: no\nTYPE_OK: yes\nCONFLICTS: no\nPROMOTE: yes".to_string()
        } else {
            "none".to_string()
        }
    });
    // `llm` ブロックはクライアントを組んだ側でしか埋められないので，run を起こす前に採る．
    let model = backend.model().to_string();
    let endpoint = backend.endpoint().to_string();
    let client = wrap_client(backend, PromptCache::in_memory());

    let parameters = cfg.to_parameters(true);
    let mut rv = Run::start(
        RunOptions::new(EXPERIMENT, "mock-smoke")
            .repo_id(REPO_ID)
            .domain(DOMAIN)
            .results_root(&results_root)
            .parameters(&parameters)
            .expect("runvault: parameters の組み立てに失敗")
            .hash_exclude(HASH_EXCLUDE)
            .seed_pointers(SEED_POINTERS)
            .master_seed(seed)
            .llm(record::llm_block(&model, &endpoint, cfg.llm.temperature))
            .replication(record::replication()),
    )
    .expect("runvault: run の開始に失敗");

    let result = run_with_client(&cfg, client).expect("mock run failed");
    record::log_simulation(&mut rv, &result);
    record::log_norms(&mut rv, &result);

    let last = result.metrics_history.last().unwrap();
    let peak = result
        .metrics_history
        .iter()
        .map(|m| m.n_conflicts)
        .max()
        .unwrap_or(0);
    let dir = rv.finish().expect("runvault: run の完了に失敗");
    println!("mock smoke wrote: {}", dir.display());
    println!(
        "final adoption={:.3} compliance={:.3} distinct_norms={} peak_conflicts={} steps={}",
        last.adoption_rate, last.compliance_rate, last.n_distinct_norms, peak, result.final_step,
    );
}
