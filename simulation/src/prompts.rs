//! LLM プロンプト生成（CRSEC の規範ライフサイクル操作）．
//!
//! 論文 Section 2 の LLM 操作（CreateNorm / DetectConflict & IdentifyNormativeInformation /
//! GenerateNormativePlan / EvaluateNorm）に対応するプロンプトを組み立てる．プロンプトは
//! キャッシュキー（`hash(prompt + model)`）の素材になるため，同一状態からは同一プロンプト
//! = 同一応答（擬似決定論）になるよう決定論的に構築する．
//!
//! # 出力契約（パーサと対）
//!
//! プロンプトは行ベースの `KEY: value` 形式の応答を要求する（[`crate::parse`] が
//! 寛容にパースする）．LLM 呼び出しを抑えるため，伝播の «衝突検出 + 会話判断 +
//! 規範識別» を 1 回の構造化呼び出しに，評価の «4 サニティ検査» を 1 回の構造化
//! 呼び出しに統合する（プロンプト先頭の番号付き設問で同時に問う）．

use crate::norm::PersonalNorm;
use crate::world::AgentProfile;

/// 適格規範集合をプロンプト用の短い箇条書きに畳む．
pub fn norm_digest(norms: &[&PersonalNorm], max_items: usize) -> String {
    if norms.is_empty() {
        return "(none yet)".to_string();
    }
    let n = norms.len().min(max_items);
    norms[..n]
        .iter()
        .map(|x| {
            format!(
                "- [{}] {} (utility {})",
                x.alpha.label(),
                x.content,
                x.utility
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 選好をプロンプト用の短い文字列に畳む．
fn pref_digest(prefs: &[String]) -> String {
    if prefs.is_empty() {
        "(no strong preferences)".to_string()
    } else {
        prefs.join("; ")
    }
}

/// CreateNorm: 規範起業家が初期個人規範を 1 件生成する（DB が空のとき）．
///
/// 出力: `CONTENT: <一文の規範>` / `TYPE: injunctive|descriptive` / `UTILITY: <1..100>`．
pub fn create_norm_prompt(profile: &AgentProfile, scene: &str) -> String {
    format!(
        "You are a member of a small community. Persona: {desc}\n\
         Your preferences: {prefs}\n\
         Shared scene/context: {scene}\n\n\
         As a norm entrepreneur, propose ONE social norm for this community that reflects \
         your values and would reduce conflict.\n\
         Reply in EXACTLY this format, nothing else:\n\
         CONTENT: <a single short rule, e.g. 'no smoking indoors'>\n\
         TYPE: <injunctive or descriptive>\n\
         UTILITY: <an integer from 1 to 100 for how useful this norm is>",
        desc = profile.description,
        prefs = pref_digest(&profile.preferences),
        scene = scene,
    )
}

/// GenerateNormativePlan (Compliance): 適格規範に沿った行動を 1 つ生成する．
///
/// 遵守判定に使う出力: `COMPLY: yes|no` / `ACTION: <短い行動>`．
pub fn compliance_prompt(
    profile: &AgentProfile,
    qualified: &[&PersonalNorm],
    scene: &str,
) -> String {
    format!(
        "You are a community member. Persona: {desc}\n\
         The social norms you currently hold (qualified):\n{norms}\n\
         Current scene: {scene}\n\n\
         Decide on your next action. You should follow your qualified norms.\n\
         Reply in EXACTLY this format, nothing else:\n\
         COMPLY: <yes if your action follows the norms, no otherwise>\n\
         ACTION: <a single short sentence describing what you do>",
        desc = profile.description,
        norms = norm_digest(qualified, 6),
        scene = scene,
    )
}

/// Spreading（統合呼び出し）: 送信者→受信者の会話 / 観察で，衝突検出・会話判断・
/// 規範識別を **1 回** で問う（LLM 呼び出し削減）．
///
/// 出力:
/// `CONFLICT: yes|no`（送信者と受信者の選好が衝突するか）/
/// `TALK: yes|no`（会話を始めるか; 観察時は識別のみで talk は無関係）/
/// `NORM: <識別した規範文 or none>` / `TYPE: injunctive|descriptive` / `UTILITY: <1..100>`．
pub fn spreading_prompt(
    sender: &AgentProfile,
    receiver: &AgentProfile,
    sender_qualified: &[&PersonalNorm],
    is_observation: bool,
    scene: &str,
) -> String {
    let mode = if is_observation {
        "You are OBSERVING another member's behaviour (no direct conversation)."
    } else {
        "You are about to have a CONVERSATION with another member."
    };
    format!(
        "{mode}\n\
         Your persona (sender): {sdesc}; preferences: {sprefs}\n\
         The other member (receiver): {rdesc}; preferences: {rprefs}\n\
         Norms you (sender) hold and may convey:\n{snorms}\n\
         Current scene: {scene}\n\n\
         Analyse the interaction. Reply in EXACTLY this format, nothing else:\n\
         CONFLICT: <yes if your values/behaviour clash with the other member, no otherwise>\n\
         TALK: <yes if you decide to start a conversation about it, no otherwise>\n\
         NORM: <the single normative rule the receiver would identify, or 'none'>\n\
         TYPE: <injunctive or descriptive>\n\
         UTILITY: <an integer from 1 to 100>",
        mode = mode,
        sdesc = sender.description,
        sprefs = pref_digest(&sender.preferences),
        rdesc = receiver.description,
        rprefs = pref_digest(&receiver.preferences),
        snorms = norm_digest(sender_qualified, 4),
        scene = scene,
    )
}

/// EvaluateNorm（統合呼び出し）: 即時評価の 4 サニティ検査（整合性・重複・型・衝突）を
/// **1 回** で問い，昇格可否を返す（LLM 呼び出し削減）．
///
/// 出力: `CONSISTENT: yes|no` / `DUPLICATE: yes|no` / `TYPE_OK: yes|no` /
/// `CONFLICTS: yes|no` / `PROMOTE: yes|no`．`PROMOTE` を最終判断として使い，欠落時は
/// 個別フラグから合議する（[`crate::parse`]）．
pub fn evaluation_prompt(
    profile: &AgentProfile,
    candidate: &PersonalNorm,
    existing_qualified: &[&PersonalNorm],
) -> String {
    format!(
        "You are a community member evaluating a newly identified candidate norm before \
         internalising it. Persona: {desc}\n\
         Candidate norm: \"{cand}\" (type {ctype}, utility {cutil})\n\
         Norms you already hold (qualified):\n{norms}\n\n\
         Run four sanity checks and decide whether to adopt the candidate as a qualified norm.\n\
         Reply in EXACTLY this format, nothing else:\n\
         CONSISTENT: <yes if it is internally consistent and plausible>\n\
         DUPLICATE: <yes if it duplicates a norm you already hold>\n\
         TYPE_OK: <yes if its declared type is appropriate>\n\
         CONFLICTS: <yes if it conflicts with your existing qualified norms>\n\
         PROMOTE: <yes to adopt it as a qualified norm, no to discard>",
        desc = profile.description,
        cand = candidate.content,
        ctype = candidate.alpha.label(),
        cutil = candidate.utility,
        norms = norm_digest(existing_qualified, 6),
    )
}

/// 規範同定（`--canonical-mode llm`）: «二つの規範表現は同じ社会規範か» を問う．
///
/// LLM ベースの canonical-norm-identity に使う．語順・冠詞・主語の揺れだけでなく，
/// 語彙が重ならないパラフレーズ（"no smoking" ↔ "refrain from cigarettes"）も同一規範
/// として束ねられる点が決定論的 keyword-set 正規化との違い．出力契約は単純な
/// `SAME: yes|no`（[`crate::parse::same_norm`] がパースする）．
pub fn same_norm_prompt(a: &str, b: &str) -> String {
    format!(
        "You compare two short statements of social norms and decide whether they express \
         the SAME underlying norm (same prescribed/observed behaviour), ignoring wording, \
         word order, articles, and subject.\n\
         Norm A: \"{a}\"\n\
         Norm B: \"{b}\"\n\n\
         Reply in EXACTLY this format, nothing else:\n\
         SAME: <yes if they are the same norm, no otherwise>",
        a = a,
        b = b,
    )
}
