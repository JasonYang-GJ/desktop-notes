use desktop_notes_core::{ErrorCode, MAX_TAG_NAME_BYTES, MAX_TAG_NAME_CHARS, prepare_tag_name};

#[test]
fn tag_names_are_trimmed_nfkc_normalized_and_case_insensitive() {
    let prepared = prepare_tag_name("  项目Ａ  ").unwrap();

    assert_eq!(prepared.name, "项目Ａ");
    assert_eq!(prepared.normalized_name, "项目a");

    let same_identity = prepare_tag_name("项目a").unwrap();
    assert_eq!(same_identity.normalized_name, prepared.normalized_name);
}

#[test]
fn blank_and_oversized_tag_names_fail_closed() {
    for invalid in ["", " \t\r\n ", &"界".repeat(MAX_TAG_NAME_CHARS + 1)] {
        assert_eq!(
            prepare_tag_name(invalid).unwrap_err().code(),
            ErrorCode::ValidationFailed,
        );
    }

    let byte_heavy = "😀".repeat((MAX_TAG_NAME_BYTES / 4) + 1);
    assert_eq!(
        prepare_tag_name(&byte_heavy).unwrap_err().code(),
        ErrorCode::ValidationFailed,
    );
}
