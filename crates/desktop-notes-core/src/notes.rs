use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

use crate::FoundationError;

pub const BODY_FORMAT: &str = "tiptap-json";
pub const BODY_SCHEMA_VERSION: u32 = 1;
pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_BODY_JSON_BYTES: usize = 1024 * 1024;
const MAX_BODY_NODES: usize = 10_000;
const MAX_BODY_DEPTH: u32 = 32;
const CONTENT_HASH_DOMAIN: &[u8] = b"desktop-notes-note-content-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub id: String,
    pub note_date: String,
    pub title: String,
    pub body_json: String,
    pub body_format: String,
    pub body_schema_version: u32,
    pub body_text: String,
    pub content_hash: [u8; 32],
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revision: u64,
    pub is_pinned: bool,
}

impl Note {
    pub fn from_new(input: NewNote) -> Self {
        Self {
            id: input.id,
            note_date: input.note_date,
            title: input.title,
            body_json: input.body_json,
            body_format: BODY_FORMAT.to_owned(),
            body_schema_version: BODY_SCHEMA_VERSION,
            body_text: input.body_text,
            content_hash: input.content_hash,
            created_at_ms: input.created_at_ms,
            updated_at_ms: input.created_at_ms,
            revision: 1,
            is_pinned: false,
        }
    }

    pub fn content_hash_hex(&self) -> String {
        hex(&self.content_hash)
    }

