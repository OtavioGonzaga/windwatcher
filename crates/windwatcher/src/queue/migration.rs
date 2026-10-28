/// Queue migration status, independent of primary-storage migrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueMigrationStatus {
    pub(super) current: Option<i64>,
    pub(super) latest: Option<i64>,
    pub(super) pending: usize,
}
impl QueueMigrationStatus {
    /// Returns highest applied migration version.
    pub fn current_version(&self) -> Option<i64> {
        self.current
    }
    /// Returns newest embedded migration version.
    pub fn latest_version(&self) -> Option<i64> {
        self.latest
    }
    /// Returns number of pending migrations.
    pub fn pending(&self) -> usize {
        self.pending
    }
    /// Returns whether schema matches embedded migrations.
    pub fn is_current(&self) -> bool {
        self.pending == 0 && self.current == self.latest
    }
}
