pub const MAX_RAG_CHUNK_CHARS: usize = 1_200;

#[must_use]
pub fn split_chunk_text(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.chars().count() <= MAX_RAG_CHUNK_CHARS {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for segment in sentence_segments(text) {
        if segment.chars().count() > MAX_RAG_CHUNK_CHARS {
            push_current(&mut chunks, &mut current);
            chunks.extend(word_chunks(segment));
            continue;
        }
        let extra = usize::from(!current.is_empty()) + segment.chars().count();
        if current.chars().count() + extra > MAX_RAG_CHUNK_CHARS {
            push_current(&mut chunks, &mut current);
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(segment);
    }
    push_current(&mut chunks, &mut current);
    chunks
}

fn sentence_segments(text: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut previous = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() && previous.is_some_and(|ch| matches!(ch, '.' | '!' | '?')) {
            let segment = text[start..index].trim();
            if !segment.is_empty() {
                segments.push(segment);
            }
            start = index + character.len_utf8();
        }
        previous = Some(character);
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        segments.push(tail);
    }
    segments
}

fn word_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > MAX_RAG_CHUNK_CHARS {
            push_current(&mut chunks, &mut current);
            let chars: Vec<char> = word.chars().collect();
            chunks.extend(
                chars
                    .chunks(MAX_RAG_CHUNK_CHARS)
                    .map(|part| part.iter().collect()),
            );
            continue;
        }
        let extra = usize::from(!current.is_empty()) + word.chars().count();
        if current.chars().count() + extra > MAX_RAG_CHUNK_CHARS {
            push_current(&mut chunks, &mut current);
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    push_current(&mut chunks, &mut current);
    chunks
}

fn push_current(chunks: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        chunks.push(std::mem::take(current));
    }
}
