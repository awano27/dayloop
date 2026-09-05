//! Foreground scheduler for evaluating daily questions and delivering reminders.
//! It never confirms plans, closes a day, discards work, or writes retro notes.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveTime, Weekday};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::engine;
use crate::notify::{self, Sent};
use crate::paths;
use crate::rituals::Outcome;
use crate::store::Store;
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Plan,
    Check,
    Close,
    Retro,
}

impl Phase {
    const ALL: [Self; 4] = [Self::Plan, Self::Check, Self::Close, Self::Retro];

    fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Check => "check",
            Self::Close => "close",
            Self::Retro => "retro",
        }
    }

    fn label_ja(self) -> &'static str {
        match self {
            Self::Plan => "計画",
            Self::Check => "途中確認",
            Self::Close => "クローズ",
            Self::Retro => "振り返り",
        }
    }

    fn cli_hint(self) -> &'static str {
        match self {
            Self::Plan => "dayloop plan",
            Self::Check => "dayloop check",
            Self::Close => "dayloop close",
            Self::Retro => "dayloop retro",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PhaseState {
    pub evaluated_on: Option<String>,
    pub evaluated_slot: Option<String>,
    pub pending_on: Option<String>,
    pub pending_count: Option<usize>,
    pub delivered_slot: Option<String>,
    pub attempted_slot: Option<String>,
    pub disabled_slot: Option<String>,
    pub error_on: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServeState {
    #[serde(default)]
    pub phases: BTreeMap<String, PhaseState>,
    /// Outstanding work is keyed by its original `YYYY-MM-DD/phase`, so a new
    /// day can never overwrite a prior unanswered question.
    #[serde(default)]
    pub pending_history: BTreeMap<String, PhaseState>,
    // Schema-1 values are retained only long enough to migrate them into an
    // explicit pending state, never treated as a completed business action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    close: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retro: Option<String>,
}

impl ServeState {
    pub fn load_at(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("serve state を読めません: {}", path.display()))
            }
        };
        let mut state: Self = serde_json::from_str(&text)
            .with_context(|| format!("serve state の形式が不正です: {}", path.display()))?;
        state.migrate_legacy();
        Ok(state)
    }

    pub fn phase_state(&self, phase: Phase) -> Option<&PhaseState> {
        self.phases.get(phase.as_str())
    }

    fn phase_state_mut(&mut self, phase: Phase) -> &mut PhaseState {
        self.phases.entry(phase.as_str().to_string()).or_default()
    }

    fn migrate_outstanding(&mut self, current_date: &str) {
        let previous = std::mem::take(&mut self.phases);
        for (name, phase_state) in previous {
            let origin = phase_state
                .pending_on
                .as_deref()
                .or(phase_state.error_on.as_deref());
            if let Some(origin) = origin.filter(|date| *date != current_date) {
                let key = format!("{origin}/{name}");
                self.pending_history.entry(key).or_insert(phase_state);
            } else {
                self.phases.insert(name, phase_state);
            }
        }
    }

    fn migrate_legacy(&mut self) {
        for (phase, legacy) in [
            (Phase::Plan, self.plan.take()),
            (Phase::Check, self.check.take()),
            (Phase::Close, self.close.take()),
            (Phase::Retro, self.retro.take()),
        ] {
            if let Some(date) = legacy {
                let phase_state = self.phase_state_mut(phase);
                if phase_state.evaluated_on.is_none() {
                    phase_state.evaluated_on = Some(date.clone());
                    phase_state.pending_on = Some(date);
                    phase_state.pending_count = Some(1);
                }
            }
        }
    }

    fn save_at(&self, path: &Path) -> Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "serve state のフォルダを作成できません: {}",
                parent.display()
            )
        })?;
        let temp = parent.join(format!(".serve-state-{}.tmp", ulid::Ulid::new()));
        let write = || -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(serde_json::to_vec_pretty(self)?.as_slice())?;
            file.sync_all()?;
            replace_file(&temp, path)
        };
        match write() {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                Err(error).with_context(|| {
                    format!("serve state を原子的に保存できません: {}", path.display())
                })
            }
        }
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, target: &Path) -> Result<()> {
    std::fs::rename(temp, target)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(temp: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let to_wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let temp_wide = to_wide(temp);
    let target_wide = to_wide(target);
    if unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        anyhow::bail!("MoveFileExW was rejected");
    }
    Ok(())
}

