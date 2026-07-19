use std::{collections::BTreeMap, fs, io::Write, path::Path};

use hiero_core::{
    db::connect_url,
    rag::{
        ImportOptions, MAX_RAG_CHUNK_CHARS, RagError, RagStore, SearchOptions, SourceType,
        load_rag_file, normalize_rag_source, split_chunk_text,
    },
};
use sqlx::Row;
use tempfile::tempdir;
use zip::{ZipWriter, write::SimpleFileOptions};

async fn pool() -> sqlx::SqlitePool {
    let url = format!(
        "sqlite:file:rag-{}?mode=memory&cache=shared",
        uuid::Uuid::new_v4()
    );
    let pool = connect_url(&url).await.unwrap();
    sqlx::query(
        "INSERT INTO series(slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("oso")
    .bind("Only Sense Online")
    .bind("ja")
    .bind("ru")
    .bind("2026-07-19T00:00:00Z")
    .bind("2026-07-19T00:00:00Z")
    .execute(&pool)
    .await
    .unwrap();
    pool
}

fn options(source_ref: &str) -> ImportOptions {
    ImportOptions {
        source_ref: Some(source_ref.to_owned()),
        ..ImportOptions::default()
    }
}

#[test]
fn text_markdown_and_structured_glossaries_parse_deterministically() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rag");
    let text = load_rag_file(&root.join("chapter.txt"), SourceType::Auto).unwrap();
    let markdown = load_rag_file(&root.join("chapter.md"), SourceType::Auto).unwrap();
    let csv = load_rag_file(&root.join("glossary.csv"), SourceType::Auto).unwrap();
    let yaml = load_rag_file(&root.join("glossary.yaml"), SourceType::Auto).unwrap();
    let json = load_rag_file(&root.join("glossary.json"), SourceType::Auto).unwrap();

    assert_eq!(text.chunks.len(), 2);
    assert_eq!(markdown.chunks[0].location, "Chapter > Terms paragraph 1");
    assert_eq!(csv.chunks[0].metadata["source"], "Sense");
    assert_eq!(yaml.chunks[0].metadata["key"], "Sense");
    assert_eq!(
        json.chunks[0].text,
        "source: Cooking Talent\ntarget: Кулинарный талант"
    );
}

#[test]
fn markdown_fenced_code_does_not_create_heading_locations() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("code.md");
    fs::write(
        &path,
        "# Real\n\n```md\n# Not a heading\n```\n\nAfter fence.",
    )
    .unwrap();
    let parsed = load_rag_file(&path, SourceType::Auto).unwrap();
    assert_eq!(parsed.chunks.len(), 2);
    assert_eq!(parsed.chunks[0].location, "Real paragraph 1");
    assert!(parsed.chunks[0].text.contains("# Not a heading"));
    assert_eq!(parsed.chunks[1].location, "Real paragraph 2");
}

#[test]
fn chunking_is_unicode_safe_and_never_exceeds_limit() {
    let input = "🦀".repeat(MAX_RAG_CHUNK_CHARS + 7);
    let chunks = split_chunk_text(&input);
    assert_eq!(chunks.len(), 2);
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_RAG_CHUNK_CHARS)
    );
    assert_eq!(chunks.concat(), input);
}

#[test]
fn oversized_glossary_entries_are_split_without_losing_metadata() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("large.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!([{"source": "Sense", "note": "界".repeat(MAX_RAG_CHUNK_CHARS + 25)}])).unwrap(),
    )
    .unwrap();
    let parsed = load_rag_file(&path, SourceType::Auto).unwrap();
    assert!(parsed.chunks.len() > 1);
    assert!(
        parsed
            .chunks
            .iter()
            .all(|chunk| chunk.text.chars().count() <= MAX_RAG_CHUNK_CHARS)
    );
    assert!(
        parsed
            .chunks
            .iter()
            .all(|chunk| chunk.metadata["source"] == "Sense")
    );
}

#[test]
fn unsupported_and_broken_sources_return_typed_errors() {
    let dir = tempdir().unwrap();
    let epub = dir.path().join("book.epub");
    fs::write(&epub, b"epub").unwrap();
    assert!(matches!(
        normalize_rag_source(&epub, dir.path()),
        Err(RagError::UnsupportedEpub)
    ));

    let broken = dir.path().join("bad.json");
    fs::write(&broken, b"{broken").unwrap();
    assert!(matches!(
        load_rag_file(&broken, SourceType::Auto),
        Err(RagError::Parse { .. })
    ));
}

#[test]
fn html_and_docx_normalize_to_managed_markdown() {
    let dir = tempdir().unwrap();
    let managed = dir.path().join("managed");
    let html = dir.path().join("chapter.html");
    fs::write(&html, "<h1>Chapter</h1><p>Important detail.</p>").unwrap();
    let normalized = normalize_rag_source(&html, &managed).unwrap();
    assert_eq!(normalized.source_type, SourceType::Markdown);
    assert_eq!(
        fs::read_to_string(&normalized.path).unwrap(),
        "# Chapter\n\nImportant detail.\n"
    );

    let docx = dir.path().join("chapter.docx");
    let file = fs::File::create(&docx).unwrap();
    let mut zip = ZipWriter::new(file);
    zip.start_file("word/document.xml", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Important detail.</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
    zip.finish().unwrap();
    let normalized = normalize_rag_source(&docx, &managed).unwrap();
    assert!(
        fs::read_to_string(normalized.path)
            .unwrap()
            .contains("Important detail.")
    );
}

#[test]
fn real_pdf_fixture_normalizes_and_broken_pdf_is_typed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rag");
    let dir = tempdir().unwrap();
    let normalized = normalize_rag_source(&root.join("sample.pdf"), dir.path()).unwrap();
    assert!(
        fs::read_to_string(normalized.path)
            .unwrap()
            .contains("Important detail.")
    );

    let broken = dir.path().join("broken.pdf");
    fs::write(&broken, b"not a pdf").unwrap();
    assert!(matches!(
        normalize_rag_source(&broken, dir.path()),
        Err(RagError::Parse { .. })
    ));
    assert!(matches!(
        normalize_rag_source(&root.join("blank.pdf"), dir.path()),
        Err(RagError::NoExtractableText(_))
    ));
}

