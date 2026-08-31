//! シミュレーション設定．
//!
//! Ren et al. (2024) CRSEC のコアモデル（規範ライフサイクル）と感度分析パラメータを
//! 保持する [`Config`] と，その JSON シリアライズ表現を定義する．社会接続トポロジ・
//! LLM 設定もここに集約する．

use serde::Serialize;

// --------------------------------------------------------------------------- //
// ネットワークトポロジ
// --------------------------------------------------------------------------- //

/// 社会接続網のトポロジ（誰が誰と会話・観察するか）．
///
/// CRSEC は非空間モデルなので，相互作用は **無向** の社会接続に沿う．既定は
/// Watts–Strogatz 小世界（局所クラスタ + 少数のショートカット）．ER/BA も比較用
/// に生成可能（`socsim-net` 生成器）．
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// Watts–Strogatz 小世界網（既定）．
    WattsStrogatz,
    /// Erdős–Rényi ランダム網．
    ErdosRenyi,
    /// Barabási–Albert スケールフリー網．
    BarabasiAlbert,
}

impl Network {
    /// 出力・キャッシュキー用の安定ラベル．
    pub fn label(&self) -> &'static str {
        match self {
            Network::WattsStrogatz => "ws",
            Network::ErdosRenyi => "er",
            Network::BarabasiAlbert => "ba",
        }
    }
}

/// 文字列から [`Network`] をパースする．
pub fn parse_network(s: &str) -> Result<Network, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "ws" | "watts_strogatz" | "watts-strogatz" | "smallworld" => Ok(Network::WattsStrogatz),
        "er" | "erdos_renyi" | "erdos-renyi" | "random" => Ok(Network::ErdosRenyi),
        "ba" | "barabasi_albert" | "barabasi-albert" | "scalefree" => Ok(Network::BarabasiAlbert),
        _ => Err(format!(
            "不正なネットワーク: \"{}\" (ws / er / ba のいずれか)",
            s
        )),
    }
}

// --------------------------------------------------------------------------- //
// 規範同定（canonical-norm-identity）
// --------------------------------------------------------------------------- //

/// 規範の canonical key（同一性）を決める方式．
///
/// LLM が生成する自然言語記述 c は言い回しが揺れるため，採用率の集計には正規化済み
/// の canonical key で規範を束ねる必要がある（論文 Fig. 2 の採用率に対応）．
///
/// - `Deterministic`（既定）: 規則ベースの決定論的正規化（小文字化・トリム・
///   ストップワード除去・キーワード集合化）．**追加の LLM 呼び出しを要しない**ため
///   二層決定論の下層に閉じ，metrics が安価に再現可能．
/// - `Llm`: LLM による意味的重複判定（キャッシュ付き）．高精度だがコスト増（拡張点）．
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalMode {
    /// 規則ベースの決定論的正規化（既定）．
    Deterministic,
    /// LLM による意味的重複判定（キャッシュ付き; 拡張点）．
    Llm,
}

impl CanonicalMode {
    pub fn label(&self) -> &'static str {
        match self {
            CanonicalMode::Deterministic => "deterministic",
            CanonicalMode::Llm => "llm",
        }
    }
}

/// 文字列から [`CanonicalMode`] をパースする．
pub fn parse_canonical_mode(s: &str) -> Result<CanonicalMode, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "deterministic" | "rule" | "rules" => Ok(CanonicalMode::Deterministic),
        "llm" => Ok(CanonicalMode::Llm),
        _ => Err(format!(
            "不正な canonical-mode: \"{}\" (deterministic / llm)",
            s
        )),
    }
}

// --------------------------------------------------------------------------- //
// LLM 設定
// --------------------------------------------------------------------------- //

/// LLM レイヤの設定 (provider / model / temperature / seed / cache)．
///
/// 定義は `socsim-llm` に集約済み (各 replication で同一だった struct を統合)．
/// `crate::config::LlmSettings` パスは re-export で温存する．
pub use socsim_llm::LlmSettings;

// --------------------------------------------------------------------------- //
// Config
// --------------------------------------------------------------------------- //

