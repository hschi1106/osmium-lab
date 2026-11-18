/// Share counts of one instrument in an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Position {
    /// Cash shares, including today's fills; negative while shares sold first are not
    /// bought back.
    pub cash: i64,
    /// Shares bought on margin.
    pub margin: i64,
    /// Shares sold short.
    pub short: i64,
}

impl Position {
    /// Whether no share is held, owed or sold short.
    #[must_use]
    pub const fn is_flat(self) -> bool {
        self.cash == 0 && self.margin == 0 && self.short == 0
    }
}
