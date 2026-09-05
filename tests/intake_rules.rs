use chrono::{Duration, Local, TimeZone};

use dayloop::config::IntakeConfig;
use dayloop::intake::model::{CalendarItem, MailItem};
use dayloop::intake::rules::{event_prep_hit, mail_hit, strip_subject_prefix};

fn cfg() -> IntakeConfig {
    IntakeConfig {
        important_senders: vec!["田中".into()],
        ..IntakeConfig::default()
    }
}

fn mail(subject: &str, sender: &str, unread: bool, flagged: bool) -> MailItem {
    MailItem {
        entry_id: "e1".into(),
        subject: subject.into(),
        sender_name: sender.into(),
        received_at: Local.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
        unread,
        flagged,
        flag_request: None,
        body_excerpt: None,
        conversation_topic: None,
    }
}

#[test]
fn flagged_wins() {
    let h = mail_hit(&mail("来週の資料", "鈴木", true, true), &cfg()).unwrap();
    assert_eq!(h.reason, "flagged");
    assert!(h.title.starts_with("対応:"));
    assert_eq!(h.source_ref, "outlook:mail:e1");
}

#[test]
fn keyword_on_subject() {
    let h = mail_hit(&mail("ご確認ください", "佐藤", true, false), &cfg()).unwrap();
    assert_eq!(h.reason, "keyword");
    assert!(h.title.starts_with("返信/対応:"));
}

#[test]
fn keyword_miss() {
    assert!(mail_hit(&mail("週刊ニュース", "配信", true, false), &cfg()).is_none());
}

#[test]
fn important_sender_unread() {
    let h = mail_hit(&mail("報告書", "田中部長", true, false), &cfg()).unwrap();
    assert_eq!(h.reason, "important_sender");
    assert!(h.title.starts_with("確認:"));
}

#[test]
fn important_sender_empty_disabled() {
    let mut c = cfg();
    c.important_senders.clear();
    assert!(mail_hit(&mail("報告書", "田中部長", true, false), &c).is_none());
}

#[test]
fn important_sender_read_ignored() {
    assert!(mail_hit(&mail("報告書", "田中部長", false, false), &cfg()).is_none());
}

#[test]
fn prefix_stripped_in_title() {
    let h = mail_hit(&mail("RE: FW: 来週の資料", "鈴木", false, true), &cfg()).unwrap();
    assert!(h.title.contains("来週の資料"));
    assert!(!h.title.contains("RE:"));
}

#[test]
fn strip_subject_prefix_repeats() {
    assert_eq!(strip_subject_prefix("RE: Re: Fwd: 件名"), "件名");
}

fn cal(hours_from_now: i64, is_organizer: bool, response_required: bool) -> (CalendarItem, chrono::DateTime<Local>) {
    let now = Local.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap();
    let start = now + Duration::hours(hours_from_now);
    let item = CalendarItem {
        entry_id: "c1".into(),
        subject: "週次定例".into(),
        start,
        end: start + Duration::hours(1),
        location: None,
        organizer: Some("自分".into()),
        is_organizer,
        response_required,
        all_day: false,
    };
    (item, now)
}

#[test]
fn meeting_prep_23h59_inside() {
    let now = Local.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap();
    let start = now + Duration::hours(24) - Duration::minutes(1);
    let item = CalendarItem {
        entry_id: "c1".into(),
        subject: "週次定例".into(),
        start,
        end: start + Duration::hours(1),
        location: None,
        organizer: Some("自分".into()),
        is_organizer: true,
        response_required: false,
        all_day: false,
    };
    let h = event_prep_hit(&item, now, &cfg()).unwrap();
    assert_eq!(h.reason, "meeting_prep");
    assert!(h.title.starts_with("会議準備:"));
    assert_eq!(h.source_ref, "outlook:cal:c1:prep");
}

#[test]
fn meeting_prep_24h01_outside() {
    let now = Local.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap();
    let start = now + Duration::hours(24) + Duration::minutes(1);
    let item = CalendarItem {
        entry_id: "c1".into(),
        subject: "週次定例".into(),
        start,
        end: start + Duration::hours(1),
        location: None,
        organizer: Some("自分".into()),
        is_organizer: true,
        response_required: false,
        all_day: false,
    };
    assert!(event_prep_hit(&item, now, &cfg()).is_none());
}

#[test]
fn meeting_prep_requires_organizer_or_response() {
    let (item, now) = cal(2, false, false);
    assert!(event_prep_hit(&item, now, &cfg()).is_none());
    let (item, now) = cal(2, false, true);
    assert!(event_prep_hit(&item, now, &cfg()).is_some());
}
