//! Behavior ported from `tests/test_rag_store.py`: checksum-aware RAG
//! import, series-isolated FTS search with typed metadata boosts, and
//! failure-safe re-import.

use std::path::Path;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::rag::{RagError, RagImport, RagStore};
use hieronymus::registry::Registry;

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    series_slug: String,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();
    let series = registry
        .create_series("only-sense-online", "Only Sense Online", "ja", "ru", None)
        .unwrap();
    Fixture {
        root,
        config,
        series_slug: series.slug,
    }
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

#[test]
fn import_file_indexes_chunks_and_searches() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.txt");
    write(&path, "Sense menu note.\n\nCooking Talent appears here.");

    let store = RagStore::open(&fixture.config).unwrap();
    let result = store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new()
                .source_ref("chapter-5.txt")
                .language_tags(["ja", "ru"])
                .story_scopes(["book:5/chapter:5"])
                .semantic_tags(["chapter:source"]),
        )
        .unwrap();
    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Cooking Talent", 5, &[], &[], &[])
        .unwrap();

    assert_eq!(result.chunk_count, 2);
    assert!(!result.skipped);
    assert_eq!(
        hits.iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["Cooking Talent appears here."]
    );
    assert_eq!(hits[0].reason, "rag project text match");
    assert_eq!(hits[0].chunk.source_ref, "chapter-5.txt");
    assert_eq!(hits[0].chunk.language_tags, ["ja", "ru"]);
    assert_eq!(hits[0].chunk.story_scopes, ["book:5/chapter:5"]);
    assert_eq!(hits[0].chunk.semantic_tags, ["chapter:source"]);
}

#[test]
fn import_html_records_normalized_markdown_provenance() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.html");
    write(&path, "<h1>Sense</h1><p>Sense menu note.</p>");

    let result = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();

    assert_eq!(result.normalized_format, "markdown");
    assert!(result.normalized_path.ends_with(".md"));
    assert_eq!(
        result.source.metadata.get("original_path"),
        Some(&serde_json::Value::String(path.display().to_string()))
    );
    assert_eq!(
        result.source.metadata.get("normalized_path"),
        Some(&serde_json::Value::String(result.normalized_path.clone()))
    );
}

#[test]
fn import_file_skips_unchanged_checksum() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.txt");
    write(&path, "Sense menu note.");
    let store = RagStore::open(&fixture.config).unwrap();

    let first = store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.txt"),
        )
        .unwrap();
    let second = store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.txt"),
        )
        .unwrap();

    assert!(!first.skipped);
    assert!(second.skipped);
    assert_eq!(second.chunk_count, first.chunk_count);
}

#[test]
fn same_checksum_with_changed_format_reindexes_source() {
    let fixture = fixture();
    let text_path = fixture.root.path().join("source.txt");
    let markdown_path = fixture.root.path().join("source.md");
    let content = "# Sense\n\nSense menu note.";
    write(&text_path, content);
    write(&markdown_path, content);
    let store = RagStore::open(&fixture.config).unwrap();

    let first = store
        .import_file(
            &fixture.series_slug,
            &text_path,
            &RagImport::new().source_ref("source"),
        )
        .unwrap();
    let second = store
        .import_file(
            &fixture.series_slug,
            &markdown_path,
            &RagImport::new().source_ref("source"),
        )
        .unwrap();
    let hit = &RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense menu", 1, &[], &[], &[])
        .unwrap()[0];

    assert_eq!(first.source.content_type, "txt");
    assert!(!second.skipped);
    assert_eq!(second.source.source_type, "markdown");
    assert_eq!(second.source.content_type, "md");
    assert_eq!(hit.chunk.chunk_kind, "markdown_section");
    assert_eq!(hit.chunk.location, "Sense paragraph 1");
}

#[test]
fn reimport_with_same_checksum_refreshes_chunk_tags() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.txt");
    write(&path, "Sense menu note.");
    let store = RagStore::open(&fixture.config).unwrap();
    store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new()
                .source_ref("chapter.txt")
                .language_tags(["ja"])
                .story_scopes(["book:5/chapter:1"])
                .semantic_tags(["draft"]),
        )
        .unwrap();

    let result = store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new()
                .source_ref("chapter.txt")
                .language_tags(["ru"])
                .story_scopes(["book:5/chapter:2"])
                .semantic_tags(["approved"]),
        )
        .unwrap();
    let hit = &RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense", 5, &[], &[], &[])
        .unwrap()[0];

    assert!(result.skipped);
    assert_eq!(hit.chunk.language_tags, ["ru"]);
    assert_eq!(hit.chunk.story_scopes, ["book:5/chapter:2"]);
    assert_eq!(hit.chunk.semantic_tags, ["approved"]);
}

