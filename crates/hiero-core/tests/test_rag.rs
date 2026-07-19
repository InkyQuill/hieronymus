use std::{collections::BTreeMap, fs, io::Write, path::Path};

use hiero_core::{
    db::connect_url,
    rag::{
        ImportOptions, MAX_RAG_CHUNK_CHARS, MAX_RAG_METADATA_BYTES, RagError, RagStore,
        SearchOptions, SourceType, load_rag_file, normalize_rag_source, split_chunk_text,
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
fn chunking_prefers_sentences_and_whitespace_only_blank_lines_split_paragraphs() {
    let first = format!("{}.", "A".repeat(700));
    let second = format!("{}!", "B".repeat(700));
    assert_eq!(
        split_chunk_text(&format!("{first} {second}")),
        vec![first, second]
    );
    let dir = tempdir().unwrap();
    let path = dir.path().join("spaces.txt");
    fs::write(&path, "First paragraph.\n \t\nSecond paragraph.").unwrap();
    assert_eq!(
        load_rag_file(&path, SourceType::Auto).unwrap().chunks.len(),
        2
    );
}

#[test]
fn markdown_long_fences_and_indented_code_cannot_create_headings() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("fences.md");
    fs::write(&path, "# Real\n\n````rust\n# code\n```\n# still code\n````\n\n    # indented\n    value\n\n## After\n\nDone.").unwrap();
    let parsed = load_rag_file(&path, SourceType::Auto).unwrap();
    assert_eq!(parsed.chunks[0].location, "Real paragraph 1");
    assert!(parsed.chunks[0].text.contains("# still code"));
    assert_eq!(parsed.chunks[1].location, "Real paragraph 2");
    assert_eq!(parsed.chunks[2].location, "Real > After paragraph 3");
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
fn oversized_glossary_metadata_is_rejected_before_split_amplification() {
    let dir = tempdir().unwrap();
    let oversized = "x".repeat(MAX_RAG_METADATA_BYTES + 1);
    let fixtures = [
        (
            "large.json",
            serde_json::to_vec(&serde_json::json!([{"note": oversized.clone()}])).unwrap(),
        ),
        (
            "large.csv",
            format!("source,note\nSense,{oversized}\n").into_bytes(),
        ),
    ];
    for (name, bytes) in fixtures {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            load_rag_file(&path, SourceType::Auto),
            Err(RagError::ResourceLimit {
                resource: "chunk metadata bytes",
                limit: MAX_RAG_METADATA_BYTES,
            })
        ));
    }
}

#[test]
fn overlong_unicode_word_splits_exactly_without_losing_scalars() {
    let input = "界".repeat(MAX_RAG_CHUNK_CHARS * 4 + 17);
    let chunks = split_chunk_text(&input);
    assert_eq!(chunks.len(), 5);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.chars().count())
            .collect::<Vec<_>>(),
        vec![
            MAX_RAG_CHUNK_CHARS,
            MAX_RAG_CHUNK_CHARS,
            MAX_RAG_CHUNK_CHARS,
            MAX_RAG_CHUNK_CHARS,
            17
        ]
    );
    assert_eq!(chunks.concat(), input);
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
    zip.write_all(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Important detail.</w:t></w:r></w:p><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t><w:tab/><w:br/></w:r><w:hyperlink r:id="rId1"><w:r><w:t>Docs</w:t></w:r></w:hyperlink></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Sense</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Сенс</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#.as_bytes()).unwrap();
    zip.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="https://example.test"/></Relationships>"#).unwrap();
    zip.finish().unwrap();
    let normalized = normalize_rag_source(&docx, &managed).unwrap();
    let output = fs::read_to_string(normalized.path).unwrap();
    assert!(output.contains("# Important detail."));
    assert!(output.contains("**Bold**"));
    assert!(output.contains("[Docs](https://example.test)"));
    assert!(output.contains("Sense"));
    assert!(output.contains("Сенс"));
}

#[test]
fn html_conversion_preserves_inline_links_emphasis_lists_and_tables_once() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("rich.html");
    fs::write(&source, "<h1>Guide</h1><p>Use <strong>Sense</strong> with <em>care</em> and <a href='https://example.test'>docs</a>.</p><ul><li>One</li><li>Two</li></ul><table><tr><th>Name</th><th>Value</th></tr><tr><td>Sense</td><td>Сенс</td></tr></table>").unwrap();
    let output = fs::read_to_string(
        normalize_rag_source(&source, &dir.path().join("managed"))
            .unwrap()
            .path,
    )
    .unwrap();
    assert!(output.contains("**Sense**"));
    assert!(output.contains("*care*"));
    assert!(output.contains("[docs](https://example.test)"));
    assert_eq!(output.matches("- One").count(), 1);
    assert!(output.contains("| Name | Value |"));
}

