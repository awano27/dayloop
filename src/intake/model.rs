use chrono::{DateTime, Local};

#[derive(Debug, Clone)]
pub struct MailItem {
    pub entry_id: String,
    pub subject: String,
    pub sender_name: String,
    pub received_at: DateTime<Local>,
    pub unread: bool,
    pub flagged: bool,
    pub flag_request: Option<String>,
    pub body_excerpt: Option<String>,
    pub conversation_topic: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CalendarItem {
    pub entry_id: String,
    pub subject: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub location: Option<String>,
    pub organizer: Option<String>,
    pub is_organizer: bool,
    pub response_required: bool,
    pub all_day: bool,
}
