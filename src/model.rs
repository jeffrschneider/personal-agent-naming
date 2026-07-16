//! Catalog data shapes. `Listing` is the document; the columns lifted out of
//! the manifest exist to be filtered and indexed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Listing {
    pub id: Uuid,
    pub source: String,
    pub source_id: String,
    pub name: String,
    pub description: String,
    pub manifest: serde_json::Value,
    pub specialties: Vec<String>,
    pub protocol: String,
    pub trust: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Joined from `presence` (LEFT JOIN — a listing may have no signal yet).
    pub presence: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// A manual submission (the 'manual' connector). Other connectors construct
/// upserts directly.
#[derive(Debug, Deserialize)]
pub struct SubmitListing {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Full document; stored verbatim.
    #[serde(default)]
    pub manifest: serde_json::Value,
    /// Namespaced claims: "build:macos", "access:hubspot-api", …
    #[serde(default)]
    pub specialties: Vec<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    /// Stable identity for idempotent re-submission; defaults to a fresh id.
    #[serde(default)]
    pub source_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Full-text query over name + description.
    #[serde(default)]
    pub q: Option<String>,
    /// Exact specialty claim filter, e.g. "access:hubspot-api".
    #[serde(default)]
    pub specialty: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// "online" filters to listings with live presence.
    #[serde(default)]
    pub presence: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}
