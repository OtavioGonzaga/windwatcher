/// A summary of embedded migration state without exposing SQLx ledger rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MigrationStatus {
    pub(super) current_version: Option<i64>,
    pub(super) latest_version: Option<i64>,
    pub(super) pending: usize,
}

impl MigrationStatus {
    /// Returns the highest successfully applied embedded migration version.
    #[must_use]
    pub fn current_version(&self) -> Option<i64> {
        self.current_version
    }
    /// Returns the newest embedded migration version.
    #[must_use]
    pub fn latest_version(&self) -> Option<i64> {
        self.latest_version
    }
    /// Returns the count of embedded migrations that have not been applied.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending
    }
    /// Returns whether every embedded migration has been applied.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.pending == 0 && self.current_version == self.latest_version
    }
}
