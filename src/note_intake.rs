//! Untrusted meeting-note intake.
//!
//! Note text is data only. This module never interprets a note as an instruction
//! and never changes task outcomes. The current Observation schema has no
//! `trusted` column; every observation created here is therefore treated as
//! `trusted=false` by contract at this boundary.

use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::business::{Category, Observation};
use crate::model::Candidate;
use crate::store::Store;

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_TITLE_BYTES: usize = 512;
const MAX_ACTIONS: usize = 50;

#[derive(Debug, Clone)]
pub struct NoteInput {
    pub category: Category,
    pub source_ref: Option<String>,
    pub meeting_id: Option<String>,
    pub title: String,
    pub body: String,
    pub observed_at: String,
}

#[derive(Debug, Serialize)]
pub struct NoteReceipt {
    pub observation: Observation,
    pub candidates: Vec<Candidate>,
    pub skipped: usize,
}

/// Save one untrusted note and propose only explicitly marked action lines.
///
/// The complete observation/candidate operation is atomic. An explicit source
/// reference is immutable: a later note with the same reference must have the
/// same category, meeting, title, and body or the whole operation is rejected.
pub fn ingest(store: &Store, input: &NoteInput) -> Result<NoteReceipt> {
    let title = normalize_note_title(&input.title);
    let body = input.body.trim().to_owned();
    validate_limits(&title, &body)?;
    if title.is_empty() {
        bail!("ノートのタイトルは必須です");
    }
    let meeting_id = normalize_optional(input.meeting_id.as_deref());
    let source_ref = match input.source_ref.as_deref() {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                bail!("source_ref は空にできません");
            }
            value.to_owned()
        }
        None => deterministic_source_ref(input.category, meeting_id.as_deref(), &title, &body),
    };
    let actions = extract_actions(&body)?;

    store.atomic(|| {
        reject_source_conflict(
            store,
            &source_ref,
            input.category,
            meeting_id.as_deref(),
            &title,
            &body,
        )?;

        let observation = store.add_observation(
            input.category,
            &source_ref,
            meeting_id.as_deref(),
            &title,
            Some(&body),
            &input.observed_at,
        )?;
        ensure_observation_matches(
            &observation,
            input.category,
            &source_ref,
            meeting_id.as_deref(),
            &title,
            &body,
        )?;

        let mut candidates = Vec::new();
        let mut skipped = 0;
        for action in actions {
            let action_key = digest(&action);
            match store.propose_action(&observation.id, &action_key, &action)? {
                Some(candidate) => candidates.push(candidate),
                None => skipped += 1,
            }
        }
        Ok(NoteReceipt {
            observation,
            candidates,
            skipped,
        })
    })
}

fn validate_limits(title: &str, body: &str) -> Result<()> {
    if title.len() > MAX_TITLE_BYTES {
        bail!("ノートのタイトルは{}バイト以内です", MAX_TITLE_BYTES);
    }
    if body.len() > MAX_BODY_BYTES {
        bail!("ノート本文は{}バイト以内です", MAX_BODY_BYTES);
    }
    Ok(())
}

fn normalize_note_title(value: &str) -> String {
    value.trim().to_owned()
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn deterministic_source_ref(
    category: Category,
    meeting_id: Option<&str>,
    title: &str,
    body: &str,
) -> String {
    let payload = [
        category.as_str(),
        meeting_id.unwrap_or_default(),
        title,
        body,
    ]
    .into_iter()
    .map(|part| format!("{}:{}", part.len(), part))
    .collect::<String>();
    digest(&payload)
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn extract_actions(body: &str) -> Result<Vec<String>> {
    let mut fence: Option<(u8, usize)> = None;
    let mut seen = HashSet::new();
    let mut actions = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some((character, width, rest)) = fence_marker(trimmed) {
            if let Some((open_character, open_width)) = fence {
                if character == open_character && width >= open_width && rest.trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = Some((character, width));
            }
            continue;
        }
        if fence.is_some() || trimmed.starts_with('>') {
            continue;
        }
        let candidate_line = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .unwrap_or(trimmed);
        let Some(rest) = ["ACTION:", "TODO:", "宿題:", "対応:"]
            .iter()
            .find_map(|marker| candidate_line.strip_prefix(marker))
        else {
            continue;
        };
        let action = rest.split_whitespace().collect::<Vec<_>>().join(" ");
        if action.is_empty() || !seen.insert(action.clone()) {
            continue;
        }
        if action.len() > MAX_TITLE_BYTES {
            bail!("ACTIONのタイトルは{}バイト以内です", MAX_TITLE_BYTES);
        }
        actions.push(action);
        if actions.len() > MAX_ACTIONS {
            bail!("ノートのACTIONは{}件以内です", MAX_ACTIONS);
        }
    }
    Ok(actions)
}

fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let bytes = line.as_bytes();
    let character = *bytes.first()?;
    if character != b'`' && character != b'~' {
        return None;
    }
    let width = bytes.iter().take_while(|byte| **byte == character).count();
    (width >= 3).then(|| (character, width, &line[width..]))
}

fn reject_source_conflict(
    store: &Store,
    source_ref: &str,
    category: Category,
    meeting_id: Option<&str>,
    title: &str,
    body: &str,
) -> Result<()> {
    let mut statement = store
        .conn
        .prepare("SELECT category,meeting_id,title,body FROM observations WHERE source_ref=?1")?;
    let rows = statement.query_map([source_ref], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (existing_category, existing_meeting_id, existing_title, existing_body) = row?;
        let expected_body = (!body.is_empty()).then(|| body.to_owned());
        if existing_category != category.as_str()
            || existing_meeting_id.as_deref() != meeting_id
            || existing_title != title
            || existing_body != expected_body
        {
            bail!("source_ref が既存ノートの内容と競合しています");
        }
    }
    Ok(())
}

fn ensure_observation_matches(
    observation: &Observation,
    category: Category,
    source_ref: &str,
    meeting_id: Option<&str>,
    title: &str,
    body: &str,
) -> Result<()> {
    let expected_body = (!body.is_empty()).then_some(body);
    if observation.category != category
        || observation.source_ref != source_ref
        || observation.meeting_id.as_deref() != meeting_id
        || observation.title != title
        || observation.body.as_deref() != expected_body
    {
        bail!("既存ノートの内容を変更できません");
    }
    Ok(())
}
