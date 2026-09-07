use crate::{FoundationError, SearchHit, SearchRepository};
pub const MAX_SEARCH_QUERY_CHARS: usize = 256;
pub const MAX_SEARCH_QUERY_BYTES: usize = 1024;
pub struct SearchService<'a> {
    repository: &'a dyn SearchRepository,
}
impl<'a> SearchService<'a> {
    pub fn new(repository: &'a dyn SearchRepository) -> Self {
        Self { repository }
    }
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>, FoundationError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }
        if query.chars().count() > MAX_SEARCH_QUERY_CHARS
            || query.len() > MAX_SEARCH_QUERY_BYTES
            || query.chars().any(char::is_control)
        {
            return Err(FoundationError::validation_failed());
        }
        self.repository.search(query, 100)
    }

    pub fn rebuild_index(&self) -> Result<(), FoundationError> {
        self.repository.rebuild_index()
    }
}