struct ServeLock {
    _file: File,
}

impl ServeLock {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock_exclusive()
            .map_err(|_| anyhow::anyhow!("dayloop serve は既に実行中です"))?;
        Ok(Self { _file: file })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Slot {
    Morning,
    Noon,
    Evening,
}

impl Slot {
    fn as_str(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Noon => "noon",
            Self::Evening => "evening",
        }
    }
}

#[derive(Debug, Default)]
pub struct ServeTick {
    pub evaluated: Vec<Phase>,
    pub notification_sent: bool,
    pub notification_disabled: bool,
    pub delivery_error: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingItem {
    phase: Phase,
    origin_date: String,
    history_key: Option<String>,
}

pub fn run(quiet: bool) -> Result<()> {
    if quiet {
        detach_console();
    }
    let data_dir = paths::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let _lock = ServeLock::acquire(&data_dir.join("serve.lock"))?;
    let state_path = paths::serve_state_path();
    log_event("serve started");
    loop {
        let cfg = match config::load_at(&paths::config_path()) {
            Ok(cfg) => cfg,
            Err(error) => {
                log_event(&format!("config error; evaluation skipped: {error}"));
                sleep_until_next_minute();
                continue;
            }
        };
        let now = Local::now();
        match run_once_at(
            &cfg,
            &state_path,
            now,
            |phase, date| evaluate_snapshot(phase, date, &cfg),
            |message| notify::send(&cfg.notify.method, message),
        ) {
            Ok(tick) => {
                for phase in tick.evaluated {
                    log_event(&format!("{} evaluated", phase.as_str()));
                }
                if let Some(error) = tick.delivery_error {
                    log_event(&format!("notification error: {error}"));
                }
            }
            Err(error) => log_event(&format!("serve tick failed: {error}")),
        }
        sleep_until_next_minute();
    }
}

/// One deterministic scheduler tick. Callers supply evaluation and delivery so
/// tests never open a real ledger, client, or notification target.
pub fn run_once_at<E, N>(
    cfg: &Config,
    state_path: &Path,
    now: DateTime<Local>,
    mut evaluate: E,
    mut deliver: N,
) -> Result<ServeTick>
where
    E: FnMut(Phase, &str) -> Result<Outcome>,
    N: FnMut(&str) -> Result<Sent>,
{
    let Some(slot) = current_slot(cfg, now) else {
        // Before the first configured slot, do not create or rewrite state.
        return Ok(ServeTick::default());
    };
    let date = now.format("%Y-%m-%d").to_string();
    let mut state = ServeState::load_at(state_path)?;
    state.migrate_outstanding(&date);
    let mut tick = ServeTick::default();
    let slot_key = format!("{}:{}", date, slot.as_str());
    for phase in due_phases_for_slot(cfg, &state, now, &slot_key) {
        tick.evaluated.push(phase);
        let phase_state = state.phase_state_mut(phase);
        phase_state.evaluated_on = Some(date.clone());
        phase_state.evaluated_slot = Some(slot_key.clone());
        phase_state.error_on = None;
        phase_state.error = None;
        match catch_unwind(AssertUnwindSafe(|| evaluate(phase, &date))) {
            Ok(Ok(Outcome::Done)) => {
                phase_state.pending_on = None;
                phase_state.pending_count = None;
            }
            Ok(Ok(Outcome::Pending(count))) => {
                phase_state.pending_on = Some(date.clone());
                phase_state.pending_count = Some(count);
            }
            Ok(Err(error)) => {
                phase_state.pending_on = None;
                phase_state.pending_count = None;
                phase_state.error_on = Some(date.clone());
                phase_state.error = Some(error.to_string());
            }
            Err(_) => {
                phase_state.pending_on = None;
                phase_state.pending_count = None;
                phase_state.error_on = Some(date.clone());
                phase_state.error = Some("panic during evaluation".into());
            }
        }
    }

    let history_keys = state.pending_history.keys().cloned().collect::<Vec<_>>();
    for key in history_keys {
        let Some((origin_date, phase)) = parse_history_key(&key) else {
            continue;
        };
        let needs_refresh = state
            .pending_history
            .get(&key)
            .is_some_and(|item| item.evaluated_slot.as_deref() != Some(slot_key.as_str()));
        if !needs_refresh {
            continue;
        }
        tick.evaluated.push(phase);
        let outcome = catch_unwind(AssertUnwindSafe(|| evaluate(phase, &origin_date)));
        let item = state
            .pending_history
            .get_mut(&key)
            .expect("history key was collected from the map");
        item.evaluated_on = Some(origin_date.clone());
        item.evaluated_slot = Some(slot_key.clone());
        item.error_on = None;
        item.error = None;
        match outcome {
            Ok(Ok(Outcome::Done)) => {
                state.pending_history.remove(&key);
            }
            Ok(Ok(Outcome::Pending(count))) => {
                item.pending_on = Some(origin_date);
                item.pending_count = Some(count);
            }
            Ok(Err(error)) => {
                item.pending_on = None;
                item.pending_count = None;
                item.error_on = Some(origin_date);
                item.error = Some(error.to_string());
            }
            Err(_) => {
                item.pending_on = None;
                item.pending_count = None;
                item.error_on = Some(origin_date);
                item.error = Some("panic during evaluation".into());
            }
        }
    }

    let waiting = pending_for_slot(cfg, &state, &date, &slot_key, now);
    if !waiting.is_empty() {
        let message = grouped_message(&waiting);
        match deliver(&message) {
            Ok(sent) => {
                for item in &waiting {
                    let phase_state = state.pending_state_mut(item);
                    phase_state.attempted_slot = Some(slot_key.clone());
                    if sent.is_delivery() {
                        phase_state.delivered_slot = Some(slot_key.clone());
                    } else {
                        phase_state.disabled_slot = Some(slot_key.clone());
                    }
                }
                tick.notification_sent = sent.is_delivery();
                tick.notification_disabled = matches!(sent, Sent::Disabled);
            }
            Err(error) => {
                for item in &waiting {
                    state.pending_state_mut(item).attempted_slot = Some(slot_key.clone());
                }
                tick.delivery_error = Some(error.to_string());
            }
        }
    }
    state.save_at(state_path)?;
    Ok(tick)
}

impl ServeState {
    fn pending_state_mut(&mut self, item: &PendingItem) -> &mut PhaseState {
        match &item.history_key {
            Some(key) => self
                .pending_history
                .get_mut(key)
                .expect("pending history item must still exist"),
            None => self.phase_state_mut(item.phase),
        }
    }
}

fn pending_for_slot(
    cfg: &Config,
    state: &ServeState,
    date: &str,
    slot_key: &str,
    now: DateTime<Local>,
) -> Vec<PendingItem> {
    let mut waiting = Phase::ALL
        .into_iter()
        .filter_map(|phase| {
            let phase_state = state.phase_state(phase)?;
            (phase_state.pending_on.as_deref() == Some(date)
                && phase_state.attempted_slot.as_deref() != Some(slot_key)
                && delivery_reached(cfg, phase, now))
            .then(|| PendingItem {
                phase,
                origin_date: date.to_string(),
                history_key: None,
            })
        })
        .collect::<Vec<_>>();
    for (key, phase_state) in &state.pending_history {
        let Some((origin_date, phase)) = parse_history_key(key) else {
            continue;
        };
        if phase_state.pending_on.as_deref() == Some(origin_date.as_str())
            && phase_state.attempted_slot.as_deref() != Some(slot_key)
            && (is_workday(cfg, now.weekday()) || phase == Phase::Retro)
        {
            waiting.push(PendingItem {
                phase,
                origin_date,
                history_key: Some(key.clone()),
            });
        }
    }
    waiting
}

fn grouped_message(items: &[PendingItem]) -> String {
    let details = items
        .iter()
        .map(|item| {
            format!(
                "{} の{}（`{}`）",
                item.origin_date,
                item.phase.label_ja(),
                item.phase.cli_hint()
            )
        })
        .collect::<Vec<_>>()
        .join("、");
    format!("dayloop: 確認が必要です: {details}")
}

fn parse_history_key(key: &str) -> Option<(String, Phase)> {
    let (date, phase) = key.rsplit_once('/')?;
    let phase = match phase {
        "plan" => Phase::Plan,
        "check" => Phase::Check,
        "close" => Phase::Close,
        "retro" => Phase::Retro,
        _ => return None,
    };
    Some((date.to_string(), phase))
}

/// Evaluation due now, independent from pending/delivery state.
pub fn due_phases(cfg: &Config, state: &ServeState, now: DateTime<Local>) -> Vec<Phase> {
    let date = now.format("%Y-%m-%d").to_string();
    let mut due = Vec::new();
    if is_workday(cfg, now.weekday()) {
        for (phase, spec) in [
            (Phase::Plan, cfg.schedule.plan.as_str()),
            (Phase::Check, cfg.schedule.check.as_str()),
            (Phase::Close, cfg.schedule.close.as_str()),
        ] {
            if time_reached(now, spec)
                && state
                    .phase_state(phase)
                    .and_then(|item| item.evaluated_on.as_deref())
                    != Some(date.as_str())
            {
                due.push(phase);
            }
        }
    }
    if retro_due(cfg, now)
        && state
            .phase_state(Phase::Retro)
            .and_then(|item| item.evaluated_on.as_deref())
            != Some(date.as_str())
    {
        due.push(Phase::Retro);
    }
    due
}

fn due_phases_for_slot(
    cfg: &Config,
    state: &ServeState,
    now: DateTime<Local>,
    slot_key: &str,
) -> Vec<Phase> {
    let date = now.format("%Y-%m-%d").to_string();
    let mut due = due_phases(cfg, state, now);
    for phase in Phase::ALL {
        let Some(item) = state.phase_state(phase) else {
            continue;
        };
        let needs_refresh = item.pending_on.as_deref() == Some(date.as_str())
            || item.error_on.as_deref() == Some(date.as_str());
        if needs_refresh
            && item.evaluated_slot.as_deref() != Some(slot_key)
            && phase_reached(cfg, phase, now)
            && !due.contains(&phase)
        {
            due.push(phase);
        }
    }
    due.sort_by_key(|phase| match phase {
        Phase::Plan => 0,
        Phase::Check => 1,
        Phase::Close => 2,
        Phase::Retro => 3,
    });
    due
}

fn phase_reached(cfg: &Config, phase: Phase, now: DateTime<Local>) -> bool {
    match phase {
        Phase::Plan => is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.plan),
        Phase::Check => is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.check),
        Phase::Close => is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.close),
        Phase::Retro => retro_due(cfg, now),
    }
}

