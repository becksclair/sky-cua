use serde::{Deserialize, Serialize};

/// Stable operator/host schema for one page of SMS observation.
pub const PHONE_SMS_QUERY_SCHEMA: &str = "sky-cua.sms-query.v1";

/// One bounded, fixed-window SMS query. `profile` is required so there is no
/// implicit device/default selection and no ADB serial/session fallback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSmsQueryRequest {
    pub profile: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default = "default_sms_limit")]
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Raw nullable columns from `Telephony.Sms`. The field names deliberately
/// preserve Android's provider names (including `_id` and `type`) so callers
/// can distinguish absent provider data from normalized host projections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSmsRecord {
    #[serde(rename = "_id")]
    pub id: Option<i64>,
    pub thread_id: Option<i64>,
    pub address: Option<String>,
    pub person: Option<String>,
    pub date: Option<i64>,
    pub date_sent: Option<i64>,
    pub protocol: Option<i64>,
    pub read: Option<i64>,
    pub status: Option<i64>,
    #[serde(rename = "type")]
    pub message_type: Option<i64>,
    pub reply_path_present: Option<i64>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub service_center: Option<String>,
    pub locked: Option<i64>,
    pub sub_id: Option<i64>,
    pub creator: Option<String>,
    pub seen: Option<i64>,
    pub priority: Option<i64>,
    pub subscription_id: Option<i64>,
    pub error_code: Option<i64>,
    pub message_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSmsScan {
    pub has_more: bool,
    pub exhausted_as_observed: bool,
    pub snapshot: bool,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSmsQueryError {
    pub code: String,
    pub message: String,
}

/// A successful page has `scan` and no `error`; a failed request has `error`,
/// no messages/cursor, and never exposes a partial page or continuation token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSmsQueryResponse {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    #[serde(default)]
    pub messages: Vec<PhoneSmsRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<PhoneSmsScan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<PhoneSmsQueryError>,
}

fn default_sms_limit() -> u32 {
    250
}
