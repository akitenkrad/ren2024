//! Ren et al. (2024) CRSEC 規範ライフサイクルの統合テスト．
//!
//! **ライブ LLM を一切必要としない**: socsim-llm の `mock::ScriptedClient` で
//! 決定論的にライフサイクルを駆動し，以下を検証する:
//! ・規範伝播（起業家の規範がネットワーク経由で集団へ広がる）
//! ・適格集合への昇格（評価フェーズで s_act=T, s_val=T へ）
//! ・canonical-norm-identity を用いた採用率計算（パラフレーズの吸収）
//! ・収束 / request_stop（適格集合が安定したら停止）
//! ・RNG 決定論（同一シード + 同一 mock → 完全再現）

use crsec_simulation::config::{Config, Network};
use crsec_simulation::llm::{wrap_client, CrsecClient};
use crsec_simulation::metrics::time_to_emergence;
use crsec_simulation::simulation::run_with_client;
use crsec_simulation::world::canonical_key;

use socsim_llm::mock::ScriptedClient;
use socsim_llm::PromptCache;

/// 一貫した規範 "no smoking indoors" を創出・伝播・昇格させる mock．
/// `paraphrase=true` のとき伝播時の識別規範を言い回し違いにして canonical 同定を試す．
fn scripted(paraphrase: bool) -> CrsecClient {
    let backend = ScriptedClient::new("mock-crsec", move |prompt: &str| {
        if prompt.contains("propose ONE social norm") {
            "CONTENT: no smoking indoors\nTYPE: injunctive\nUTILITY: 80".to_string()
        } else if prompt.contains("Decide on your next action") {
            "COMPLY: yes\nACTION: refrain from smoking.".to_string()
        } else if prompt.contains("Analyse the interaction") {
            let norm = if paraphrase {
                "Indoors, you should not be smoking"
            } else {
                "no smoking indoors"
            };
            format!("CONFLICT: yes\nTALK: yes\nNORM: {norm}\nTYPE: injunctive\nUTILITY: 80")
        } else if prompt.contains("Run four sanity checks") {
            "CONSISTENT: yes\nDUPLICATE: no\nTYPE_OK: yes\nCONFLICTS: no\nPROMOTE: yes".to_string()
        } else {
            "none".to_string()
        }
    });
    wrap_client(backend, PromptCache::in_memory())
}

fn base_config() -> Config {
    Config {
        population: 6,
        entrepreneurs: 2,
        network: Network::WattsStrogatz,
        ws_k: 4,
        ws_beta: 0.1,
        rounds: 15,
        convergence_window: 100, // 早期停止させない（普及しきるまで回す）
        synth_threshold: 1e9,    // 統合させない
        emergence_threshold: 0.9,
        seed: Some(7),
        ..Config::default()
    }
}

// --------------------------------------------------------------------------- //
// 伝播 + 昇格 → 採用率が上昇する
// --------------------------------------------------------------------------- //

#[test]
fn norm_propagates_and_adoption_rises() {
    let cfg = base_config();
    let result = run_with_client(&cfg, scripted(false)).unwrap();
    let first = result.metrics_history[0].adoption_rate;
    let last = result.metrics_history.last().unwrap().adoption_rate;
    assert_eq!(first, 0.0, "t=0 は規範皆無で採用率 0");
    assert!(last > first, "伝播・昇格で採用率は上昇する (last={last})");
    assert!(
        last >= 0.5,
        "十分なラウンドで過半が規範を共有する (last={last})"
    );
}

// --------------------------------------------------------------------------- //
// 適格集合への昇格: 最終 DB に qualified な規範が存在する
// --------------------------------------------------------------------------- //

#[test]
fn norms_get_promoted_to_qualified() {
    let cfg = base_config();
    let result = run_with_client(&cfg, scripted(false)).unwrap();
    let qualified_holders = result
        .final_norm_db
        .values()
        .filter(|norms| norms.iter().any(|n| n.qualified()))
        .count();
    assert!(
        qualified_holders >= 2,
        "起業家以外にも昇格規範が広がる (holders={qualified_holders})"
    );
}

