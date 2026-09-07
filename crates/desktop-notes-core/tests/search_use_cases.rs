use std::sync::Mutex;

use desktop_notes_core::{
    ErrorCode, FoundationError, MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_QUERY_CHARS, SearchHit,
    SearchRepository, SearchService,
};

#[derive(Default)]
struct RecordingRepository {
    calls: Mutex<Vec<(String, u32)>>,
}

impl SearchRepository for RecordingRepository {
    fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, FoundationError> {
        self.calls.lock().unwrap().push((query.to_owned(), limit));
        Ok(Vec::new())
    }

    fn rebuild_index(&self) -> Result<(), FoundationError> {
        Ok(())
    }
}

#[test]
fn search_service_trims_valid_queries_and_applies_a_fixed_result_limit() {
    let repository = RecordingRepository::default();
    let service = SearchService::new(&repository);

    assert!(service.search("  天气 Search \"OR\"  ").unwrap().is_empty());
    assert_eq!(
        *repository.calls.lock().unwrap(),
        vec![("天气 Search \"OR\"".to_owned(), 100)]
    );
}

#[test]
fn empty_queries_do_not_reach_the_repository() {
    let repository = RecordingRepository::default();
    let service = SearchService::new(&repository);

    assert!(service.search(" \t\r\n ").unwrap().is_empty());
    assert!(repository.calls.lock().unwrap().is_empty());
}

#[test]
fn oversized_and_control_character_queries_fail_closed() {
    let repository = RecordingRepository::default();
    let service = SearchService::new(&repository);
    let too_many_chars = "a".repeat(MAX_SEARCH_QUERY_CHARS + 1);
    let too_many_bytes = "界".repeat((MAX_SEARCH_QUERY_BYTES / 3) + 1);

    for invalid in [&too_many_chars, &too_many_bytes, "safe\u{0}unsafe"] {
        let error = service.search(invalid).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ValidationFailed);
        assert_eq!(
            error.safe_message(),
            "The note request did not match the supported format."
        );
    }
    assert!(repository.calls.lock().unwrap().is_empty());
}