fn delivery_reached(cfg: &Config, phase: Phase, now: DateTime<Local>) -> bool {
    match phase {
        Phase::Plan => is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.plan),
        Phase::Check => is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.check),
        // On retro day, wait until both newly due close and retro can be grouped.
        Phase::Close => {
            evening_notification_time(cfg, now.weekday()).is_some_and(|time| now.time() >= time)
        }
        Phase::Retro => retro_due(cfg, now),
    }
}

fn evaluate_snapshot(phase: Phase, date: &str, cfg: &Config) -> Result<Outcome> {
    if date == util::today() && matches!(phase, Phase::Plan | Phase::Check) {
        serve_intake(cfg);
    }
    let store = Store::open()?;
    // This only creates the explicit routine occurrences and review set. It does
    // not confirm a plan, close a day, discard work, or save a retrospective.
    if matches!(phase, Phase::Plan | Phase::Check | Phase::Close)
        && store
            .get_day(date)?
            .is_some_and(|day| day.closed_at.is_some())
    {
        return Ok(Outcome::Done);
    }
    let _prepared = store.prepare_day(date)?;
    if matches!(phase, Phase::Plan)
        && store
            .get_day(date)?
            .is_some_and(|day| day.plan_confirmed_at.is_some())
    {
        return Ok(Outcome::Done);
    }
    if matches!(phase, Phase::Close)
        && store
            .get_day(date)?
            .is_some_and(|day| day.closed_at.is_some())
    {
        return Ok(Outcome::Done);
    }
    if matches!(phase, Phase::Retro) && week_has_retro_note(&store, date)? {
        return Ok(Outcome::Done);
    }
    let questions = match phase {
        Phase::Plan => engine::plan_view(&store, date)?["questions"]
            .as_array()
            .map_or(0, Vec::len),
        Phase::Check => engine::check_view(&store, date)?["questions"]
            .as_array()
            .map_or(0, Vec::len),
        Phase::Close => engine::close_view(&store, date)?["questions"]
            .as_array()
            .map_or(0, Vec::len),
        Phase::Retro => engine::retro_view(&store, date)?["questions"]
            .as_array()
            .map_or(0, Vec::len),
    };
    if matches!(phase, Phase::Close | Phase::Retro) || questions > 0 {
        Ok(Outcome::Pending(questions.max(1)))
    } else {
        Ok(Outcome::Done)
    }
}