    pub fn validate_stored(&self) -> Result<(), FoundationError> {
        validate_note_id(&self.id)?;
        if self.revision == 0 || self.created_at_ms < 0 || self.updated_at_ms < self.created_at_ms {
            return Err(FoundationError::validation_failed());
        }
        let value: Value = serde_json::from_str(&self.body_json)
            .map_err(|_| FoundationError::validation_failed())?;
        let prepared = prepare_content(
            &self.note_date,
            &self.title,
            &self.body_format,
            self.body_schema_version,
            &value,
        )?;
        if prepared.canonical_json != self.body_json
            || prepared.body_text != self.body_text
            || prepared.content_hash != self.content_hash
        {
            return Err(FoundationError::validation_failed());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteSummary {
    pub id: String,
    pub note_date: String,
    pub title: String,
    pub updated_at_ms: i64,
    pub revision: u64,
    pub is_pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub note: NoteSummary,
    pub snippet: String,
    pub matching_tags: Vec<SearchMatchTag>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatchTag {
    pub id: String,
    pub name: String,
}

impl From<&Note> for NoteSummary {
    fn from(note: &Note) -> Self {
        Self {
            id: note.id.clone(),
            note_date: note.note_date.clone(),
            title: note.title.clone(),
            updated_at_ms: note.updated_at_ms,
            revision: note.revision,
            is_pinned: note.is_pinned,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreateNoteRequest {
    pub note_date: String,
    pub title: String,
    pub body_format: String,
    pub body_schema_version: u32,
    pub body_json: Value,
}

#[derive(Clone, Debug)]
pub struct UpdateNoteContentRequest {
    pub note_id: String,
    pub title: String,
    pub body_format: String,
    pub body_schema_version: u32,
    pub body_json: Value,
    pub base_revision: u64,
    pub client_change_id: String,
    pub content_hash: String,
}

#[derive(Clone, Debug)]
pub struct DeleteNoteRequest {
    pub note_id: String,
    pub base_revision: u64,
    pub client_change_id: String,
}

#[derive(Clone, Debug)]
pub struct NewNote {
    pub id: String,
    pub note_date: String,
    pub title: String,
    pub body_json: String,
    pub body_text: String,
    pub content_hash: [u8; 32],
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct NoteUpdate {
    pub id: String,
    pub title: String,
    pub body_json: String,
    pub body_text: String,
    pub content_hash: [u8; 32],
    pub updated_at_ms: i64,
    pub base_revision: u64,
}

#[derive(Clone, Debug)]
pub struct NoteDeletion {
    pub id: String,
    pub deleted_at_ms: i64,
    pub base_revision: u64,
}

pub(crate) struct PreparedContent {
    pub canonical_json: String,
    pub body_text: String,
    pub content_hash: [u8; 32],
}

pub(crate) fn prepare_content(
    note_date: &str,
    title: &str,
    body_format: &str,
    body_schema_version: u32,
    body_json: &Value,
) -> Result<PreparedContent, FoundationError> {
    validate_note_date(note_date)?;
    validate_title(title)?;
    if body_format != BODY_FORMAT || body_schema_version != BODY_SCHEMA_VERSION {
        return Err(FoundationError::validation_failed());
    }
    if serde_json::to_vec(body_json)
        .map_err(|_| FoundationError::validation_failed())?
        .len()
        > MAX_BODY_JSON_BYTES
    {
        return Err(FoundationError::validation_failed());
    }

    let mut node_count = 0;
    let canonical = canonical_doc(body_json, 0, &mut node_count)?;
    let canonical_json =
        serde_json::to_string(&canonical).map_err(|_| FoundationError::validation_failed())?;
    let body_text = document_text(&canonical)?;
    let content_hash = calculate_content_hash(note_date, title, &canonical_json);
    Ok(PreparedContent {
        canonical_json,
        body_text,
        content_hash,
    })
}

pub fn calculate_content_hash_hex(note_date: &str, title: &str, body_json: &str) -> String {
    hex(&calculate_content_hash(note_date, title, body_json))
}

pub fn extract_image_asset_occurrences(body_json: &str) -> Result<Vec<String>, FoundationError> {
    let value: Value =
        serde_json::from_str(body_json).map_err(|_| FoundationError::validation_failed())?;
    let mut assets = Vec::new();
    collect_image_asset_occurrences(&value, &mut assets)?;
    Ok(assets)
}

fn collect_image_asset_occurrences(
    value: &Value,
    assets: &mut Vec<String>,
) -> Result<(), FoundationError> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_image_asset_occurrences(value, assets)?;
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("imageRef") {
                let asset_id = object
                    .get("attrs")
                    .and_then(Value::as_object)
                    .and_then(|attrs| attrs.get("asset_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(FoundationError::validation_failed)?;
                validate_note_id(asset_id)?;
                assets.push(asset_id.to_owned());
            }
            for (key, child) in object {
                if key != "attrs" {
                    collect_image_asset_occurrences(child, assets)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_note_id(value: &str) -> Result<(), FoundationError> {
    if is_uuid(value) {
        Ok(())
    } else {
        Err(FoundationError::validation_failed())
    }
}

pub fn validate_note_date(value: &str) -> Result<(), FoundationError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(FoundationError::validation_failed());
    }
    let year = parse_digits(&bytes[0..4])?;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return Err(FoundationError::validation_failed()),
    };
    if day == 0 || day > max_day {
        return Err(FoundationError::validation_failed());
    }
    Ok(())
}

pub(crate) fn validate_client_change_id(value: &str) -> Result<(), FoundationError> {
    validate_note_id(value)
}

pub(crate) fn decode_content_hash(value: &str) -> Result<[u8; 32], FoundationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FoundationError::validation_failed());
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[index] = (hex_value(chunk[0])? << 4) | hex_value(chunk[1])?;
    }
    Ok(output)
}

fn validate_title(value: &str) -> Result<(), FoundationError> {
    if value.chars().count() <= MAX_TITLE_CHARS {
        Ok(())
    } else {
        Err(FoundationError::validation_failed())
    }
}

fn canonical_doc(value: &Value, depth: u32, nodes: &mut usize) -> Result<Value, FoundationError> {
    check_depth_and_count(depth, nodes)?;
    let object = exact_object(value, &["type", "content"], &["type", "content"])?;
    require_type(object, "doc")?;
    let content = required_array(object, "content")?;
    if content.is_empty() {
        return invalid();
    }
    let canonical = content
        .iter()
        .map(|node| canonical_block(node, depth + 1, nodes))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(object_with([
        ("type", Value::String("doc".to_owned())),
        ("content", Value::Array(canonical)),
    ]))
}

fn canonical_block(value: &Value, depth: u32, nodes: &mut usize) -> Result<Value, FoundationError> {
    check_depth_and_count(depth, nodes)?;
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    match object.get("type").and_then(Value::as_str) {
        Some("paragraph") => canonical_text_block(object, "paragraph", false, depth, nodes),
        Some("heading") => canonical_heading(object, depth, nodes),
        Some("codeBlock") => canonical_code_block(object, depth, nodes),
        Some("bulletList") => canonical_list(object, "bulletList", "listItem", depth, nodes),
        Some("orderedList") => canonical_ordered_list(object, depth, nodes),
        Some("taskList") => canonical_list(object, "taskList", "taskItem", depth, nodes),
        _ => invalid(),
    }
}

fn canonical_text_block(
    object: &Map<String, Value>,
    kind: &str,
    require_attrs: bool,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    let allowed = if require_attrs {
        &["type", "attrs", "content"][..]
    } else {
        &["type", "content"][..]
    };
    let required = if require_attrs {
        &["type", "attrs"][..]
    } else {
        &["type"][..]
    };
    exact_map(object, allowed, required)?;
    require_type(object, kind)?;
    let content = optional_array(object, "content")?
        .unwrap_or(&[])
        .iter()
        .map(|node| canonical_inline(node, depth + 1, nodes))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String(kind.to_owned()));
    if !content.is_empty() {
        output.insert("content".to_owned(), Value::Array(content));
    }
    Ok(Value::Object(output))
}

fn canonical_heading(
    object: &Map<String, Value>,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "attrs", "content"], &["type", "attrs"])?;
    require_type(object, "heading")?;
    let attrs = exact_object(
        object
            .get("attrs")
            .ok_or_else(FoundationError::validation_failed)?,
        &["level"],
        &["level"],
    )?;
    let level = attrs
        .get("level")
        .and_then(Value::as_u64)
        .filter(|level| (1..=6).contains(level))
        .ok_or_else(FoundationError::validation_failed)?;
    let content = optional_array(object, "content")?
        .unwrap_or(&[])
        .iter()
        .map(|node| canonical_inline(node, depth + 1, nodes))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("heading".to_owned()));
    output.insert(
        "attrs".to_owned(),
        object_with([("level", Value::Number(level.into()))]),
    );
    if !content.is_empty() {
        output.insert("content".to_owned(), Value::Array(content));
    }
    Ok(Value::Object(output))
}

fn canonical_code_block(
    object: &Map<String, Value>,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "content"], &["type"])?;
    require_type(object, "codeBlock")?;
    let content = optional_array(object, "content")?
        .unwrap_or(&[])
        .iter()
        .map(|value| {
            check_depth_and_count(depth + 1, nodes)?;
            let child = exact_object(value, &["type", "text"], &["type", "text"])?;
            require_type(child, "text")?;
            let text = non_empty_string(child, "text")?;
            Ok(object_with([
                ("type", Value::String("text".to_owned())),
                ("text", Value::String(text.to_owned())),
            ]))
        })
        .collect::<Result<Vec<_>, FoundationError>>()?;
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("codeBlock".to_owned()));
    if !content.is_empty() {
        output.insert("content".to_owned(), Value::Array(content));
    }
    Ok(Value::Object(output))
}

