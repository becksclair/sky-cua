use uuid::Uuid;

#[must_use]
pub fn new_snapshot_id() -> String {
    Uuid::new_v4().to_string()
}
