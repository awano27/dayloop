//! Foreground daemon: once a minute, run due plan/check/close/retro without prompting.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::notify;
use crate::paths;
use crate::rituals::{self, Outcome, Ui};
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
    fn as_str(self) -> &'static str {
        match self {
            Phase::Plan => "plan",
            Phase::Check => "check",
            Phase::Close => "close",
            Phase::Retro => "retro",
        }
    }

    fn label_ja(self) -> &'static str {
        match self {
            Phase::Plan => "計画",
            Phase::Check => "途中確認",
            Phase::Close => "クローズ",
            Phase::Retro => "振り返り",
        }
    }

    fn cli_hint(self) -> &'static str {
        match self {
            Phase::Plan => "dayloop plan",
            Phase::Check => "dayloop check",
            Phase::Close => "dayloop close",
            Phase::Retro => "dayloop retro",
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServeState {
    pub plan: Option<String>,
    pub check: Option<String>,
    pub close: Option<String>,
    pub retro: Option<String>,
}

impl ServeState {
    fn load() -> Self {
        let p = paths::serve_state_path();
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<()> {
        let p = paths::serve_state_path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn last(&self, phase: Phase) -> Option<&str> {
        match phase {
            Phase::Plan => self.plan.as_deref(),
            Phase::Check => self.check.as_deref(),
            Phase::Close => self.close.as_deref(),
            Phase::Retro => self.retro.as_deref(),
        }
    }

    fn mark(&mut self, phase: Phase, date: &str) {
        let slot = match phase {
            Phase::Plan => &mut self.plan,
            Phase::Check => &mut self.check,
            Phase::Close => &mut self.close,
            Phase::Retro => &mut self.retro,
        };
        *slot = Some(date.to_string());
    }
}

pub fn run(quiet: bool) -> Result<()> {
    if quiet {
        hide_console();
    }
    let _ = std::fs::create_dir_all(paths::data_dir());
    log_event("serve started");
    let mut state = ServeState::load();
    loop {
        let cfg = config::load();
        let now = Local::now();
        let due = due_phases(&cfg, &state, now);
        for phase in due {
            run_phase(phase, &cfg, &mut state, now);
        }
        sleep_until_next_minute();
    }
}

fn run_phase(phase: Phase, cfg: &Config, state: &mut ServeState, now: DateTime<Local>) {
    let date = now.format("%Y-%m-%d").to_string();
    let outcome = catch_unwind(AssertUnwindSafe(|| execute(phase, &date)));
    match outcome {
        Ok(Ok(Outcome::Done)) => {
            log_event(&format!("{} {date} done", phase.as_str()));
            state.mark(phase, &date);
            let _ = state.save();
        }
        Ok(Ok(Outcome::Pending(n))) => {
            log_event(&format!("{} {date} pending {n}", phase.as_str()));
            let sent = notify::send(
                &cfg.notify.method,
                &format!(
                    "dayloop: {date} の{} 未確定 {n} 件。`{}` を実行してください",
                    phase.label_ja(),
                    phase.cli_hint()
                ),
            );
            log_event(&sent.log_line());
            state.mark(phase, &date);
            let _ = state.save();
        }
        Ok(Err(e)) => {
            log_event(&format!("{} {date} error {e}", phase.as_str()));
        }
        Err(_) => {
            log_event(&format!("{} {date} panic", phase.as_str()));
        }
    }
}

fn execute(phase: Phase, date: &str) -> Result<Outcome> {
    if matches!(phase, Phase::Plan | Phase::Check) {
        serve_intake();
    }
    let store = Store::open()?;
    let ui = Ui::new(true);
    let out = match phase {
        Phase::Plan => rituals::plan_ritual(&store, &ui, date)?,
        Phase::Check => rituals::check_ritual(&store, &ui, date)?,
        Phase::Close => rituals::close_ritual(&store, &ui, date)?,
        Phase::Retro => rituals::retro_ritual(&store, &ui, date)?,
    };
    if let Err(e) = crate::markdown::export(&store, date) {
        log_event(&format!("markdown export: {e}"));
    }
    Ok(out)
}

pub fn due_phases(cfg: &Config, state: &ServeState, now: DateTime<Local>) -> Vec<Phase> {
    let today = now.format("%Y-%m-%d").to_string();
    let mut out = Vec::new();
    if is_workday(cfg, now.weekday()) {
        for (phase, spec) in [
            (Phase::Plan, cfg.schedule.plan.as_str()),
            (Phase::Check, cfg.schedule.check.as_str()),
            (Phase::Close, cfg.schedule.close.as_str()),
        ] {
            if time_reached(now, spec) && state.last(phase) != Some(today.as_str()) {
                out.push(phase);
            }
        }
    }
    if retro_due(cfg, now) && state.last(Phase::Retro) != Some(today.as_str()) {
        out.push(Phase::Retro);
    }
    out
}

fn is_workday(cfg: &Config, w: Weekday) -> bool {
    cfg.schedule
        .workdays
        .iter()
        .any(|d| d.eq_ignore_ascii_case(weekday_token(w)))
}

fn weekday_token(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

fn time_reached(now: DateTime<Local>, spec: &str) -> bool {
    let Some((h, m)) = parse_hm(spec) else {
        return false;
    };
    let Some(t) = NaiveTime::from_hms_opt(h, m, 0) else {
        return false;
    };
    now.time() >= t
}

fn retro_due(cfg: &Config, now: DateTime<Local>) -> bool {
    let spec = cfg.schedule.retro.trim();
    let (day_part, time_part) = match spec.split_once(' ') {
        Some(p) => p,
        None => return time_reached(now, spec),
    };
    if !day_part.eq_ignore_ascii_case(weekday_token(now.weekday())) {
        return false;
    }
    time_reached(now, time_part)
}

fn serve_intake() {
    let cfg = config::load();
    if !cfg.intake.outlook {
        return;
    }
    match Store::open() {
        Ok(store) => {
            for r in crate::intake::sync_all(&store, &cfg) {
                log_event(&r.serve_line());
            }
        }
        Err(e) => log_event(&format!("intake outlook skipped: {e}")),
    }
}

fn sleep_until_next_minute() {
    let now = Local::now();
    let secs = 60 - now.timestamp() % 60;
    thread::sleep(Duration::from_secs(secs as u64));
}

fn log_event(msg: &str) {
    let p = paths::serve_log_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(meta) = std::fs::metadata(&p) {
        if meta.len() > 10 * 1024 * 1024 {
            let bak = p.with_file_name("serve.log.1");
            let _ = std::fs::rename(&p, bak);
        }
    }
    let line = format!("{} {msg}\n", util::now());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(windows)]
fn hide_console() {
    use windows_sys::Win32::System::Console::FreeConsole;
    unsafe {
        FreeConsole();
    }
}

#[cfg(not(windows))]
fn hide_console() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Notify, Schedule};
    use chrono::TimeZone;

    fn cfg() -> Config {
        Config {
            schedule: Schedule {
                plan: "08:30".into(),
                check: "13:00".into(),
                close: "18:00".into(),
                retro: "Fri 18:30".into(),
                workdays: default_wd(),
            },
            notify: Notify { method: "file".into() },
            intake: crate::config::IntakeConfig::default(),
            jev: crate::config::JevConfig::default(),
            observe: crate::config::ObserveConfig::default(),
            minutes: crate::config::MinutesConfig::default(),
        }
    }

    fn default_wd() -> Vec<String> {
        ["Mon", "Tue", "Wed", "Thu", "Fri"]
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap()
    }

    #[test]
    fn before_close_is_not_due() {
        // 2026-09-03 is Thursday
        let due = due_phases(&cfg(), &ServeState::default(), at(2026, 9, 3, 17, 59));
        assert!(!due.contains(&Phase::Close));
        assert!(due.contains(&Phase::Plan));
        assert!(due.contains(&Phase::Check));
    }

    #[test]
    fn catch_up_runs_missed_phases_once() {
        let now = at(2026, 9, 3, 18, 5);
        let due = due_phases(&cfg(), &ServeState::default(), now);
        assert_eq!(due, vec![Phase::Plan, Phase::Check, Phase::Close]);
        let mut state = ServeState::default();
        state.mark(Phase::Plan, "2026-09-03");
        let due = due_phases(&cfg(), &state, now);
        assert_eq!(due, vec![Phase::Check, Phase::Close]);
    }

    #[test]
    fn weekend_skips_work_phases() {
        // 2026-09-05 is Saturday
        let due = due_phases(&cfg(), &ServeState::default(), at(2026, 9, 5, 18, 5));
        assert!(due.is_empty());
    }

    #[test]
    fn friday_retro_after_1830() {
        // 2026-09-04 is Friday
        let due = due_phases(&cfg(), &ServeState::default(), at(2026, 9, 4, 18, 30));
        assert!(due.contains(&Phase::Retro));
        assert!(due.contains(&Phase::Close));
    }
}
