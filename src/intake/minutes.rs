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

/// Model output is accepted only through [`from_source`]. An empty route yields no lines.
pub struct LlmMinutes {
    pub route: String,
}

impl DecisionSource for LlmMinutes {
    fn statements(&mut self, _text: &str) -> Vec<String> {
        // A non-empty route still returns nothing here. Lines would have to pass
        // `from_source` before they could become candidates, and no response shape
        // is accepted yet.
        let _ = &self.route;
        Vec::new()
    }
}

pub struct MinuteReport {
    pub actions: usize,
    pub info: usize,
    pub ignored: usize,
}

pub fn ingest(store: &Store, text: &str) -> Result<MinuteReport> {
    let cfg = crate::config::load();
    let lines = if cfg.minutes.generator.eq_ignore_ascii_case("llm") {
        let mut source = LlmMinutes {
            route: cfg.jev.route.clone(),
        };
        from_source(&mut source, text)
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
    fn model_lines_outside_the_closed_set_are_dropped() {
        let mut source = Script {
            lines: vec!["決定: 見積を直す".into(), "来週よろしく".into()],
        };
        let lines = from_source(&mut source, "本文");
        assert_eq!(lines, vec![MinuteLine::Action("見積を直す".into())]);
    }
}