#[test]
fn changed_import_replaces_old_chunks() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.txt");
    let store = RagStore::open(&fixture.config).unwrap();
    write(&path, "Old Sense note.");
    store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.txt"),
        )
        .unwrap();

    write(&path, "New Cooking Talent note.");
    store
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.txt"),
        )
        .unwrap();

    assert!(
        RagStore::open(&fixture.config)
            .unwrap()
            .search(&fixture.series_slug, "Old", 5, &[], &[], &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        RagStore::open(&fixture.config)
            .unwrap()
            .search(&fixture.series_slug, "Cooking Talent", 5, &[], &[], &[])
            .unwrap()
            .iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["New Cooking Talent note."]
    );
}

#[test]
fn failed_changed_import_preserves_old_chunks() {
    let fixture = fixture();
    let good_path = fixture.root.path().join("glossary.json");
    let store = RagStore::open(&fixture.config).unwrap();
    write(&good_path, r#"[{"source": "Sense", "target": "Сенс"}]"#);
    store
        .import_file(
            &fixture.series_slug,
            &good_path,
            &RagImport::new().source_ref("glossary.json"),
        )
        .unwrap();

    write(&good_path, "{broken json");
    let error = store
        .import_file(
            &fixture.series_slug,
            &good_path,
            &RagImport::new().source_ref("glossary.json"),
        )
        .unwrap_err();
    assert!(matches!(error, RagError::InvalidSource(_)));
    assert!(error.to_string().starts_with("invalid RAG source:"));

    assert_eq!(
        RagStore::open(&fixture.config)
            .unwrap()
            .search(&fixture.series_slug, "Sense", 5, &[], &[], &[])
            .unwrap()
            .iter()
            .map(|hit| hit.chunk.metadata.get("source").cloned())
            .collect::<Vec<_>>(),
        [Some(serde_json::Value::String("Sense".to_string()))]
    );
}

#[test]
fn glossary_hits_get_glossary_reason() {
    let fixture = fixture();
    let path = fixture.root.path().join("glossary.csv");
    write(&path, "source,target\nSense,Сенс\n");

    RagStore::open(&fixture.config)
        .unwrap()
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("glossary.csv"),
        )
        .unwrap();
    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense", 5, &[], &[], &[])
        .unwrap();

    assert_eq!(hits[0].reason, "rag glossary match");
    assert_eq!(hits[0].chunk.chunk_kind, "glossary_entry");
    assert!(hits[0].score > 0.0);
}