fn canonical_list(
    object: &Map<String, Value>,
    kind: &str,
    item_kind: &str,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "content"], &["type", "content"])?;
    require_type(object, kind)?;
    let content = required_array(object, "content")?;
    if content.is_empty() {
        return invalid();
    }
    let canonical = content
        .iter()
        .map(|item| canonical_list_item(item, item_kind, depth + 1, nodes))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(object_with([
        ("type", Value::String(kind.to_owned())),
        ("content", Value::Array(canonical)),
    ]))
}

fn canonical_ordered_list(
    object: &Map<String, Value>,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "attrs", "content"], &["type", "content"])?;
    require_type(object, "orderedList")?;
    let start = match object.get("attrs") {
        None => 1,
        Some(value) => {
            let attrs = exact_object(value, &["start"], &["start"])?;
            attrs
                .get("start")
                .and_then(Value::as_u64)
                .filter(|start| (1..=1_000_000).contains(start))
                .ok_or_else(FoundationError::validation_failed)?
        }
    };
    let content = required_array(object, "content")?;
    if content.is_empty() {
        return invalid();
    }
    let canonical = content
        .iter()
        .map(|item| canonical_list_item(item, "listItem", depth + 1, nodes))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("orderedList".to_owned()));
    if start != 1 {
        output.insert(
            "attrs".to_owned(),
            object_with([("start", Value::Number(start.into()))]),
        );
    }
    output.insert("content".to_owned(), Value::Array(canonical));
    Ok(Value::Object(output))
}

