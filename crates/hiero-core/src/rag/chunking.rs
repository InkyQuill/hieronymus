pub const MAX_RAG_CHUNK_CHARS: usize = 1_200;

#[must_use]
pub fn split_chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    emit_chunk_text(text, |chunk| {
        chunks.push(chunk);
        Ok::<_, std::convert::Infallible>(())
    })
    .expect("infallible chunk collector");
    chunks
}

pub(crate) fn emit_chunk_text<E>(
    text: &str,
    mut emit: impl FnMut(String) -> Result<(), E>,
) -> Result<(), E> {
    let text = text.trim();
    if text.chars().count() <= MAX_RAG_CHUNK_CHARS {
        return emit(text.to_owned());
    }
    let mut current = String::new();
    let mut start = 0;
    let mut previous = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() && previous.is_some_and(|ch| matches!(ch, '.' | '!' | '?')) {
            let segment = text[start..index].trim();
            if !segment.is_empty() {
                push_segment(segment, &mut current, &mut emit)?;
            }
            start = index + character.len_utf8();
        }
        previous = Some(character);
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        push_segment(tail, &mut current, &mut emit)?;
    }
    emit_current(&mut current, &mut emit)
}

fn push_segment<E>(
    segment: &str,
    current: &mut String,
    emit: &mut impl FnMut(String) -> Result<(), E>,
) -> Result<(), E> {
    if segment.chars().count() > MAX_RAG_CHUNK_CHARS {
        emit_current(current, emit)?;
        emit_word_chunks(segment, emit)?;
        return Ok(());
    }
    let extra = usize::from(!current.is_empty()) + segment.chars().count();
    if current.chars().count() + extra > MAX_RAG_CHUNK_CHARS {
        emit_current(current, emit)?;
    }
    if !current.is_empty() {
        current.push(' ');
    }
    current.push_str(segment);
    Ok(())
}

fn emit_word_chunks<E>(
    text: &str,
    emit: &mut impl FnMut(String) -> Result<(), E>,
) -> Result<(), E> {
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > MAX_RAG_CHUNK_CHARS {
            emit_current(&mut current, emit)?;
            let mut part = String::new();
            for character in word.chars() {
                if part.chars().count() == MAX_RAG_CHUNK_CHARS {
                    emit(std::mem::take(&mut part))?;
                }
                part.push(character);
            }
            emit_current(&mut part, emit)?;
            continue;
        }
        let extra = usize::from(!current.is_empty()) + word.chars().count();
        if current.chars().count() + extra > MAX_RAG_CHUNK_CHARS {
            emit_current(&mut current, emit)?;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    emit_current(&mut current, emit)
}

fn emit_current<E>(
    current: &mut String,
    emit: &mut impl FnMut(String) -> Result<(), E>,
) -> Result<(), E> {
    if !current.is_empty() {
        emit(std::mem::take(current))?;
    }
    Ok(())
}