#[test]
fn search_boosts_matching_rag_chunk_metadata() {
    let fixture = fixture();
    let store = RagStore::open(&fixture.config).unwrap();
    let general_path = fixture.root.path().join("general.txt");
    let scoped_path = fixture.root.path().join("scoped.txt");
    write(&general_path, "Sense shared evidence.");
    write(&scoped_path, "Sense shared evidence.");
    store
        .import_file(
            &fixture.series_slug,
            &general_path,
            &RagImport::new().source_ref("general.txt"),
        )
        .unwrap();
    store
        .import_file(
            &fixture.series_slug,
            &scoped_path,
            &RagImport::new()
                .source_ref("scoped.txt")
                .language_tags(["ru"])
                .story_scopes(["book:5/chapter:5"])
                .semantic_tags(["skill:name"]),
        )
        .unwrap();

    let hits = store
        .search(
            &fixture.series_slug,
            "Sense shared evidence",
            2,
            &["ru".to_string()],
            &["book:5/chapter:5".to_string()],
            &["skill:name".to_string()],
        )
        .unwrap();

    assert_eq!(
        hits.iter()
            .map(|hit| hit.chunk.source_ref.as_str())
            .collect::<Vec<_>>(),
        ["scoped.txt", "general.txt"]
    );
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn search_applies_metadata_boost_before_candidate_truncation() {
    let fixture = fixture();
    let store = RagStore::open(&fixture.config).unwrap();
    for index in 0..51usize {
        let path = fixture.root.path().join(format!("source-{index}.txt"));
        write(&path, "Sense shared evidence.");
        let mut import = RagImport::new().source_ref(path.file_name().unwrap().to_string_lossy());
        if index == 50 {
            import = import.semantic_tags(["skill:name"]);
        }
        store
            .import_file(&fixture.series_slug, &path, &import)
            .unwrap();
    }

    let hits = store
        .search(
            &fixture.series_slug,
            "Sense shared evidence",
            50,
            &[],
            &[],
            &["skill:name".to_string()],
        )
        .unwrap();

    assert_eq!(hits[0].chunk.source_ref, "source-50.txt");
}

#[test]
fn default_source_ref_keeps_same_basename_paths_distinct() {
    let fixture = fixture();
    let first_path = fixture.root.path().join("first").join("chapter.txt");
    let second_path = fixture.root.path().join("second").join("chapter.txt");
    write(&first_path, "Alpha Sense note.");
    write(&second_path, "Beta Cooking Talent note.");
    let store = RagStore::open(&fixture.config).unwrap();

    let first = store
        .import_file(&fixture.series_slug, &first_path, &RagImport::new())
        .unwrap();
    let second = store
        .import_file(&fixture.series_slug, &second_path, &RagImport::new())
        .unwrap();

    assert_eq!(first.source.source_ref, first_path.display().to_string());
    assert_eq!(second.source.source_ref, second_path.display().to_string());
    assert_eq!(
        store
            .search(&fixture.series_slug, "Alpha", 5, &[], &[], &[])
            .unwrap()
            .iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["Alpha Sense note."]
    );
    assert_eq!(
        store
            .search(&fixture.series_slug, "Beta", 5, &[], &[], &[])
            .unwrap()
            .iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["Beta Cooking Talent note."]
    );
}

#[test]
fn search_caps_large_limit_at_fifty() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.txt");
    let content = (0..60)
        .map(|index| format!("Sense indexed note {index}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    write(&path, &content);
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();

    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense", 1000, &[], &[], &[])
        .unwrap();

    assert_eq!(hits.len(), 50);
}

#[test]
fn search_rejects_zero_limit() {
    let fixture = fixture();
    let error = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense", 0, &[], &[], &[])
        .unwrap_err();
    assert!(matches!(error, RagError::LimitTooSmall));
}

#[test]
fn chunk_title_uses_source_ref_and_location() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.md");
    write(&path, "# Sense\n\nSense menu note.");
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();
    let hit = &RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense menu", 1, &[], &[], &[])
        .unwrap()[0];

    assert_eq!(
        hit.chunk.title(),
        format!("{} {}", path.display(), "Sense paragraph 1")
    );
    assert_eq!(hit.chunk.kind(), "markdown_section");
}

// ---------------------------------------------------------------------------
// DOCX and PDF ingestion (Python `rag_conversion.py`: mammoth and pypdf)
// ---------------------------------------------------------------------------

/// Write a minimal DOCX whose only payload is `word/document.xml`, mirroring
/// the Python test fixtures that hand-roll the archive with `ZipFile`.
fn write_docx(path: &Path, body: &str) {
    use std::io::Write as _;

    let document = format!(
        "<?xml version=\"1.0\"?><w:document \
         xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}</w:body></w:document>"
    );
    let file = std::fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    archive.start_file("word/document.xml", options).unwrap();
    archive.write_all(document.as_bytes()).unwrap();
    archive.finish().unwrap();
}

fn docx_paragraph(style: Option<&str>, text: &str) -> String {
    let properties = match style {
        Some(style) => format!("<w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>"),
        None => String::new(),
    };
    format!("<w:p>{properties}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

/// Hand-craft a single-page PDF with correct xref offsets: an empty page when
/// `text` is `None`, otherwise one text run in base-14 Helvetica.
fn write_pdf(path: &Path, text: Option<&str>) {
    write_pdf_with_font(
        path,
        text,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    );
}

/// Variant of [`write_pdf`] with a caller-supplied font dictionary, used to
/// craft malformed-but-PDF-shaped inputs for pdf-extract.
fn write_pdf_with_font(path: &Path, text: Option<&str>, font_dictionary: &str) {
    let content = text
        .map(|text| format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET"))
        .unwrap_or_default();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        font_dictionary.to_string(),
    ];

    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.drain(..).enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let object_count = offsets.len();
    let xref_offset = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n", object_count + 1));
    pdf.push_str("0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        object_count + 1
    ));
    std::fs::write(path, pdf).unwrap();
}

#[test]
fn import_docx_converts_to_managed_markdown_and_searches() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.docx");
    write_docx(
        &path,
        &[
            docx_paragraph(Some("Heading1"), "Sense"),
            docx_paragraph(None, "Sense menu note."),
            docx_paragraph(None, "Cooking Talent appears here."),
        ]
        .concat(),
    );

    let result = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.docx"),
        )
        .unwrap();

    assert_eq!(result.source.source_type, "markdown");
    // Content type follows the normalized file, like Python (`load_rag_file`
    // on the managed markdown); the DOCX path stays visible via provenance.
    assert_eq!(result.source.content_type, "md");
    assert_eq!(
        result.source.metadata.get("original_path"),
        Some(&serde_json::Value::String(path.display().to_string()))
    );
    assert_eq!(result.normalized_format, "markdown");
    assert!(result.normalized_path.ends_with(".md"));
    assert_eq!(
        std::fs::read_to_string(&result.normalized_path).unwrap(),
        "# Sense\n\nSense menu note.\n\nCooking Talent appears here.\n"
    );
    assert_eq!(result.chunk_count, 2);

    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Cooking Talent", 5, &[], &[], &[])
        .unwrap();
    assert_eq!(
        hits.iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["Cooking Talent appears here."]
    );
    assert_eq!(hits[0].chunk.chunk_kind, "markdown_section");
    assert_eq!(hits[0].chunk.location, "Sense paragraph 2");
    assert_eq!(hits[0].chunk.source_ref, "chapter.docx");
}