fn canonical_list_item(
    value: &Value,
    kind: &str,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    check_depth_and_count(depth, nodes)?;
    let allowed = if kind == "taskItem" {
        &["type", "attrs", "content"][..]
    } else {
        &["type", "content"][..]
    };
    let object = exact_object(value, allowed, allowed)?;
    require_type(object, kind)?;
    let content = required_array(object, "content")?;
    if content.is_empty()
        || content[0]
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            != Some("paragraph")
    {
        return invalid();
    }
    let canonical = content
        .iter()
        .enumerate()
        .map(|(index, child)| {
            if index == 0 {
                canonical_block(child, depth + 1, nodes)
            } else {
                let child_type = child
                    .as_object()
                    .and_then(|object| object.get("type"))
                    .and_then(Value::as_str);
                if !matches!(child_type, Some("bulletList" | "orderedList" | "taskList")) {
                    return invalid();
                }
                canonical_block(child, depth + 1, nodes)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String(kind.to_owned()));
    if kind == "taskItem" {
        let attrs = exact_object(
            object
                .get("attrs")
                .ok_or_else(FoundationError::validation_failed)?,
            &["checked"],
            &["checked"],
        )?;
        let checked = attrs
            .get("checked")
            .and_then(Value::as_bool)
            .ok_or_else(FoundationError::validation_failed)?;
        output.insert(
            "attrs".to_owned(),
            object_with([("checked", Value::Bool(checked))]),
        );
    }
    output.insert("content".to_owned(), Value::Array(canonical));
    Ok(Value::Object(output))
}

fn canonical_inline(
    value: &Value,
    depth: u32,
    nodes: &mut usize,
) -> Result<Value, FoundationError> {
    check_depth_and_count(depth, nodes)?;
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    match object.get("type").and_then(Value::as_str) {
        Some("text") => canonical_text(object),
        Some("imageRef") => canonical_image_ref(object),
        Some("noteLink") => canonical_note_link(object),
        _ => invalid(),
    }
}

fn canonical_text(object: &Map<String, Value>) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "text", "marks"], &["type", "text"])?;
    require_type(object, "text")?;
    let text = non_empty_string(object, "text")?;
    let marks = optional_array(object, "marks")?
        .unwrap_or(&[])
        .iter()
        .map(canonical_mark)
        .collect::<Result<Vec<_>, _>>()?;
    if marks.len() > 2 {
        return invalid();
    }
    let mut seen_bold = false;
    let mut seen_link = false;
    for mark in &marks {
        match mark.get("type").and_then(Value::as_str) {
            Some("bold") if !seen_bold => seen_bold = true,
            Some("link") if !seen_link => seen_link = true,
            _ => return invalid(),
        }
    }
    let mut marks = marks;
    marks.sort_by_key(|mark| usize::from(mark["type"] == "link"));
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("text".to_owned()));
    if !marks.is_empty() {
        output.insert("marks".to_owned(), Value::Array(marks));
    }
    output.insert("text".to_owned(), Value::String(text.to_owned()));
    Ok(Value::Object(output))
}

