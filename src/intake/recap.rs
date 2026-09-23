//! Action lines from an M365 Copilot meeting recap, in English or Japanese.

pub fn actions(text: &str) -> Vec<String> {
    let mut in_actions = false;
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('#').trim();
        if line.is_empty() {
            continue;
        }
        if is_heading(line) {
            in_actions = is_action_heading(line);
            continue;
        }
        if !in_actions {
            continue;
        }
        let body = line
            .trim_start_matches(['-', '*', '•'])
            .trim()
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['.', ')'])
            .trim();
        if body.is_empty() {
            continue;
        }
        out.push(body.to_string());
    }
    out
}

fn is_heading(line: &str) -> bool {
    let lower = line.to_lowercase();
    is_action_heading(line)
        || lower.starts_with("discussion")
        || lower.starts_with("summary")
        || line.starts_with("議題")
        || line.starts_with("要約")
        || line.starts_with("概要")
}

fn is_action_heading(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("action item")
        || lower.contains("follow-up")
        || lower.contains("follow up")
        || lower.contains("next step")
        || line.contains("アクション")
        || line.contains("フォローアップ")
        || line.contains("次のステップ")
        || line.contains("宿題")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_english_and_japanese_action_sections() {
        let text = "Summary\nWe talked.\nAction items\n- send the estimate\n- confirm the date\nDiscussion\nnope\n";
        assert_eq!(
            actions(text),
            vec!["send the estimate".to_string(), "confirm the date".to_string()]
        );
        let ja = "概要\n話した\nアクション アイテム\n- 見積を送る\n宿題\n- 日程を確定する\n";
        assert_eq!(actions(ja), vec!["見積を送る".to_string(), "日程を確定する".to_string()]);
    }
}