#[test]
fn import_corrupt_docx_is_invalid_source() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.docx");
    write(&path, "not a zip archive");

    let error = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap_err();

    assert!(matches!(error, RagError::InvalidSource(_)));
}

#[test]
fn import_pdf_extracts_text_and_searches() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.pdf");
    write_pdf(&path, Some("Cooking Talent appears here."));

    let result = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(
            &fixture.series_slug,
            &path,
            &RagImport::new().source_ref("chapter.pdf"),
        )
        .unwrap();

    assert_eq!(result.source.source_type, "text");
    assert_eq!(result.source.content_type, "txt");
    assert_eq!(
        result.source.metadata.get("original_path"),
        Some(&serde_json::Value::String(path.display().to_string()))
    );
    assert_eq!(result.normalized_format, "text");
    assert!(result.normalized_path.ends_with(".txt"));
    assert_eq!(result.chunk_count, 1);

    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Cooking Talent", 5, &[], &[], &[])
        .unwrap();
    assert_eq!(
        hits.iter()
            .map(|hit| hit.chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["Cooking Talent appears here."]
    );
    assert_eq!(hits[0].chunk.chunk_kind, "text");
    assert_eq!(hits[0].chunk.source_ref, "chapter.pdf");
}

#[test]
fn import_pdf_without_extractable_text_is_rejected() {
    let fixture = fixture();
    let path = fixture.root.path().join("scan.pdf");
    write_pdf(&path, None);

    let error = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap_err();

    assert!(
        matches!(error, RagError::InvalidSource(_)),
        "unexpected error: {error}"
    );
    assert!(
        error
            .to_string()
            .starts_with("invalid RAG source: source produced no extractable text"),
        "unexpected error: {error}"
    );
}

/// PDF-shaped garbage: an unknown `/Encoding` name hits a `panic!` inside
/// pdf-extract, so the import must come back as `InvalidSource` (mentioning
/// the panic) instead of aborting the process. This only works because the
/// workspace Cargo profiles never set `panic = "abort"`.
#[test]
fn import_malformed_pdf_panicking_parser_is_invalid_source() {
    let fixture = fixture();
    let path = fixture.root.path().join("malformed.pdf");
    write_pdf_with_font(
        &path,
        Some("Sense"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
         /Encoding /BogusEncoding >>",
    );

    let error = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap_err();

    assert!(
        matches!(error, RagError::InvalidSource(_)),
        "unexpected error: {error}"
    );
    let message = error.to_string();
    assert!(
        message.starts_with("invalid RAG source: invalid PDF source"),
        "unexpected error: {message}"
    );
    assert!(message.contains("panic"), "unexpected error: {message}");
}

/// A DOCX whose `word/document.xml` decompresses past the 64 MiB cap must be
/// rejected up front instead of being read into memory.
#[test]
fn import_docx_oversized_document_xml_is_invalid_source() {
    let fixture = fixture();
    let path = fixture.root.path().join("bomb.docx");
    let file = std::fs::File::create(&path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    archive.start_file("word/document.xml", options).unwrap();
    let megabyte = vec![b'A'; 1024 * 1024];
    for _ in 0..65 {
        use std::io::Write as _;
        archive.write_all(&megabyte).unwrap();
    }
    archive.finish().unwrap();

    let error = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap_err();

    assert!(
        matches!(error, RagError::InvalidSource(_)),
        "unexpected error: {error}"
    );
    let message = error.to_string();
    assert!(
        message.starts_with("invalid RAG source: invalid DOCX source"),
        "unexpected error: {message}"
    );
    assert!(message.contains("exceeds"), "unexpected error: {message}");
}

/// Pin: `.rtf` is not a supported RAG extension and must name the suffix with
/// its leading dot.
#[test]
fn import_rtf_is_unsupported_source_extension() {
    let fixture = fixture();
    let path = fixture.root.path().join("chapter.rtf");
    write(&path, r"{\rtf1\ansi Sense menu note.}");

    let error = RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap_err();

    assert!(
        matches!(error, RagError::InvalidSource(_)),
        "unexpected error: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("unsupported RAG source extension: .rtf"),
        "unexpected error: {error}"
    );
}
