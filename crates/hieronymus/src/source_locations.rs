//! Lossless source-locator projection shared by agent responses and the console.
use crate::memory_models::MemorySource;
use std::collections::BTreeMap;

struct Located {
    source: MemorySource,
    path: String,
    suffix: String,
    start: u64,
    end: u64,
}

fn locate(source: MemorySource) -> Result<Located, MemorySource> {
    let (location, suffix) = source.source_ref.split_once(';').map_or(
        (source.source_ref.as_str(), "".into()),
        |(location, suffix)| (location, format!(";{suffix}")),
    );
    let Some((path, lines)) = location.rsplit_once(':') else {
        return Err(source);
    };
    if std::path::Path::new(path).extension().is_none() {
        return Err(source);
    }
    let (start, end) = lines.split_once('-').unwrap_or((lines, lines));
    let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else {
        return Err(source);
    };
    if start == 0 || end < start {
        return Err(source);
    }
    Ok(Located {
        path: path.into(),
        suffix,
        start,
        end,
        source,
    })
}

pub(crate) fn compact(sources: Vec<MemorySource>) -> Vec<MemorySource> {
    let mut groups: BTreeMap<(String, String, String, String), Vec<Located>> = BTreeMap::new();
    let mut plain = Vec::new();
    for source in sources {
        match locate(source) {
            Ok(location) => groups
                .entry((
                    location.path.clone(),
                    location.suffix.clone(),
                    location.source.volume.clone(),
                    location.source.chapter.clone(),
                ))
                .or_default()
                .push(location),
            Err(source) => plain.push(source),
        }
    }
    for mut group in groups.into_values() {
        if group.len() == 1 {
            plain.push(group.remove(0).source);
            continue;
        }
        group.sort_by_key(|location| (location.start, location.end));
        let mut source = group[0].source.clone();
        let mut spans: Vec<(u64, u64)> = Vec::new();
        let mut ids = Vec::new();
        for location in &group {
            ids.push(location.source.memory_id);
            ids.extend(&location.source.memory_ids);
            if let Some(previous) = spans.last_mut()
                && location.start <= previous.1.saturating_add(1)
            {
                previous.1 = previous.1.max(location.end);
            } else {
                spans.push((location.start, location.end));
            }
        }
        ids.sort_unstable();
        ids.dedup();
        source.memory_id = ids[0];
        source.memory_ids = ids;
        let lines = spans
            .into_iter()
            .map(|(start, end)| {
                if start == end {
                    start.to_string()
                } else {
                    format!("{start}-{end}")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        source.source_ref = format!("{}:{lines}{}", group[0].path, group[0].suffix);
        plain.push(source);
    }
    plain.sort_by_key(|source| source.memory_id);
    plain
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id: i64, line: &str, hash: &str, chapter: &str) -> MemorySource {
        MemorySource {
            memory_id: id,
            memory_ids: Vec::new(),
            source_ref: format!("/book/notes.md:{line};sha256={hash}"),
            volume: "1".into(),
            chapter: chapter.into(),
        }
    }
    #[test]
    fn agent_citations_preserve_exact_ranges_and_all_memory_ids() {
        let hash = "a".repeat(64);
        let sources = (142..=160)
            .map(|line| source(line, &line.to_string(), &hash, "2"))
            .collect();
        let compact = compact(sources);
        assert_eq!(compact.len(), 1);
        assert_eq!(
            compact[0].source_ref,
            format!("/book/notes.md:142-160;sha256={hash}")
        );
        assert_eq!(compact[0].memory_ids, (142..=160).collect::<Vec<_>>());
    }
    #[test]
    fn gaps_share_one_reference_but_revisions_chapters_and_unknown_locators_stay_separate() {
        let hash = "a".repeat(64);
        let sources = vec![
            source(1, "142", &hash, "2"),
            source(2, "144", &hash, "2"),
            source(3, "143", "other-revision", "2"),
            source(4, "143", &hash, "3"),
            MemorySource {
                memory_id: 5,
                memory_ids: Vec::new(),
                source_ref: "Vol 3, chapter 2".into(),
                volume: "".into(),
                chapter: "".into(),
            },
        ];
        let grouped = compact(sources.clone());
        assert_eq!(grouped.len(), 4);
        assert_eq!(
            grouped[0].source_ref,
            format!("/book/notes.md:142,144;sha256={hash}")
        );
        assert_eq!(grouped[0].memory_ids, vec![1, 2]);
        assert_eq!(&grouped[1..], &sources[2..]);
    }
    #[test]
    fn overlapping_and_duplicate_ranges_merge_without_inventing_lines() {
        let hash = "a".repeat(64);
        let compact = compact(vec![
            source(3, "144-150", &hash, "2"),
            source(1, "142-145", &hash, "2"),
            source(2, "144", &hash, "2"),
        ]);
        assert_eq!(
            compact[0].source_ref,
            format!("/book/notes.md:142-150;sha256={hash}")
        );
        assert_eq!(compact[0].memory_ids, vec![1, 2, 3]);
    }
}
