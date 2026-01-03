//! Type conversions between domain types and protobuf types

use chrono::{DateTime, Utc};
use prost_types::Timestamp;

/// Convert chrono DateTime to protobuf Timestamp
pub fn to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

/// Convert protobuf Timestamp to chrono DateTime
pub fn from_timestamp(ts: Timestamp) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(ts.seconds, ts.nanos as u32).unwrap_or_else(Utc::now)
}

/// Convert Unix timestamp (seconds since epoch) to protobuf Timestamp
pub fn to_timestamp_from_secs(secs: i64) -> Timestamp {
    Timestamp {
        seconds: secs,
        nanos: 0,
    }
}
