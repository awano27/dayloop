//! Outlook desktop COM adapter. Compiles without Outlook; fails closed at runtime.

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Local, NaiveDate};

use super::model::{CalendarItem, MailItem};
use super::Source;

pub struct OutlookCom {
    read_body: bool,
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
fn com_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
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
    let ns_v = com::call(&app, "GetNamespace", &[com::var_bstr("MAPI")])
        .map_err(|_| anyhow!("new_outlook_or_missing: GetNamespace に失敗。段階4の Edge 経由になります"))?;
    let ns = com::as_dispatch(&ns_v)?;
    let folder_v = com::call(&ns, "GetDefaultFolder", &[com::var_i32(6)])?;
    let folder = com::as_dispatch(&folder_v)?;
    let items_v = com::get(&folder, "Items")?;
    let items = com::as_dispatch(&items_v)?;
    let _ = com::call(
        &items,
        "Sort",
        &[com::var_bstr("[ReceivedTime]"), com::var_bool(true)],
    );
    let since_s = since.format("%Y/%m/%d %H:%M").to_string();
    let restrict = format!("[ReceivedTime] >= '{since_s}'");
    let filtered_v = com::call(&items, "Restrict", &[com::var_bstr(&restrict)]).unwrap_or(items_v);
    let filtered = com::as_dispatch(&filtered_v).unwrap_or(items);
    let count = com::as_i32(&com::get(&filtered, "Count")?).unwrap_or(0);
    let n = count.min(500);
    let mut out = Vec::new();
    for i in 1..=n {
        let item_v = match com::call(&filtered, "Item", &[com::var_i32(i)]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let item = match com::as_dispatch(&item_v) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let entry_id = com::as_string(&com::get(&item, "EntryID")?).unwrap_or_default();
        if entry_id.is_empty() {
            continue;
        }
        let subject = com::as_string(&com::get(&item, "Subject")?).unwrap_or_default();
        let sender_name = com::as_string(&com::get(&item, "SenderName")?).unwrap_or_default();
        let received = com::as_f64(&com::get(&item, "ReceivedTime")?).unwrap_or(0.0);
        let unread = com::as_bool(&com::get(&item, "UnRead")?).unwrap_or(false);
        let flag_status = com::as_i32(&com::get(&item, "FlagStatus")?).unwrap_or(0);
        let flag_request = com::as_string(&com::get(&item, "FlagRequest")?).ok();
        let conversation_topic = com::as_string(&com::get(&item, "ConversationTopic")?).ok();
        let body_excerpt = if read_body {
            com::get(&item, "Body")
                .ok()
                .and_then(|v| com::as_string(&v).ok())
                .map(|s: String| s.chars().take(2000).collect())
        } else {
            None
        };
        out.push(MailItem {
            entry_id,
            subject,
            sender_name,
            received_at: com::date_serial_to_local(received),
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
    let ns_v = com::call(&app, "GetNamespace", &[com::var_bstr("MAPI")])
        .map_err(|_| anyhow!("new_outlook_or_missing: GetNamespace に失敗。段階4の Edge 経由になります"))?;
    let ns = com::as_dispatch(&ns_v)?;
    let me = com::as_string(&com::get(&com::as_dispatch(&com::get(&ns, "CurrentUser")?)?, "Name")?).unwrap_or_default();
    let folder_v = com::call(&ns, "GetDefaultFolder", &[com::var_i32(9)])?;
    let folder = com::as_dispatch(&folder_v)?;
    let items_v = com::get(&folder, "Items")?;
    let items = com::as_dispatch(&items_v)?;
    let _ = com::put(&items, "IncludeRecurrences", com::var_bool(true));
    let _ = com::call(&items, "Sort", &[com::var_bstr("[Start]")]);
    let from_s = from.format("%Y/%m/%d 00:00").to_string();
    let to_s = (to + chrono::Duration::days(1)).format("%Y/%m/%d 00:00").to_string();
    let restrict = format!("[Start] >= '{from_s}' AND [End] <= '{to_s}'");
    let filtered_v = com::call(&items, "Restrict", &[com::var_bstr(&restrict)]).unwrap_or(items_v);
    let filtered = com::as_dispatch(&filtered_v).unwrap_or(items);
    let count = com::as_i32(&com::get(&filtered, "Count")?).unwrap_or(0);
    let n = count.min(500);
    let mut out = Vec::new();
    for i in 1..=n {
        let item_v = match com::call(&filtered, "Item", &[com::var_i32(i)]) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let item = match com::as_dispatch(&item_v) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let entry_id = com::as_string(&com::get(&item, "EntryID")?).unwrap_or_default();
        if entry_id.is_empty() {
            continue;
        }
        let subject = com::as_string(&com::get(&item, "Subject")?).unwrap_or_default();
        let start = com::as_f64(&com::get(&item, "Start")?).unwrap_or(0.0);
        let end = com::as_f64(&com::get(&item, "End")?).unwrap_or(0.0);
        let location = com::as_string(&com::get(&item, "Location")?).ok();
        let organizer = com::as_string(&com::get(&item, "Organizer")?).ok();
        let response_required = com::as_bool(&com::get(&item, "ResponseRequested")?).unwrap_or(false);
        let all_day = com::as_bool(&com::get(&item, "AllDayEvent")?).unwrap_or(false);
        let meeting_status = com::as_i32(&com::get(&item, "MeetingStatus")?).unwrap_or(0);
        let is_organizer = meeting_status == 1 && organizer.as_deref() == Some(me.as_str());
        out.push(CalendarItem {
            entry_id,
            subject,
            start: com::date_serial_to_local(start),
            end: com::date_serial_to_local(end),
            location: location.filter(|s| !s.is_empty()),
            organizer,
            is_organizer,
            response_required,
            all_day,
        });
    }
    Ok(out)
}
