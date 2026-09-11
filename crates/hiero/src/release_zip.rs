//! Strict ZIP32 framing before the ZIP library (which coalesces duplicate names).
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
fn read(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn u16b(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
fn u32b(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}
pub(crate) fn validate(file: &mut File) -> Result<(), String> {
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length < 22 {
        return Err("truncated ZIP".into());
    }
    let end = read(file, length - 22, 22)?;
    if u32b(&end, 0) != 0x06054b50
        || u16b(&end, 4) != 0
        || u16b(&end, 6) != 0
        || u16b(&end, 20) != 0
    {
        return Err("ZIP32 without comment or multiple disks required".into());
    }
    let count = u16b(&end, 10);
    let start = u64::from(u32b(&end, 16));
    if count > 10000 || count != u16b(&end, 8) || start + u64::from(u32b(&end, 12)) != length - 22 {
        return Err("invalid ZIP directory bounds".into());
    }
    let mut central = start;
    let mut local_end = 0u64;
    let mut names = HashSet::new();
    for _ in 0..count {
        if central + 46 > length - 22 {
            return Err("truncated ZIP central header".into());
        }
        let c = read(file, central, 46)?;
        let namesize = u16b(&c, 28) as usize;
        let local = u64::from(u32b(&c, 42));
        if u32b(&c, 0) != 0x02014b50
            || u16b(&c, 8) & !0x800 != 0
            || ![0, 8].contains(&u16b(&c, 10))
            || u16b(&c, 30) != 0
            || u16b(&c, 32) != 0
            || u16b(&c, 34) != 0
            || local != local_end
            || local + 30 > start
            || central + 46 + namesize as u64 > length - 22
        {
            return Err("unsupported or overlapping ZIP entry".into());
        }
        let name = read(file, central + 46, namesize)?;
        if std::str::from_utf8(&name).is_err() || !names.insert(name.clone()) {
            return Err("invalid or duplicate ZIP name".into());
        }
        let l = read(file, local, 30)?;
        if u32b(&l, 0) != 0x04034b50
            || u16b(&l, 6) != u16b(&c, 8)
            || u16b(&l, 8) != u16b(&c, 10)
            || l[14..26] != c[16..28]
            || u16b(&l, 26) as usize != namesize
            || u16b(&l, 28) != 0
            || read(file, local + 30, namesize)? != name
        {
            return Err("ZIP local/central header mismatch".into());
        }
        local_end = local + 30 + namesize as u64 + u64::from(u32b(&c, 20));
        if local_end > start {
            return Err("ZIP entry overlaps directory".into());
        }
        central += 46 + namesize as u64;
    }
    if central != length - 22 || local_end != start {
        return Err("unaccounted ZIP bytes".into());
    }
    file.rewind().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive(names: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for name in names {
            let mut l = [0u8; 30];
            l[..4].copy_from_slice(&0x04034b50u32.to_le_bytes());
            l[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            let mut c = [0u8; 46];
            c[..4].copy_from_slice(&0x02014b50u32.to_le_bytes());
            c[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            c[42..46].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            bytes.extend(l);
            bytes.extend(name.as_bytes());
            central.extend(c);
            central.extend(name.as_bytes());
        }
        let mut end = [0u8; 22];
        end[..4].copy_from_slice(&0x06054b50u32.to_le_bytes());
        end[8..10].copy_from_slice(&(names.len() as u16).to_le_bytes());
        end[10..12].copy_from_slice(&(names.len() as u16).to_le_bytes());
        end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
        end[16..20].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        bytes.extend(central);
        bytes.extend(end);
        bytes
    }
    fn check(bytes: &[u8]) -> Result<(), String> {
        let mut f = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut f, bytes).unwrap();
        validate(&mut f)
    }
    #[test]
    fn checks_all_zip_entries_before_library_coalesces_duplicates() {
        assert!(check(&archive(&["hiero.exe", "assets.json"])).is_ok());
        assert!(
            check(&archive(&["hiero.exe", "hiero.exe"]))
                .unwrap_err()
                .contains("duplicate")
        );
    }
    #[test]
    fn rejects_local_central_disagreement_and_unaccounted_bytes() {
        let mut b = archive(&["hiero.exe"]);
        b[30] = b'x';
        assert!(check(&b).is_err());
        let mut b = archive(&["hiero.exe"]);
        b.push(0);
        assert!(check(&b).is_err());
    }
}
