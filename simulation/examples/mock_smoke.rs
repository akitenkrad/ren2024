//! Mock 駆動のスモーク実行（ライブ LLM 不要）．
//!
//! ライブ Ollama/OpenAI が使えない環境（CI・ネットワーク遮断サンドボックス）で
//! 出力パイプライン（metrics.csv / norms.csv / run_metadata.json / latest）と Python
//! 可視化を検証するための補助バイナリ．`socsim-llm::mock::ScriptedClient` で
//! 決定論的に規範ライフサイクルを駆動し，本番 `run` と同じ writer で結果を書き出す．
//!
//! 規範起業家が "no smoking indoors"（injunctive）を創出し，伝播・評価を経て集団へ
//! 広がる小社会を再現する．採用率が 1.0 へ上昇し，衝突が立ち上がってから減るさまを
//! 確認できる．
//!
//! ```bash
//! cargo run --release --example mock_smoke -- results
//! ```

use std::env;

use socsim_results::{refresh_latest_symlink, timestamp, write_json};

use crsec_simulation::config::{Config, Network};
use crsec_simulation::llm::wrap_client;
use crsec_simulation::simulation::{
    ensure_output_dir, run_with_client, save_metrics, save_norms, save_run_metadata,
};
use socsim_llm::mock::ScriptedClient;
use socsim_llm::PromptCache;

fn main() {
    let base = env::args().nth(1).unwrap_or_else(|| "results".to_string());
    let timestamp = timestamp();
    let output_dir = format!("{base}/{timestamp}");

    let cfg = Config {
        population: 8,
        entrepreneurs: 2,
        network: Network::WattsStrogatz,
        ws_k: 4,
        ws_beta: 0.2,
        rounds: 12,
        convergence_window: 4,
        synth_threshold: 1e9, // 統合させず単純な創発を観察
        seed: Some(42),
        output_dir: output_dir.clone(),
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
    let client = wrap_client(backend, PromptCache::in_memory());

    ensure_output_dir(&cfg.output_dir);
    let result = run_with_client(&cfg, client).expect("mock run failed");
    save_metrics(&result.metrics_history, &cfg.output_dir);
    save_norms(&result, &cfg.output_dir);
    save_run_metadata(&result, &cfg, &cfg.output_dir);

    // config.json (socsim_results::write_json に委譲)．
    let cfg_path = format!("{}/config.json", cfg.output_dir);
    write_json(&cfg.to_run_config_json(), &cfg_path).unwrap();

    // latest symlink (socsim_results に委譲)．
    let _ = refresh_latest_symlink(&base, &timestamp);

    let last = result.metrics_history.last().unwrap();
    let peak = result
        .metrics_history
        .iter()
        .map(|m| m.n_conflicts)
        .max()
        .unwrap_or(0);
    println!("mock smoke wrote: {output_dir}");
    println!(
        "final adoption={:.3} compliance={:.3} distinct_norms={} peak_conflicts={} steps={}",
        last.adoption_rate, last.compliance_rate, last.n_distinct_norms, peak, result.final_step,
    );
}