fn canonical_mark(value: &Value) -> Result<Value, FoundationError> {
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    match object.get("type").and_then(Value::as_str) {
        Some("bold") => {
            exact_map(object, &["type"], &["type"])?;
            Ok(object_with([("type", Value::String("bold".to_owned()))]))
        }
        Some("link") => {
            exact_map(object, &["type", "attrs"], &["type", "attrs"])?;
            let attrs = exact_object(
                object
                    .get("attrs")
                    .ok_or_else(FoundationError::validation_failed)?,
                &["href"],
                &["href"],
            )?;
            let href = non_empty_string(attrs, "href")?;
            if !approved_external_href(href) {
                return invalid();
            }
            Ok(object_with([
                ("type", Value::String("link".to_owned())),
                (
                    "attrs",
                    object_with([("href", Value::String(href.to_owned()))]),
                ),
            ]))
        }
        _ => invalid(),
    }
}

fn approved_external_href(href: &str) -> bool {
    if href.chars().count() > 2048 || href.chars().any(char::is_whitespace) {
        return false;
    }
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => url.host_str().is_some_and(|host| !host.is_empty()),
        "mailto" => !url.path().is_empty(),
        _ => false,
    }
}

fn canonical_image_ref(object: &Map<String, Value>) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "attrs"], &["type", "attrs"])?;
    let attrs = exact_object(
        object
            .get("attrs")
            .ok_or_else(FoundationError::validation_failed)?,
        &["asset_id", "display_width"],
        &["asset_id"],
    )?;
    let asset_id = non_empty_string(attrs, "asset_id")?;
    validate_note_id(asset_id)?;
    let mut canonical_attrs = Map::new();
    canonical_attrs.insert("asset_id".to_owned(), Value::String(asset_id.to_owned()));
    if let Some(width) = attrs.get("display_width") {
        let width = width
            .as_u64()
            .filter(|width| (48..=4096).contains(width))
            .ok_or_else(FoundationError::validation_failed)?;
        canonical_attrs.insert("display_width".to_owned(), Value::Number(width.into()));
    }
    Ok(object_with([
        ("type", Value::String("imageRef".to_owned())),
        ("attrs", Value::Object(canonical_attrs)),
    ]))
}

fn canonical_note_link(object: &Map<String, Value>) -> Result<Value, FoundationError> {
    exact_map(object, &["type", "attrs"], &["type", "attrs"])?;
    let attrs = exact_object(
        object
            .get("attrs")
            .ok_or_else(FoundationError::validation_failed)?,
        &["target_note_id", "display_text"],
        &["target_note_id"],
    )?;
    let target_note_id = non_empty_string(attrs, "target_note_id")?;
    validate_note_id(target_note_id)?;
    let mut canonical_attrs = Map::new();
    canonical_attrs.insert(
        "target_note_id".to_owned(),
        Value::String(target_note_id.to_owned()),
    );
    if let Some(display_text) = attrs.get("display_text") {
        let display_text = display_text
            .as_str()
            .filter(|text| !text.is_empty() && text.chars().count() <= 500)
            .ok_or_else(FoundationError::validation_failed)?;
        canonical_attrs.insert(
            "display_text".to_owned(),
            Value::String(display_text.to_owned()),
        );
    }
    Ok(object_with([
        ("type", Value::String("noteLink".to_owned())),
        ("attrs", Value::Object(canonical_attrs)),
    ]))
}

fn document_text(document: &Value) -> Result<String, FoundationError> {
    required_array(
        document
            .as_object()
            .ok_or_else(FoundationError::validation_failed)?,
        "content",
    )?
    .iter()
    .map(block_text)
    .collect::<Result<Vec<_>, _>>()
    .map(|blocks| blocks.join("\n"))
}

fn block_text(value: &Value) -> Result<String, FoundationError> {
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    match object.get("type").and_then(Value::as_str) {
        Some("paragraph" | "heading" | "codeBlock") => inline_texts(object),
        Some("bulletList" | "orderedList") => required_array(object, "content")?
            .iter()
            .map(list_item_text)
            .collect::<Result<Vec<_>, _>>()
            .map(|items| items.join("\n")),
        Some("taskList") => required_array(object, "content")?
            .iter()
            .map(task_item_text)
            .collect::<Result<Vec<_>, _>>()
            .map(|items| items.join("\n")),
        _ => invalid(),
    }
}

