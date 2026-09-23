//! Meeting notes. A generator emits statements, then a closed set decides
//! which of them are task candidates.

use anyhow::Result;

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinuteLine {
    Action(String),
    Info,
    Ignore,
}

pub fn classify(text: &str) -> Vec<MinuteLine> {
    text.lines().map(classify_line).collect()
}

pub fn classify_line(line: &str) -> MinuteLine {
    let line = line.trim();
    for (prefix, action) in [
        ("決定", true),
        ("TODO", true),
        ("アクション", true),
        ("共有", false),
        ("情報", false),
    ] {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start();
        let body = if let Some(rest) = rest.strip_prefix(':') {
            rest.trim()
        } else if let Some(rest) = rest.strip_prefix('：') {
            rest.trim()
        } else {
            continue;
        };
        if body.is_empty() {
            return MinuteLine::Ignore;
        }
        return if action {
            MinuteLine::Action(body.to_string())
        } else {
            MinuteLine::Info
        };
    }
    MinuteLine::Ignore
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Decision,
    Share,
}

pub fn read_minutes(text: &str) -> Vec<MinuteLine> {
    let mut section = Section::None;
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "決定事項" || line == "# 決定事項" {
            section = Section::Decision;
            continue;
        }
        if line == "共有事項" || line == "# 共有事項" {
            section = Section::Share;
            continue;
        }
        if line.starts_with('#') {
            section = Section::None;
            continue;
        }
        match classify_line(line) {
            MinuteLine::Ignore => {}
            other => {
                out.push(other);
                continue;
            }
        }
        let Some(body) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) else {
            out.push(MinuteLine::Ignore);
            continue;
        };
        let body = body.trim();
        if body.is_empty() {
            out.push(MinuteLine::Ignore);
            continue;
        }
        match section {
            Section::Decision => out.push(MinuteLine::Action(body.to_string())),
            Section::Share => out.push(MinuteLine::Info),
            Section::None => out.push(MinuteLine::Ignore),
        }
    }
    out
}

pub trait DecisionSource {
    fn statements(&mut self, text: &str) -> Vec<String>;
}

pub fn from_source(source: &mut dyn DecisionSource, text: &str) -> Vec<MinuteLine> {
    source
        .statements(text)
        .iter()
        .filter_map(|line| match classify_line(line) {
            MinuteLine::Ignore => None,
            other => Some(other),
        })
        .collect()
}

/// Lines the rules already classified stay as they are.
/// Each remaining line is one closed choice: 決定, 共有, or 無視.
pub fn judge_ignored(
    text: &str,
    decider: &mut dyn crate::jev::Decider,
    floor: f64,
) -> Vec<MinuteLine> {
    let choices = vec!["決定".to_string(), "共有".to_string(), "無視".to_string()];
    let ruled = read_minutes(text);
    let raws: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_heading(line))
        .collect();
    ruled
        .into_iter()
        .zip(raws)
        .map(|(line, raw)| match line {
            MinuteLine::Ignore => {
                let body = raw.trim_start_matches(['-', '*']).trim();
                if body.is_empty() {
                    MinuteLine::Ignore
                } else {
                    judge_one(decider, &choices, floor, body)
                }
            }
            other => other,
        })
        .collect()
}

fn judge_one(
    decider: &mut dyn crate::jev::Decider,
    choices: &[String],
    floor: f64,
    body: &str,
) -> MinuteLine {
    match decider.decide(body, choices) {
        crate::jev::JevOutcome::Answer { choice, confidence }
            if confidence >= floor && choice == "決定" =>
        {
            MinuteLine::Action(body.to_string())
        }
        crate::jev::JevOutcome::Answer { choice, confidence }
            if confidence >= floor && choice == "共有" =>
        {
            MinuteLine::Info
        }
        _ => MinuteLine::Ignore,
    }
}

fn is_heading(line: &str) -> bool {
    matches!(line, "決定事項" | "# 決定事項" | "共有事項" | "# 共有事項") || line.starts_with('#')
}

pub struct MinuteReport {
    pub actions: usize,
    pub info: usize,
    pub ignored: usize,
}

pub fn ingest(store: &Store, text: &str) -> Result<MinuteReport> {
    let cfg = crate::config::load();
    let lines = if cfg.minutes.generator.eq_ignore_ascii_case("llm") && jev_ready(&cfg) {
        let mut decider = crate::jev::HttpDecider {
            route: cfg.jev.route.clone(),
            timeout_ms: cfg.jev.timeout_ms,
        };
        let floor = cfg.jev.commit_confidence.unwrap_or(crate::jev::DEFAULT_FLOOR);
        judge_ignored(text, &mut decider, floor)
    } else {
        read_minutes(text)
    };
    let mut report = MinuteReport {
        actions: 0,
        info: 0,
        ignored: 0,
    };
    for (i, line) in lines.into_iter().enumerate() {
        match line {
            MinuteLine::Action(title) => {
                let source_ref = format!("minutes:{i}:{title}");
                if store
                    .add_candidate(&title, "meeting", Some(&source_ref))?
                    .is_some()
                {
                    report.actions += 1;
                }
            }
            MinuteLine::Info => report.info += 1,
            MinuteLine::Ignore => report.ignored += 1,
        }
    }
    Ok(report)
}

fn jev_ready(cfg: &crate::config::Config) -> bool {
    if !cfg.jev.mode.eq_ignore_ascii_case("on") || cfg.jev.route.trim().is_empty() {
        return false;
    }
    std::env::var("DAYLOOP_JEV_API_KEY")
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_split_action_info_and_noise() {
        let text = "決定: 見積を直す\n共有: 来週は休会\nこんにちは\nTODO：会場を押さえる\n";
        let lines = classify(text);
        assert_eq!(
            lines,
            vec![
                MinuteLine::Action("見積を直す".into()),
                MinuteLine::Info,
                MinuteLine::Ignore,
                MinuteLine::Action("会場を押さえる".into()),
            ]
        );
    }

    struct Script {
        lines: Vec<String>,
    }

    impl DecisionSource for Script {
        fn statements(&mut self, _text: &str) -> Vec<String> {
            self.lines.clone()
        }
    }

    #[test]
    fn an_unmarked_line_becomes_an_action_only_when_jev_says_so() {
        struct Pick;
        impl crate::jev::Decider for Pick {
            fn decide(&mut self, state: &str, _choices: &[String]) -> crate::jev::JevOutcome {
                if state == "見積を直す" {
                    crate::jev::JevOutcome::Answer { choice: "決定".into(), confidence: 0.9 }
                } else {
                    crate::jev::JevOutcome::Answer { choice: "無視".into(), confidence: 0.9 }
                }
            }
        }
        let lines = judge_ignored("見積を直す\n雑談\n", &mut Pick, 0.5);
        assert_eq!(
            lines,
            vec![MinuteLine::Action("見積を直す".into()), MinuteLine::Ignore]
        );
    }

    #[test]
    fn model_lines_outside_the_closed_set_are_dropped() {
        let mut source = Script {
            lines: vec!["決定: 見積を直す".into(), "来週よろしく".into()],
        };
        let lines = from_source(&mut source, "本文");
        assert_eq!(lines, vec![MinuteLine::Action("見積を直す".into())]);
    }
}
