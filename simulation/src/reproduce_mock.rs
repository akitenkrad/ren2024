//! オフライン（LLM 不要）再現用のスクリプト化クライアント．
//!
//! 論文（Ren et al. 2024, CRSEC）の **見出し的知見** を，ライブ LLM 無しで構造的に
//! 再現するための決定論的 mock を提供する．`reproduce` サブコマンドと `run --mock`，
//! および各種テストがこの mock を共用する．
//!
//! 再現する定性的挙動（論文 Section 3 / Fig. 2 / Fact 7）:
//! - **社会規範の創発（emergence）**: 規範起業家が創出した規範が会話・観察を介して
//!   集団へ伝播し，評価（サニティ検査）で昇格して **適格** 規範が共有される．採用率
//!   `adoption_rate` が高い水準（≥ 0.9）へ上昇する．
//! - **規範の統合（consolidation）**: 相異なる canonical 規範数 `n_distinct_norms` が
//!   初期に立ち上がってから 1〜少数へ縮約する（共有規範への収斂）．
//! - **衝突の rise-then-fall**: 規範が普及するまでは選好衝突が起きるが，共有が進むと
//!   `n_conflicts` が減衰する（mock は «相手がまだ規範を持たないか» で近似）．
//! - **Fact 7（injunctive が descriptive より先に創発）**: 命令的規範（"should not …"）が
//!   先に創出・伝播し，記述的規範（"people tend to …"）は遅れて立ち上がる．mock は
//!   ラウンド進行を «送信者がいくつ規範を保持しているか» で近似し，序盤は injunctive
//!   のみ，規範が定着した後に descriptive も識別させることで型別の創発順序を作る．
//!
//! この mock は ground-truth LLM ではなく，論文の定性的結論を再現するための «規範
//! ライフサイクルの戯画» である．プロンプト文字列から «どの操作か»・«送信者が既に
//! 規範を持つか» を読み取って応答を決める．ライブ llama3.2 ではこの戯画ではなく実
//! モデルの応答を用いる（cache 経由）．

use socsim_llm::mock::ScriptedClient;
use socsim_llm::PromptCache;

use crate::llm::{wrap_client, CrsecClient};

/// 各操作プロンプトを判別するためのマーカ（prompts.rs と一致させる）．
const CREATE_MARK: &str = "propose ONE social norm";
const COMPLY_MARK: &str = "Decide on your next action";
const SPREAD_MARK: &str = "Analyse the interaction";
const EVAL_MARK: &str = "Run four sanity checks";
const SAME_MARK: &str = "express the SAME underlying norm";
/// 送信者がまだ規範を持たない（伝播の初期段階）ことを示すマーカ（prompts.rs）．
const NO_NORMS_MARK: &str = "(none yet)";
/// 観察（会話ではない）相互作用を示すマーカ（prompts.rs の spreading_prompt）．
const OBSERVE_MARK: &str = "OBSERVING another member";

/// 中核となる命令的規範（起業家が創出する; 先に創発）．
pub const INJUNCTIVE_NORM: &str = "no smoking indoors";
/// 記述的規範（規範定着後に識別される; 遅れて創発）．
pub const DESCRIPTIVE_NORM: &str = "people greet newcomers warmly";

/// 伝播プロンプトに対する応答を組み立てる（mock の中核ロジック）．
///
/// 命令的規範を **先に**，記述的規範を **後に** 創発させて Fact 7 を再現する:
/// - **会話（conversation）** は常に命令的規範を伝える（起業家由来の規範が会話を
///   通じて積極的に広がり，先に集団へ行き渡る）．送信者がまだ規範を持たない序盤
///   （`(none yet)`）は選好衝突あり，規範が定着すると衝突なし（rise-then-fall）．
/// - **観察（observation）** は，送信者が既に命令的規範を内在化している場合にのみ
///   記述的規範を識別させる（「皆がこうしている」を観察するには規範の定着が前提）．
///   よって記述的規範は命令的規範より **遅れて** 立ち上がる．
fn spreading_reply(prompt: &str) -> String {
    let observing = prompt.contains(OBSERVE_MARK);
    let sender_has_norm = !prompt.contains(NO_NORMS_MARK);

    if observing && sender_has_norm {
        // 規範定着後の観察 → 記述的規範を後発で識別（衝突なし）．記述的規範は «送信者が
        // 命令的規範を内在化している» ことを前提に立ち上がるため，命令的より遅れる．
        format!("CONFLICT: no\nTALK: yes\nNORM: {DESCRIPTIVE_NORM}\nTYPE: descriptive\nUTILITY: 60")
    } else {
        // 会話・初期観察はいずれも命令的規範を伝える（先行・高速に集団へ行き渡る）．
        // 命令的規範は会話と観察の両経路で広がるため，観察 1 経路のみの記述的規範より
        // 確実に先に飽和する（Fact 7 の順序を頑健に保証する）．衝突は «送信者が規範を
        // 持つか» で rise-then-fall．
        let conflict = if sender_has_norm { "no" } else { "yes" };
        format!(
            "CONFLICT: {conflict}\nTALK: yes\nNORM: {INJUNCTIVE_NORM}\nTYPE: injunctive\nUTILITY: 85"
        )
    }
}