fn week_has_retro_note(store: &Store, date: &str) -> Result<bool> {
    let (from, to) = util::week_range(date)?;
    let mut day = util::parse_date(&from)?;
    let end = util::parse_date(&to)?;
    while day <= end {
        let day_text = day.format("%Y-%m-%d").to_string();
        if store
            .get_day(&day_text)?
            .is_some_and(|record| record.retro_note.is_some())
        {
            return Ok(true);
        }
        day += chrono::Duration::days(1);
    }
    Ok(false)
}

fn is_workday(cfg: &Config, weekday: Weekday) -> bool {
    cfg.schedule
        .workdays
        .iter()
        .any(|item| item.eq_ignore_ascii_case(weekday_token(weekday)))
}

fn weekday_token(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn parse_hm(value: &str) -> Option<NaiveTime> {
    let (hour, minute) = value.trim().split_once(':')?;
    NaiveTime::from_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)
}

fn time_reached(now: DateTime<Local>, spec: &str) -> bool {
    parse_hm(spec).is_some_and(|time| now.time() >= time)
}

fn retro_due(cfg: &Config, now: DateTime<Local>) -> bool {
    let spec = cfg.schedule.retro.trim();
    let (day, time) = match spec.split_once(' ') {
        Some(parts) => parts,
        None => return time_reached(now, spec),
    };
    day.eq_ignore_ascii_case(weekday_token(now.weekday())) && time_reached(now, time)
}

