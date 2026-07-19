pub const MAX_RAG_CHUNK_CHARS: usize = 1_200;

#[must_use]
pub fn split_chunk_text(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.chars().count() <= MAX_RAG_CHUNK_CHARS {
        return vec![text.to_owned()];
    }
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