#[test]
fn managed_output_rejects_symlinks_and_concurrent_publication_is_complete() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.html");
    fs::write(&source, "<p>Complete content.</p>").unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let link = dir.path().join("managed-link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    #[cfg(unix)]
    assert!(matches!(
        normalize_rag_source(&source, &link),
        Err(RagError::UnsafeManagedPath(_))
    ));

    let managed = dir.path().join("managed");
    let paths: Vec<_> = std::thread::scope(|scope| {
        (0..8)
            .map(|_| scope.spawn(|| normalize_rag_source(&source, &managed).unwrap().path))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert!(paths.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(
        fs::read_to_string(&paths[0]).unwrap(),
        "Complete content.\n"
    );
}

#[test]
fn corrupt_existing_managed_artifact_is_never_authoritative() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.html");
    let managed = dir.path().join("managed");
    fs::write(&source, "<p>Expected content.</p>").unwrap();
    let normalized = normalize_rag_source(&source, &managed).unwrap();
    fs::write(&normalized.path, "corrupt\n").unwrap();

    assert!(matches!(
        normalize_rag_source(&source, &managed),
        Err(RagError::ManagedArtifactMismatch(path)) if path == normalized.path
    ));
    assert_eq!(fs::read_to_string(normalized.path).unwrap(), "corrupt\n");
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

#[test]
fn chunk_count_limit_rejects_before_materializing_all_text_chunks() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("too-many.txt");
    fs::write(&path, "x\n\n".repeat(hiero_core::rag::MAX_RAG_CHUNKS + 1)).unwrap();
    assert!(matches!(
        load_rag_file(&path, SourceType::Auto),
        Err(RagError::ResourceLimit {
            resource: "chunks",
            ..
        })
    ));
}

#[test]
fn markdown_chunk_limit_uses_real_fence_state_without_false_rejection() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("many-code-headings.md");
    let mut input = String::from("# Real\n\n```text\n");
    for _ in 0..=hiero_core::rag::MAX_RAG_CHUNKS {
        input.push_str("# x\n");
    }
    input.push_str("```\n");
    fs::write(&path, input).unwrap();

    let parsed = load_rag_file(&path, SourceType::Auto).unwrap();
    assert!(parsed.chunks.len() < hiero_core::rag::MAX_RAG_CHUNKS);
    assert!(
        parsed
            .chunks
            .iter()
            .all(|chunk| chunk.location.starts_with("Real paragraph"))
    );
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
async fn canonical_default_identity_deduplicates_relative_dot_absolute_and_symlink_aliases() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let source = dir.path().join("chapter.txt");
    fs::write(&source, "Canonical Sense.").unwrap();
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.txt");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let first = store
            .import_file("oso", &source, ImportOptions::default())
            .await
            .unwrap();
        let second = store
            .import_file("oso", &alias, ImportOptions::default())
            .await
            .unwrap();
        assert_eq!(first.source_id, second.source_id);
        assert!(second.skipped);
        assert_eq!(
            first.source.source_ref,
            fs::canonicalize(&source).unwrap().to_string_lossy()
        );
    }
}

#[tokio::test]
async fn changed_indexed_source_clears_old_semantic_state_and_enqueues_new_job() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("chapter.txt");
    fs::write(&path, "Old Sense.").unwrap();
    store
        .import_file("oso", &path, options("chapter.txt"))
        .await
        .unwrap();
    let old_id: i64 = sqlx::query_scalar("SELECT id FROM rag_chunks")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO semantic_chunk_state(chunk_id, checksum, generation_id, indexed_at) VALUES (?, 'old', 'g1', '2026-07-19T00:00:00Z')").bind(old_id).execute(&pool).await.unwrap();
    fs::write(&path, "New Moonstone.").unwrap();
    store
        .import_file("oso", &path, options("chapter.txt"))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_chunk_state")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM semantic_index_jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
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
async fn tag_normalization_is_trimmed_deduplicated_and_unicode_casefolded_for_boosts() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    for name in ["plain", "tagged"] {
        fs::write(dir.path().join(format!("{name}.txt")), "Sense evidence.").unwrap();
    }
    store
        .import_file("oso", &dir.path().join("plain.txt"), options("plain"))
        .await
        .unwrap();
    let mut tagged = options("tagged");
    tagged.story_scopes = vec!["  STRASSE  ".into(), "strasse".into(), "".into()];
    tagged.semantic_tags = vec!["  SKİLL  ".into(), "ski̇ll".into()];
    store
        .import_file("oso", &dir.path().join("tagged.txt"), tagged)
        .await
        .unwrap();
    let hits = store
        .search(
            "oso",
            "Sense",
            SearchOptions {
                story_scopes: vec!["Straße".into()],
                semantic_tags: vec!["SKİLL".into()],
                limit: 2,
                ..SearchOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(hits[0].chunk.source_ref, "tagged");
    assert_eq!(hits[0].chunk.story_scopes, vec!["strasse"]);
    assert_eq!(hits[0].chunk.semantic_tags, vec!["ski̇ll"]);
}

#[tokio::test]
async fn unchanged_receipt_matches_authoritative_metadata_and_markdown_reason_is_exact() {
    let pool = pool().await;
    let store = RagStore::new(&pool);
    let dir = tempdir().unwrap();
    let path = dir.path().join("chapter.md");
    fs::write(&path, "# Sense\n\nEvidence.").unwrap();
    let first = store
        .import_file("oso", &path, options("chapter.md"))
        .await
        .unwrap();
    sqlx::query("UPDATE rag_sources SET metadata_json = '{\"stale\":true}' WHERE id = ?")
        .bind(first.source_id)
        .execute(&pool)
        .await
        .unwrap();
    let second = store
        .import_file("oso", &path, options("chapter.md"))
        .await
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT metadata_json FROM rag_sources WHERE id = ?")
        .bind(first.source_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        serde_json::to_value(&second.source.metadata).unwrap()
    );
    assert_eq!(
        store
            .search("oso", "Evidence", SearchOptions::default())
            .await
            .unwrap()[0]
            .reason,
        "rag markdown section match"
    );
}

#[test]
fn blank_delimited_rows_are_skipped() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("blank.csv");
    fs::write(&path, "source,target\n,\nSense,Сенс\n").unwrap();
    assert_eq!(
        load_rag_file(&path, SourceType::Auto).unwrap().chunks.len(),
        1
    );
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