#[test]
fn source_file_size_limit_is_enforced_before_parsing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("oversized.txt");
    let file = fs::File::create(&path).unwrap();
    file.set_len(hiero_core::rag::MAX_RAG_FILE_BYTES + 1)
        .unwrap();
    assert!(matches!(
        load_rag_file(&path, SourceType::Auto),
        Err(RagError::ResourceLimit {
            resource: "source bytes",
            ..
        })
    ));
}

#[tokio::test]
async fn import_search_reimport_and_tag_refresh_are_atomic() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("chapter.txt");
    fs::write(&path, "Sense menu note.\n\nCooking Talent appears here.").unwrap();
    let mut opts = options("chapter-5.txt");
    opts.language_tags = vec!["ja".into(), "ru".into(), "ja".into()];
    opts.story_scopes = vec!["book:5/chapter:5".into()];
    opts.semantic_tags = vec!["chapter:source".into()];

    let first = store.import_file("oso", &path, opts.clone()).await.unwrap();
    let hits = store
        .search("oso", "Cooking Talent", SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(first.chunks_created, 2);
    assert!(!first.skipped);
    assert_eq!(hits[0].chunk.source_ref, "chapter-5.txt");
    assert_eq!(hits[0].chunk.language_tags, vec!["ja", "ru"]);
    assert_eq!(hits[0].source, hiero_core::recall::MemorySource::Rag);

    opts.language_tags = vec!["ru".into()];
    opts.semantic_tags = vec!["approved".into()];
    let second = store.import_file("oso", &path, opts).await.unwrap();
    assert!(second.skipped);
    let refreshed = store
        .search("oso", "Sense", SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(refreshed[0].chunk.semantic_tags, vec!["approved"]);

    fs::write(&path, "New Moonstone evidence.").unwrap();
    store
        .import_file("oso", &path, options("chapter-5.txt"))
        .await
        .unwrap();
    assert!(
        store
            .search("oso", "Sense", SearchOptions::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .search("oso", "Moonstone", SearchOptions::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_reimport_preserves_last_valid_source_and_chunks() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("glossary.json");
    fs::write(&path, r#"[{"source":"Sense","target":"Сенс"}]"#).unwrap();
    store
        .import_file("oso", &path, options("glossary.json"))
        .await
        .unwrap();
    fs::write(&path, "{broken").unwrap();
    assert!(
        store
            .import_file("oso", &path, options("glossary.json"))
            .await
            .is_err()
    );
    let hits = store
        .search("oso", "Sense", SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.metadata["source"], "Sense");
}

#[tokio::test]
async fn database_failure_during_changed_reimport_rolls_back_source_replacement() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("chapter.txt");
    fs::write(&path, "Last valid Sense evidence.").unwrap();
    store
        .import_file("oso", &path, options("chapter.txt"))
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_blocked_rag BEFORE INSERT ON rag_chunks WHEN new.text LIKE '%blocked%' BEGIN SELECT RAISE(ABORT, 'injected chunk failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    fs::write(&path, "blocked replacement").unwrap();
    assert!(
        store
            .import_file("oso", &path, options("chapter.txt"))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .search("oso", "Last valid Sense", SearchOptions::default())
            .await
            .unwrap()[0]
            .chunk
            .text,
        "Last valid Sense evidence."
    );
}

#[tokio::test]
async fn search_is_stable_bounded_and_applies_metadata_boosts() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    for index in 0..51 {
        let path = dir.path().join(format!("source-{index}.txt"));
        fs::write(&path, "Sense shared evidence.").unwrap();
        let mut opts = options(&format!("source-{index}.txt"));
        if index == 50 {
            opts.semantic_tags = vec!["skill:name".into()];
        }
        store.import_file("oso", &path, opts).await.unwrap();
    }
    let hits = store
        .search(
            "oso",
            "Sense shared evidence",
            SearchOptions {
                limit: 1_000,
                semantic_tags: vec!["skill:name".into()],
                ..SearchOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 50);
    assert_eq!(hits[0].chunk.source_ref, "source-50.txt");
    assert!(hits[0].score > hits[1].score);
}

#[tokio::test]
async fn import_persists_one_pending_semantic_job_for_changed_content_only() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("chapter.txt");
    fs::write(&path, "Sense.").unwrap();
    store
        .import_file("oso", &path, options("chapter.txt"))
        .await
        .unwrap();
    store
        .import_file("oso", &path, options("chapter.txt"))
        .await
        .unwrap();
    let count: i64 = sqlx::query("SELECT count(*) AS count FROM semantic_index_jobs")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("count");
    assert_eq!(count, 1);
}

#[test]
fn public_models_have_flat_serialization_contracts() {
    let options = SearchOptions::default();
    assert_eq!(options.limit, 10);
    assert_eq!(options.retrieval_mode.to_string(), "lexical");
    let metadata: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    assert!(metadata.is_empty());
}
