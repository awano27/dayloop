//! Outlook desktop COM adapter. Compiles without Outlook; fails closed at runtime.

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Local, NaiveDate};

use super::model::{CalendarItem, MailItem};
use super::Source;

pub struct OutlookCom {
    read_body: bool,
}

#[cfg(any(windows, test))]
fn checked_count(count: i32) -> Result<i32> {
    if !(0..=500).contains(&count) {
        bail!("source result is invalid or exceeds the 500 item safety limit; narrow the requested scope");
    }
    Ok(count)
}

#[cfg(any(windows, test))]
fn checked_serial(value: f64) -> Result<f64> {
    if !value.is_finite() || !(-657434.0..2958466.0).contains(&value) {
        bail!("invalid Outlook date");
    }
    Ok(value)
}

#[cfg(windows)]
fn is_no_object(value: &windows::core::VARIANT) -> bool {
    // GetFirst/GetNext returns Nothing at end of enumeration. Other conversion
    // failures remain errors rather than being mistaken for the end.
    unsafe {
        let raw = value.as_raw();
        let vt = raw.Anonymous.Anonymous.vt;
        vt == 0
            || vt == 1
            || ((vt == 9 || vt == 13) && raw.Anonymous.Anonymous.Anonymous.pdispVal.is_null())
    }
}

impl OutlookCom {
    pub fn new(read_body: bool) -> Self {
        Self { read_body }
    }
}

impl Source for OutlookCom {
    fn name(&self) -> &'static str {
        "outlook"
    }

    fn mails(&self, since: DateTime<Local>) -> Result<Vec<MailItem>> {
        #[cfg(not(windows))]
        {
            let _ = since;
            bail!("new_outlook_or_missing: Outlook COM はこの OS では使えません。段階4の Edge 経由になります");
        }
        #[cfg(windows)]
        {
            let read_body = self.read_body;
            com_timeout(30, move || fetch_mails(since, read_body))
        }
    }

    fn events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>> {
        #[cfg(not(windows))]
        {
            let _ = (from, to);
            bail!("new_outlook_or_missing: Outlook COM はこの OS では使えません。段階4の Edge 経由になります");
        }
        #[cfg(windows)]
        {
            com_timeout(30, move || fetch_events(from, to))
        }
    }
}

pub fn probe_version() -> Result<String> {
    #[cfg(not(windows))]
    {
        bail!("new_outlook_or_missing: Outlook COM はこの OS では使えません");
    }
    #[cfg(windows)]
    {
        com_timeout(10, || {
            let _init = crate::com::ComInit::new()?;
            let app = match crate::com::create_progid("Outlook.Application") {
                Ok(a) => a,
                Err(_) => {
                    bail!("new_outlook_or_missing: Outlook.Application がありません。新しい Outlook か未インストール。段階4の Edge 経由になります")
                }
            };
            let v = crate::com::get(&app, "Version")?;
            crate::com::as_string(&v)
        })
    }
}

#[cfg(windows)]
fn com_timeout<T: Send + 'static>(
    secs: u64,
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs))
        .map_err(|_| anyhow!("timeout"))?
}