/// 単一実行の設定．
#[derive(Debug, Clone)]
pub struct Config {
    /// 人口（エージェント数 N）．
    pub population: usize,
    /// 規範起業家の人数（初期に規範を保持し伝播を駆動する少数）．
    pub entrepreneurs: usize,
    /// 社会接続トポロジ（ws / er / ba）．
    pub network: Network,
    /// WS の各ノードの初期次数 k（偶数）．
    pub ws_k: usize,
    /// WS の再配線確率 β．
    pub ws_beta: f64,
    /// ER の辺生成確率 p．
    pub er_p: f64,
    /// BA の新規ノードあたりの結合数 m．
    pub ba_m: usize,
    /// ラウンド数 T（1 ラウンド = 全エージェントが創出・伝播・評価・遵守を一巡）．
    pub rounds: usize,
    /// 長期統合の有用性閾値 θ（適格規範群の合計有用性が θ 超で抽象規範へ統合）．
    pub synth_threshold: f64,
    /// 収束判定の安定ウィンドウ K（適格規範集合が K ラウンド不変なら停止）．
    pub convergence_window: usize,
    /// 採用率の創発しきい（time_to_emergence の判定; 既定 0.9）．
    pub emergence_threshold: f64,
    /// 規範同定の方式（canonical-norm-identity）．
    pub canonical_mode: CanonicalMode,
    /// 乱数シード（None の場合はランダム; socsim コア層のみ支配）．
    pub seed: Option<u64>,
    /// LLM レイヤ設定．
    pub llm: LlmSettings,
}

impl Default for Config {
    /// 論文 §3 に近い標準設定（N=10, 起業家3, WS, 48 ラウンド相当）．
    fn default() -> Self {
        Config {
            population: 10,
            entrepreneurs: 3,
            network: Network::WattsStrogatz,
            ws_k: 4,
            ws_beta: 0.1,
            er_p: 0.3,
            ba_m: 2,
            rounds: 48,
            synth_threshold: 200.0,
            convergence_window: 3,
            emergence_threshold: 0.9,
            canonical_mode: CanonicalMode::Deterministic,
            seed: Some(42),
            llm: LlmSettings::default(),
        }
    }
}

/// run 1 本の実験条件（runvault の `config.json` の `parameters` に入る）．
///
/// 旧 `config.json` から `command` と `output_dir` を落とした．どのサブコマンドの実行
/// なのかは `run.json` が持ち，run ディレクトリが出力先そのものなので，条件ではない．
///
/// 代わりに旧 `config.json` に無かった **結果を決める値** を 2 つ足した:
///
/// - `mock`: `--mock` は決定論的 scripted クライアントへ切り替える．LLM 応答が丸ごと
///   変わるので，これが条件に入っていないと `config_hash` が結果を決める値に盲目になる．
/// - `llm_cache_path`: プロンプト→応答キャッシュの置き場．**値そのものは結果を決めない**
///   （同じキャッシュ内容なら同じ応答になる）ので `hash_exclude` で `config_hash` から
///   外す．どこを見ていたかは記録に残す．
#[derive(Serialize)]
pub struct RunConfigJson {
    pub population: usize,
    pub entrepreneurs: usize,
    pub network: String,
    pub ws_k: usize,
    pub ws_beta: f64,
    pub er_p: f64,
    pub ba_m: usize,
    pub rounds: usize,
    pub synth_threshold: f64,
    pub convergence_window: usize,
    pub emergence_threshold: f64,
    pub canonical_mode: String,
    pub seed: Option<u64>,
    pub mock: bool,
    pub llm_temperature: f32,
    pub llm_seed: u64,
    pub llm_cache_path: Option<String>,
}

/// `config_hash` から外す JSON ポインタ（置き場でしかない値）．
pub const HASH_EXCLUDE: [&str; 1] = ["/llm_cache_path"];

/// `master_seed` の由来を指す JSON ポインタ．
pub const SEED_POINTERS: [&str; 1] = ["/seed"];

impl Config {
    /// runvault の `parameters` 用の表現を組み立てる．
    ///
    /// `mock` は Config が持たない（どのクライアントを組むかは呼び出し側の判断で，
    /// Config はモデルの条件だけを持つ）ので引数で受け取る．
    pub fn to_parameters(&self, mock: bool) -> RunConfigJson {
        RunConfigJson {
            population: self.population,
            entrepreneurs: self.entrepreneurs,
            network: self.network.label().to_string(),
            ws_k: self.ws_k,
            ws_beta: self.ws_beta,
            er_p: self.er_p,
            ba_m: self.ba_m,
            rounds: self.rounds,
            synth_threshold: self.synth_threshold,
            convergence_window: self.convergence_window,
            emergence_threshold: self.emergence_threshold,
            canonical_mode: self.canonical_mode.label().to_string(),
            seed: self.seed,
            mock,
            llm_temperature: self.llm.temperature,
            llm_seed: self.llm.seed,
            llm_cache_path: self.llm.cache_path.clone(),
        }
    }
}