fn list_item_text(value: &Value) -> Result<String, FoundationError> {
    required_array(
        value
            .as_object()
            .ok_or_else(FoundationError::validation_failed)?,
        "content",
    )?
    .iter()
    .map(block_text)
    .collect::<Result<Vec<_>, _>>()
    .map(|parts| parts.join("\n"))
}

fn task_item_text(value: &Value) -> Result<String, FoundationError> {
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    let checked = object
        .get("attrs")
        .and_then(Value::as_object)
        .and_then(|attrs| attrs.get("checked"))
        .and_then(Value::as_bool)
        .ok_or_else(FoundationError::validation_failed)?;
    Ok(format!(
        "[{}] {}",
        if checked { "x" } else { " " },
        required_array(object, "content")?
            .iter()
            .map(block_text)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")
    ))
}

fn inline_texts(object: &Map<String, Value>) -> Result<String, FoundationError> {
    optional_array(object, "content")?
        .unwrap_or(&[])
        .iter()
        .map(|node| {
            let child = node
                .as_object()
                .ok_or_else(FoundationError::validation_failed)?;
            match child.get("type").and_then(Value::as_str) {
                Some("text") => Ok(child["text"].as_str().unwrap_or_default().to_owned()),
                Some("imageRef") => Ok(String::new()),
                Some("noteLink") => {
                    let attrs = child["attrs"]
                        .as_object()
                        .ok_or_else(FoundationError::validation_failed)?;
                    Ok(attrs
                        .get("display_text")
                        .and_then(Value::as_str)
                        .or_else(|| attrs.get("target_note_id").and_then(Value::as_str))
                        .unwrap_or_default()
                        .to_owned())
                }
                _ => invalid(),
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.concat())
}

pub(crate) fn calculate_content_hash(note_date: &str, title: &str, body_json: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(CONTENT_HASH_DOMAIN);
    hash.update([0]);
    hash.update(note_date.as_bytes());
    hash.update([0]);
    hash.update(title.as_bytes());
    hash.update([0]);
    hash.update(body_json.as_bytes());
    hash.finalize().into()
}

fn exact_object<'a>(
    value: &'a Value,
    allowed: &[&str],
    required: &[&str],
) -> Result<&'a Map<String, Value>, FoundationError> {
    let object = value
        .as_object()
        .ok_or_else(FoundationError::validation_failed)?;
    exact_map(object, allowed, required)?;
    Ok(object)
}

fn exact_map(
    object: &Map<String, Value>,
    allowed: &[&str],
    required: &[&str],
) -> Result<(), FoundationError> {
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || required.iter().any(|key| !object.contains_key(*key))
    {
        return invalid();
    }
    Ok(())
}

fn require_type(object: &Map<String, Value>, expected: &str) -> Result<(), FoundationError> {
    if object.get("type").and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        invalid()
    }
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a [Value], FoundationError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(FoundationError::validation_failed)
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a [Value]>, FoundationError> {
    object
        .get(key)
        .map(|value| {
            value
                .as_array()
                .map(Vec::as_slice)
                .ok_or_else(FoundationError::validation_failed)
        })
        .transpose()
}

fn non_empty_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, FoundationError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(FoundationError::validation_failed)
}

fn object_with<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    let mut output = Map::new();
    for (key, value) in pairs {
        output.insert(key.to_owned(), value);
    }
    Value::Object(output)
}

fn check_depth_and_count(depth: u32, nodes: &mut usize) -> Result<(), FoundationError> {
    *nodes = nodes.saturating_add(1);
    if depth > MAX_BODY_DEPTH || *nodes > MAX_BODY_NODES {
        invalid()
    } else {
        Ok(())
    }
}

fn parse_digits(bytes: &[u8]) -> Result<u32, FoundationError> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        if byte.is_ascii_digit() {
            Ok(value * 10 + u32::from(byte - b'0'))
        } else {
            Err(FoundationError::validation_failed())
        }
    })
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn hex_value(byte: u8) -> Result<u8, FoundationError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(FoundationError::validation_failed()),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn invalid<T>() -> Result<T, FoundationError> {
    Err(FoundationError::validation_failed())
}