#[cfg(windows)]
fn fetch_mails(since: DateTime<Local>, read_body: bool) -> Result<Vec<MailItem>> {
    use crate::com;
    let _init = com::ComInit::new()?;
    let app = match com::create_progid("Outlook.Application") {
        Ok(a) => a,
        Err(_) => {
            bail!("new_outlook_or_missing: Outlook.Application がありません。新しい Outlook か未インストール。段階4の Edge 経由になります")
        }
    };
    let ns_v = com::call(&app, "GetNamespace", &[com::var_bstr("MAPI")]).map_err(|_| {
        anyhow!("new_outlook_or_missing: GetNamespace に失敗。段階4の Edge 経由になります")
    })?;
    let ns = com::as_dispatch(&ns_v)?;
    let folder_v = com::call(&ns, "GetDefaultFolder", &[com::var_i32(6)])?;
    let folder = com::as_dispatch(&folder_v)?;
    let items_v = com::get(&folder, "Items")?;
    let items = com::as_dispatch(&items_v)?;
    com::call(
        &items,
        "Sort",
        &[com::var_bstr("[ReceivedTime]"), com::var_bool(true)],
    )?;
    let since_s = since.format("%Y/%m/%d %H:%M").to_string();
    let restrict = format!("[ReceivedTime] >= '{since_s}'");
    let filtered_v = com::call(&items, "Restrict", &[com::var_bstr(&restrict)])?;
    let filtered = com::as_dispatch(&filtered_v)?;
    let n = checked_count(com::as_i32(&com::get(&filtered, "Count")?)?)?;
    let mut out = Vec::new();
    for i in 1..=n {
        let item_v = com::call(&filtered, "Item", &[com::var_i32(i)])?;
        let item = com::as_dispatch(&item_v)?;
        let entry_id = com::as_string(&com::get(&item, "EntryID")?)?;
        if entry_id.is_empty() {
            bail!("mail entry id is missing");
        }
        let subject = com::as_string(&com::get(&item, "Subject")?)?;
        let sender_name = com::as_string(&com::get(&item, "SenderName")?)?;
        let received = checked_serial(com::as_f64(&com::get(&item, "ReceivedTime")?)?)?;
        let received_at = com::date_serial_to_local(received);
        if received_at < since {
            continue;
        }
        let unread = com::as_bool(&com::get(&item, "UnRead")?)?;
        let flag_status = com::as_i32(&com::get(&item, "FlagStatus")?)?;
        let flag_request = com::as_string(&com::get(&item, "FlagRequest")?).ok();
        let conversation_topic = com::as_string(&com::get(&item, "ConversationTopic")?).ok();
        let body_excerpt = if read_body {
            Some(
                com::as_string(&com::get(&item, "Body")?)?
                    .chars()
                    .take(2000)
                    .collect(),
            )
        } else {
            None
        };
        out.push(MailItem {
            entry_id,
            subject,
            sender_name,
            received_at,
            unread,
            flagged: flag_status != 0,
            flag_request: flag_request.filter(|s| !s.is_empty()),
            body_excerpt,
            conversation_topic,
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn fetch_events(from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>> {
    use crate::com;
    let _init = com::ComInit::new()?;
    let app = match com::create_progid("Outlook.Application") {
        Ok(a) => a,
        Err(_) => {
            bail!("new_outlook_or_missing: Outlook.Application がありません。新しい Outlook か未インストール。段階4の Edge 経由になります")
        }
    };
    let ns_v = com::call(&app, "GetNamespace", &[com::var_bstr("MAPI")]).map_err(|_| {
        anyhow!("new_outlook_or_missing: GetNamespace に失敗。段階4の Edge 経由になります")
    })?;
    let ns = com::as_dispatch(&ns_v)?;
    let me = com::as_string(&com::get(
        &com::as_dispatch(&com::get(&ns, "CurrentUser")?)?,
        "Name",
    )?)
    .unwrap_or_default();
    let folder_v = com::call(&ns, "GetDefaultFolder", &[com::var_i32(9)])?;
    let folder = com::as_dispatch(&folder_v)?;
    let items_v = com::get(&folder, "Items")?;
    let items = com::as_dispatch(&items_v)?;
    com::call(&items, "Sort", &[com::var_bstr("[Start]")])?;
    com::put(&items, "IncludeRecurrences", com::var_bool(true))?;
    let from_s = from.format("%Y/%m/%d 00:00").to_string();
    let to_s = (to + chrono::Duration::days(1))
        .format("%Y/%m/%d 00:00")
        .to_string();
    let restrict = format!("[Start] >= '{from_s}' AND [Start] < '{to_s}'");
    let filtered_v = com::call(&items, "Restrict", &[com::var_bstr(&restrict)])?;
    let filtered = com::as_dispatch(&filtered_v)?;
    // Count is undefined for IncludeRecurrences. Enumerate until a null object,
    // and fail rather than silently truncate when the bounded window is too large.
    let mut out = Vec::new();
    for i in 0..=500 {
        let item_v = com::call(&filtered, if i == 0 { "GetFirst" } else { "GetNext" }, &[])?;
        if is_no_object(&item_v) {
            break;
        }
        if i == 500 {
            bail!("calendar exceeds the 500 item safety limit");
        }
        let item = com::as_dispatch(&item_v)?;
        let entry_id = com::as_string(&com::get(&item, "EntryID")?)?;
        if entry_id.is_empty() {
            bail!("calendar entry id is missing");
        }
        let subject = com::as_string(&com::get(&item, "Subject")?)?;
        let start =
            com::date_serial_to_local(checked_serial(com::as_f64(&com::get(&item, "Start")?)?)?);
        let end =
            com::date_serial_to_local(checked_serial(com::as_f64(&com::get(&item, "End")?)?)?);
        if start.date_naive() < from || start.date_naive() > to {
            continue;
        }
        if end < start {
            bail!("calendar end precedes start");
        }
        let location = com::as_string(&com::get(&item, "Location")?).ok();
        let organizer = com::as_string(&com::get(&item, "Organizer")?).ok();
        let response_required = com::as_bool(&com::get(&item, "ResponseRequested")?)?;
        let all_day = com::as_bool(&com::get(&item, "AllDayEvent")?)?;
        let meeting_status = com::as_i32(&com::get(&item, "MeetingStatus")?)?;
        let is_organizer = meeting_status == 1 && organizer.as_deref() == Some(me.as_str());
        out.push(CalendarItem {
            entry_id,
            subject,
            start,
            end,
            location: location.filter(|s| !s.is_empty()),
            organizer,
            is_organizer,
            response_required,
            all_day,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_counts_and_dates_fail_instead_of_claiming_empty_or_complete_results() {
        assert_eq!(checked_count(0).unwrap(), 0);
        assert_eq!(checked_count(500).unwrap(), 500);
        assert!(checked_count(-1).is_err());
        assert!(checked_count(501).is_err());
        assert!(checked_serial(f64::NAN).is_err());
        assert!(checked_serial(f64::INFINITY).is_err());
        assert!(checked_serial(1e30).is_err());
        assert!(checked_serial(46000.5).is_ok());
    }
}