fn current_slot(cfg: &Config, now: DateTime<Local>) -> Option<Slot> {
    if !is_workday(cfg, now.weekday()) {
        return retro_due(cfg, now).then_some(Slot::Evening);
    }
    let evening_time = evening_notification_time(cfg, now.weekday())?;
    if now.time() >= evening_time {
        return Some(Slot::Evening);
    }
    if is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.check) {
        return Some(Slot::Noon);
    }
    if is_workday(cfg, now.weekday()) && time_reached(now, &cfg.schedule.plan) {
        return Some(Slot::Morning);
    }
    None
}

fn evening_notification_time(cfg: &Config, weekday: Weekday) -> Option<NaiveTime> {
    let mut evening = parse_hm(&cfg.schedule.close)?;
    if let Some((day, time)) = cfg.schedule.retro.trim().split_once(' ') {
        if day.eq_ignore_ascii_case(weekday_token(weekday)) {
            evening = evening.max(parse_hm(time)?);
        }
    }
    Some(evening)
}

fn serve_intake(cfg: &Config) {
    if !cfg.intake.outlook {
        return;
    }
    match Store::open() {
        Ok(store) => {
            for result in crate::intake::sync_all(&store, cfg) {
                log_event(&result.serve_line());
            }
        }
        Err(error) => log_event(&format!("intake outlook skipped: {error}")),
    }
}

fn sleep_until_next_minute() {
    let now = Local::now();
    let seconds = 60 - now.timestamp().rem_euclid(60);
    thread::sleep(Duration::from_secs(seconds as u64));
}

fn log_event(message: &str) {
    let path = paths::serve_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(metadata) = std::fs::metadata(&path) {
        if metadata.len() > 10 * 1024 * 1024 {
            let _ = std::fs::rename(&path, path.with_file_name("serve.log.1"));
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(format!("{} {message}\n", util::now()).as_bytes());
    }
}

#[cfg(windows)]
fn detach_console() {
    // Detach only this process; never hide the caller's shared terminal window.
    unsafe { windows_sys::Win32::System::Console::FreeConsole() };
}

#[cfg(not(windows))]
fn detach_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_lifetime_lock_releases_when_dropped() {
        let path = std::env::temp_dir().join(format!("dayloop-lock-{}", ulid::Ulid::new()));
        let first = ServeLock::acquire(&path).unwrap();
        assert!(ServeLock::acquire(&path).is_err());
        drop(first);
        assert!(ServeLock::acquire(&path).is_ok());
        let _ = std::fs::remove_file(path);
    }
}
