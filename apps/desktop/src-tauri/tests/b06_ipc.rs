use std::sync::Mutex;

use desktop_notes_core::{
    ErrorCode, FoundationError, NoteSummary, SearchHit, SearchMatchTag, SearchRepository,
    SearchService,
};
use desktop_notes_desktop::ipc::handle_search_notes;
use serde_json::json;

#[derive(Default)]
struct RecordingRepository {
    queries: Mutex<Vec<String>>,
}

impl SearchRepository for RecordingRepository {
    fn search(&self, query: &str, _limit: u32) -> Result<Vec<SearchHit>, FoundationError> {
        self.queries.lock().unwrap().push(query.to_owned());
        Ok(vec![SearchHit {
            note: NoteSummary {
                id: "cdd026eb-e8ab-41e0-a6a9-763087d78c4b".to_owned(),
                note_date: "2026-09-06".to_owned(),
                title: "天气 Search".to_owned(),
                updated_at_ms: 123,
                revision: 7,
                is_pinned: false,
            },
            snippet: "正文天气 Search".to_owned(),
            matching_tags: vec![SearchMatchTag {
                id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_owned(),
                name: "天气".to_owned(),
            }],
        }])
    }

    fn rebuild_index(&self) -> Result<(), FoundationError> {
        Ok(())
    }
}

#[test]
fn typed_search_command_returns_only_the_bounded_search_dto() {
    let repository = RecordingRepository::default();
    let service = SearchService::new(&repository);
    let response = serde_json::to_value(handle_search_notes(
        json!({ "protocolVersion": 1, "query": "  天气 Search  " }),
        &service,
    ))
    .unwrap();

    assert_eq!(
        repository.queries.lock().unwrap().as_slice(),
        ["天气 Search"]
    );
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["hits"][0]["id"],
        "cdd026eb-e8ab-41e0-a6a9-763087d78c4b"
    );
    assert_eq!(response["hits"][0]["title"], "天气 Search");
    assert_eq!(response["hits"][0]["snippet"], "正文天气 Search");
    assert_eq!(
        response["hits"][0]["matchingTags"],
        json!([{ "id": "dddddddd-dddd-4ddd-8ddd-dddddddddddd", "name": "天气" }])
    );
    assert_eq!(response["hits"][0]["revision"], 7);
    assert!(response["hits"][0].get("bodyJson").is_none());
}

#[test]
fn malformed_unknown_and_oversized_search_dtos_are_safely_rejected() {
    let repository = RecordingRepository::default();
    let service = SearchService::new(&repository);
    let invalid = [
        json!({}),
        json!({ "protocolVersion": 2, "query": "search" }),
        json!({ "protocolVersion": 1, "query": "search", "sql": "SELECT *" }),
        json!({ "protocolVersion": 1, "query": "x".repeat(257) }),
        json!({ "protocolVersion": 1, "query": "bad\u{0}query" }),
    ];

    for request in invalid {
        let response = serde_json::to_value(handle_search_notes(request, &service)).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(
            response["error"]["code"],
            json!(ErrorCode::ValidationFailed)
        );
        assert_eq!(
            response["error"]["message"],
            "The note request did not match the supported format."
        );
    }
    assert!(repository.queries.lock().unwrap().is_empty());
}
