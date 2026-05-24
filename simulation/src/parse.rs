//! LLM 応答の寛容なパース（`KEY: value` 行形式）．
//!
//! プロンプト（[`crate::prompts`]）が要求する行ベースの構造化出力を，雑な LLM 応答
//! からもベストエフォートで抽出する．キャッシュ済み応答にも生応答にも同じパーサを
//! 適用するため，二層決定論の上層（擬似決定論）に閉じる．

use crate::norm::{NormType, PersonalNorm};

/// `KEY: value` 行から大文字小文字を無視して `key` の値を取り出す．
///
/// 同名キーが複数あれば最初の出現を返す．`key` 行が無ければ `None`．
pub fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let key_l = key.to_ascii_lowercase();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_ascii_lowercase() == key_l {
                return Some(v.trim());
            }
        }
    }
    None
}

/// `yes` / `true` / `1` を真，それ以外（含 None）を偽とみなす（先頭語のみ判定）．
pub fn yes(text: &str, key: &str) -> bool {
    match field(text, key) {
        Some(v) => {
            let w = v
                .split(|c: char| !c.is_ascii_alphanumeric())
                .find(|t| !t.is_empty())
                .unwrap_or("")
                .to_ascii_lowercase();
            matches!(
                w.as_str(),
                "yes" | "true" | "y" | "1" | "comply" | "promote"
            )
        }
        None => false,
    }
}

/// `UTILITY:` 等から最初の整数を抽出し `[1, 100]` にクランプする（既定 50）．
pub fn utility(text: &str, key: &str) -> u8 {
    let raw = field(text, key).unwrap_or("");
    let digits: String = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse::<u32>()
        .map(|v| v.clamp(1, 100) as u8)
        .unwrap_or(50)
}

/// 候補規範を `NORM` / `TYPE` / `UTILITY` 行から組み立てる（未適格で返す）．
///
/// `NORM` が空・`none`・欠落なら `None`（識別なし）．
pub fn identified_norm(text: &str) -> Option<PersonalNorm> {
    let content = field(text, "NORM").or_else(|| field(text, "CONTENT"))?;
    let content = content.trim().trim_matches('"');
    if content.is_empty() || content.eq_ignore_ascii_case("none") {
        return None;
    }
    let alpha = field(text, "TYPE")
        .map(NormType::parse_loose)
        .unwrap_or(NormType::Descriptive);
    let util = utility(text, "UTILITY");
    Some(PersonalNorm::identified(content, util, alpha))
}

/// 創出された規範を `CONTENT` / `TYPE` / `UTILITY` 行から組み立てる（適格で返す）．
pub fn created_norm(text: &str) -> Option<PersonalNorm> {
    let content = field(text, "CONTENT").or_else(|| field(text, "NORM"))?;
    let content = content.trim().trim_matches('"');
    if content.is_empty() || content.eq_ignore_ascii_case("none") {
        return None;
    }
    let alpha = field(text, "TYPE")
        .map(NormType::parse_loose)
        .unwrap_or(NormType::Injunctive);
    let util = utility(text, "UTILITY");
    Some(PersonalNorm::created(content, util, alpha))
}

/// 評価の昇格判断: `PROMOTE` があればそれを，無ければ 4 検査から合議する．
///
/// 合議規則: 整合的 ∧ 型OK ∧ ¬重複 ∧ ¬衝突 のとき昇格．
pub fn promote_decision(text: &str) -> bool {
    if field(text, "PROMOTE").is_some() {
        return yes(text, "PROMOTE");
    }
    let consistent = yes(text, "CONSISTENT");
    let type_ok = yes(text, "TYPE_OK");
    let duplicate = yes(text, "DUPLICATE");
    let conflicts = yes(text, "CONFLICTS");
    consistent && type_ok && !duplicate && !conflicts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_is_case_insensitive() {
        let t = "Conflict: yes\nNORM: no smoking";
        assert_eq!(field(t, "conflict"), Some("yes"));
        assert_eq!(field(t, "NORM"), Some("no smoking"));
    }

    #[test]
    fn yes_parses_first_word() {
        assert!(yes("TALK: yes, definitely", "TALK"));
        assert!(!yes("TALK: no way", "TALK"));
        assert!(!yes("", "TALK"));
    }

    #[test]
    fn utility_clamps_and_defaults() {
        assert_eq!(utility("UTILITY: 80", "UTILITY"), 80);
        assert_eq!(utility("UTILITY: 0", "UTILITY"), 1);
        assert_eq!(utility("UTILITY: 999", "UTILITY"), 100);
        assert_eq!(utility("UTILITY: n/a", "UTILITY"), 50);
    }

    #[test]
    fn identified_norm_none_when_absent() {
        assert!(identified_norm("NORM: none\nTYPE: inj").is_none());
        let n = identified_norm("NORM: no smoking indoors\nTYPE: injunctive\nUTILITY: 70").unwrap();
        assert_eq!(n.content, "no smoking indoors");
        assert_eq!(n.alpha, NormType::Injunctive);
        assert!(!n.qualified());
    }

    #[test]
    fn promote_decision_consensus_and_explicit() {
        assert!(promote_decision("PROMOTE: yes"));
        assert!(!promote_decision("PROMOTE: no"));
        assert!(promote_decision(
            "CONSISTENT: yes\nTYPE_OK: yes\nDUPLICATE: no\nCONFLICTS: no"
        ));
        assert!(!promote_decision(
            "CONSISTENT: yes\nTYPE_OK: yes\nDUPLICATE: yes\nCONFLICTS: no"
        ));
    }
}