// --------------------------------------------------------------------------- //
// canonical-norm-identity: パラフレーズされても同じ規範として採用率に計上される
// --------------------------------------------------------------------------- //

#[test]
fn paraphrased_norms_bucket_to_same_canonical() {
    // 起業家は "no smoking indoors" を保持，伝播では別の言い回しを識別させる．
    // canonical_key が両者を束ねるなら採用率は分裂せず高くなる．
    let cfg = base_config();
    let result = run_with_client(&cfg, scripted(true)).unwrap();
    let last = result.metrics_history.last().unwrap();
    // 言い回しが違っても canonical 同定で 1 規範に束ねられ，相異規範数は小さい．
    assert!(
        last.n_distinct_norms <= 2,
        "パラフレーズは canonical 同定で束ねられる (distinct={})",
        last.n_distinct_norms
    );
    assert!(
        last.adoption_rate >= 0.5,
        "束ねられた規範の採用率は高い (={})",
        last.adoption_rate
    );
    // canonical_key の健全性: 2 つの言い回しのキーが大きく重なる．
    let a = canonical_key("no smoking indoors");
    let b = canonical_key("Indoors, you should not be smoking");
    assert_eq!(
        a, b,
        "語順・主語・冠詞の揺れは同一 canonical key へ束ねられる"
    );
}

// --------------------------------------------------------------------------- //
// 収束 / request_stop: ウィンドウを小さくすれば安定後に停止する
// --------------------------------------------------------------------------- //

#[test]
fn convergence_requests_stop_when_stable() {
    let mut cfg = base_config();
    cfg.rounds = 40;
    cfg.convergence_window = 3; // 適格集合が 3 ラウンド不変なら停止
    let result = run_with_client(&cfg, scripted(false)).unwrap();
    assert!(result.converged, "安定後は収束フラグが立つべき");
    assert!(
        result.final_step < 40,
        "rounds 上限より前に停止する (step={})",
        result.final_step
    );
    // 創発時刻が記録される（採用率がしきいを超える）．
    let adoptions: Vec<f64> = result
        .metrics_history
        .iter()
        .map(|m| m.adoption_rate)
        .collect();
    let _ = time_to_emergence(&adoptions, cfg.emergence_threshold);
}

// --------------------------------------------------------------------------- //
// 決定論性: 同一シード + 同一 mock → 完全再現（socsim コア層）
// --------------------------------------------------------------------------- //

#[test]
fn core_is_deterministic_given_fixed_mock() {
    let cfg = base_config();
    let a = run_with_client(&cfg, scripted(false)).unwrap();
    let b = run_with_client(&cfg, scripted(false)).unwrap();
    let av: Vec<f64> = a.metrics_history.iter().map(|m| m.adoption_rate).collect();
    let bv: Vec<f64> = b.metrics_history.iter().map(|m| m.adoption_rate).collect();
    assert_eq!(av, bv, "同一シードは完全再現すべき");
    assert_eq!(a.final_step, b.final_step);
}

// --------------------------------------------------------------------------- //
// ネットワーク種ごとに正しいノード数で組み上がる
// --------------------------------------------------------------------------- //

#[test]
fn networks_build_with_correct_population() {
    for net in [
        Network::WattsStrogatz,
        Network::ErdosRenyi,
        Network::BarabasiAlbert,
    ] {
        let mut cfg = base_config();
        cfg.network = net;
        cfg.population = 8;
        cfg.rounds = 3;
        let result = run_with_client(&cfg, scripted(false)).unwrap();
        assert_eq!(
            result.final_norm_db.len(),
            8,
            "{:?}: 全 8 エージェントが DB を持つ",
            net
        );
    }
}