/// 規範同定（`--canonical-mode llm`）判定への応答を組み立てる．
///
/// 二つの記述がともに «喫煙» 系，あるいはともに «挨拶» 系なら同一規範（SAME: yes）．
/// それ以外は別規範（SAME: no）．語彙が重ならないパラフレーズも束ねうる戯画．
fn same_norm_reply(prompt: &str) -> String {
    let smoke = prompt.matches("smok").count() >= 2
        || (prompt.contains("smok") && prompt.contains("cigarette"));
    let greet = prompt.matches("greet").count() >= 2
        || (prompt.contains("greet") && prompt.contains("welcome"));
    if smoke || greet {
        "SAME: yes".to_string()
    } else {
        "SAME: no".to_string()
    }
}

/// 再現用の決定論的スクリプトクライアントを構築する（in-memory cache）．
///
/// CRSEC の各 LLM 操作（創出・遵守・伝播・評価・規範同定）に対して，論文の定性的知見を
/// 再現する固定応答を返す．`reproduce` / `run --mock` / テストが共用する．
pub fn build_reproduce_client() -> CrsecClient {
    let backend = ScriptedClient::new("mock-crsec", |prompt: &str| {
        if prompt.contains(CREATE_MARK) {
            // 起業家は命令的規範を創出（injunctive が先行）．
            format!("CONTENT: {INJUNCTIVE_NORM}\nTYPE: injunctive\nUTILITY: 85")
        } else if prompt.contains(COMPLY_MARK) {
            "COMPLY: yes\nACTION: I refrain from smoking indoors.".to_string()
        } else if prompt.contains(SPREAD_MARK) {
            spreading_reply(prompt)
        } else if prompt.contains(EVAL_MARK) {
            // 4 サニティ検査を通過させ昇格（適格化）．
            "CONSISTENT: yes\nDUPLICATE: no\nTYPE_OK: yes\nCONFLICTS: no\nPROMOTE: yes".to_string()
        } else if prompt.contains(SAME_MARK) {
            same_norm_reply(prompt)
        } else {
            "none".to_string()
        }
    });
    wrap_client(backend, PromptCache::in_memory())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_always_spreads_injunctive() {
        // 会話（OBSERVE_MARK なし）は常に命令的規範を伝える．序盤は衝突あり．
        let early = spreading_reply("CONVERSATION ...\n(none yet)\n... Analyse the interaction");
        assert!(early.contains("injunctive"));
        assert!(early.contains(INJUNCTIVE_NORM));
        assert!(early.contains("CONFLICT: yes"));
        // 規範定着後の会話 → 命令的規範，衝突なし．
        let late = spreading_reply(
            "CONVERSATION ...\n- [inj] no smoking indoors (utility 85)\n... Analyse",
        );
        assert!(late.contains("injunctive"));
        assert!(late.contains("CONFLICT: no"));
    }

    #[test]
    fn observation_spreads_descriptive_only_after_norm_internalised() {
        // 規範を持つ送信者の観察 → 記述的規範を後発で識別．
        let after = spreading_reply(&format!(
            "{OBSERVE_MARK} ...\n- [inj] no smoking indoors (utility 85)\n... Analyse"
        ));
        assert!(after.contains("descriptive"));
        assert!(after.contains(DESCRIPTIVE_NORM));
        // まだ規範を持たない観察者は記述的規範を識別できず，命令的規範が先行して広がる．
        let before = spreading_reply(&format!("{OBSERVE_MARK} ...\n(none yet)\n... Analyse"));
        assert!(before.contains("injunctive"));
        assert!(before.contains(INJUNCTIVE_NORM));
    }

    #[test]
    fn same_norm_reply_buckets_synonyms() {
        // 喫煙系どうし → SAME: yes．
        let p = format!(
            "{SAME_MARK}\nNorm A: \"no smoking indoors\"\nNorm B: \"please refrain from cigarettes inside\""
        );
        assert!(same_norm_reply(&p).contains("yes"));
        // 無関係 → SAME: no．
        let q = format!(
            "{SAME_MARK}\nNorm A: \"no smoking indoors\"\nNorm B: \"keep the library quiet\""
        );
        assert!(same_norm_reply(&q).contains("no"));
    }
}
