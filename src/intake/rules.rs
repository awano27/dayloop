use chrono::{DateTime, Duration, Local};

use crate::config::IntakeConfig;

use super::model::{CalendarItem, MailItem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub source_ref: String,
    pub reason: &'static str,
}

pub fn strip_subject_prefix(s: &str) -> &str {
    let mut t = s.trim();
    loop {
        let before = t;
        for p in ["RE:", "Re:", "re:", "FW:", "Fw:", "fw:", "FWD:", "Fwd:", "fwd:"] {
            if let Some(rest) = t.strip_prefix(p) {
                t = rest.trim_start();
            }
        }
        if t.len() == before.len() {
            break;
        }
    }
    t
}

fn has_keyword(text: &str, keywords: &[String]) -> bool {
    if text.is_empty() || keywords.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    keywords.iter().any(|k| {
        if k.is_empty() {
            false
        } else if k.is_ascii() {
            lower.contains(&k.to_lowercase())
        } else {
            text.contains(k)
        }
    })
}

fn sender_important(name: &str, senders: &[String]) -> bool {
    if senders.is_empty() {
        return false;
    }
    senders.iter().any(|s| !s.is_empty() && name.contains(s))
}

/// First matching mail rule wins.
pub fn mail_hit(mail: &MailItem, cfg: &IntakeConfig) -> Option<Hit> {
    let display = strip_subject_prefix(&mail.subject);
    let flagged = mail.flagged || mail.flag_request.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false);
    if flagged {
        return Some(Hit {
            title: format!("対応: {display}（{}）", mail.sender_name),
            source_ref: format!("outlook:mail:{}", mail.entry_id),
            reason: "flagged",
        });
    }
    let body = mail.body_excerpt.as_deref().unwrap_or("");
    if has_keyword(&mail.subject, &cfg.keywords) || has_keyword(body, &cfg.keywords) {
        return Some(Hit {
            title: format!("返信/対応: {display}（{}）", mail.sender_name),
            source_ref: format!("outlook:mail:{}", mail.entry_id),
            reason: "keyword",
        });
    }
    if mail.unread && sender_important(&mail.sender_name, &cfg.important_senders) {
        return Some(Hit {
            title: format!("確認: {display}（{}）", mail.sender_name),
            source_ref: format!("outlook:mail:{}", mail.entry_id),
            reason: "important_sender",
        });
    }
    None
}

/// Meeting-prep candidate. `now` is injected so tests can pin the 24h window.
pub fn event_prep_hit(cal: &CalendarItem, now: DateTime<Local>, cfg: &IntakeConfig) -> Option<Hit> {
    if !cfg.meeting_prep {
        return None;
    }
    if cal.start < now {
        return None;
    }
    if cal.start > now + Duration::hours(24) {
        return None;
    }
    if cfg.meeting_prep_only_required && !cal.is_organizer && !cal.response_required {
        return None;
    }
    let hhmm = cal.start.format("%H:%M");
    Some(Hit {
        title: format!("会議準備: {}（{hhmm}）", strip_subject_prefix(&cal.subject)),
        source_ref: format!("outlook:cal:{}:prep", cal.entry_id),
        reason: "meeting_prep",
    })
}
