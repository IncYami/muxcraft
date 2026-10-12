//! Read only the OpenType metadata needed for ASS font selection. Font data is
//! never rewritten, split, converted, or merged.

use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

pub struct Metadata {
    pub family: Option<String>,
    pub aliases: BTreeSet<String>,
    pub exact: BTreeSet<String>,
    pub weight: u16,
    pub italic: bool,
    pub variable: bool,
}

fn bytes(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset.checked_add(len).context("Font offset overflow")?;
    data.get(offset..end).context("Truncated OpenType font")
}
fn u16be(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(bytes(data, offset, 2)?.try_into()?))
}
fn u32be(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(bytes(data, offset, 4)?.try_into()?))
}

fn face_offsets(data: &[u8]) -> Result<Vec<usize>> {
    if bytes(data, 0, 4)? != b"ttcf" {
        return Ok(vec![0]);
    }
    let count = u32be(data, 8)? as usize;
    ensure!(
        (1..=1024).contains(&count),
        "Invalid OpenType collection count"
    );
    bytes(data, 12, count * 4)?;
    (0..count)
        .map(|i| Ok(u32be(data, 12 + i * 4)? as usize))
        .collect()
}

fn table<'a>(data: &'a [u8], face: usize, tag: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    let signature = bytes(data, face, 4)?;
    ensure!(
        signature == b"OTTO"
            || signature == b"true"
            || signature == b"typ1"
            || signature == [0, 1, 0, 0],
        "Invalid OpenType face signature"
    );
    let count = u16be(data, face + 4)? as usize;
    ensure!(count <= 4096, "Too many OpenType tables");
    let records = face.checked_add(12).context("Font offset overflow")?;
    bytes(data, records, count * 16)?;
    for i in 0..count {
        let record = records + i * 16;
        if bytes(data, record, 4)? == tag {
            let offset = u32be(data, record + 8)? as usize;
            let length = u32be(data, record + 12)? as usize;
            return Ok(Some(bytes(data, offset, length)?));
        }
    }
    Ok(None)
}

fn names(data: &[u8]) -> Result<(Option<String>, BTreeSet<String>, BTreeSet<String>)> {
    ensure!(data.len() >= 6, "Truncated OpenType name table");
    ensure!(
        u16be(data, 0)? <= 1,
        "Unsupported OpenType name table format"
    );
    let count = u16be(data, 2)? as usize;
    let storage = u16be(data, 4)? as usize;
    bytes(data, 6, count * 12)?;
    let mut family = None;
    let mut preferred = None;
    let mut aliases = BTreeSet::new();
    let mut exact = BTreeSet::new();
    for i in 0..count {
        let record = 6 + i * 12;
        let platform = u16be(data, record)?;
        let language = u16be(data, record + 4)?;
        let id = u16be(data, record + 6)?;
        if ![1, 4, 6, 16].contains(&id) || ![0, 3].contains(&platform) {
            continue;
        }
        let len = u16be(data, record + 8)? as usize;
        let offset = storage
            .checked_add(u16be(data, record + 10)? as usize)
            .context("Font name offset overflow")?;
        let raw = bytes(data, offset, len)?;
        if raw.len() % 2 != 0 {
            continue;
        }
        let utf16 = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes(*b))
            .collect::<Vec<_>>();
        let Ok(value) = String::from_utf16(&utf16) else {
            continue;
        };
        let value = value.trim_matches('\0').trim().to_owned();
        if value.is_empty() {
            continue;
        }
        let normalized = value.trim_start_matches('@').to_lowercase();
        aliases.insert(normalized.clone());
        if [4, 6].contains(&id) {
            exact.insert(normalized);
        }
        if id == 1 && (family.is_none() || language == 0x409) {
            family = Some(value.clone());
        }
        if id == 16 && (preferred.is_none() || language == 0x409) {
            preferred = Some(value);
        }
    }
    Ok((preferred.or(family), aliases, exact))
}

pub fn parse(data: &[u8]) -> Result<Vec<Metadata>> {
    face_offsets(data)?
        .into_iter()
        .map(|face| {
            let name = table(data, face, b"name")?.context("OpenType face has no name table")?;
            let (family, aliases, exact) = names(name)?;
            let os2 = table(data, face, b"OS/2")?;
            let head = table(data, face, b"head")?;
            let weight = os2
                .and_then(|t| u16be(t, 4).ok())
                .filter(|w| *w > 0)
                .unwrap_or(400);
            let italic = os2
                .and_then(|t| u16be(t, 62).ok())
                .map(|flags| flags & 1 != 0)
                .or_else(|| {
                    head.and_then(|t| u16be(t, 44).ok())
                        .map(|flags| flags & 2 != 0)
                })
                .unwrap_or(false);
            Ok(Metadata {
                family,
                aliases,
                exact,
                weight,
                italic,
                variable: table(data, face, b"fvar")?.is_some(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::fonts::{FontIndex, Request};
    use std::collections::BTreeSet;

    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    fn face(base: usize, family: &str, weight: u16, italic: bool) -> Vec<u8> {
        let text = family
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        let name_offset = 12 + 3 * 16;
        let name_len = 6 + 12 + text.len();
        let os2_offset = name_offset + name_len;
        let head_offset = os2_offset + 64;
        let mut data = vec![0; head_offset + 46];
        data[..4].copy_from_slice(&[0, 1, 0, 0]);
        put16(&mut data, 4, 3);
        for (i, (tag, offset, len)) in [
            (b"name", name_offset, name_len),
            (b"OS/2", os2_offset, 64),
            (b"head", head_offset, 46),
        ]
        .into_iter()
        .enumerate()
        {
            let at = 12 + i * 16;
            data[at..at + 4].copy_from_slice(tag);
            put32(&mut data, at + 8, (base + offset) as u32);
            put32(&mut data, at + 12, len as u32);
        }
        put16(&mut data, name_offset + 2, 1);
        put16(&mut data, name_offset + 4, 18);
        put16(&mut data, name_offset + 6, 3);
        put16(&mut data, name_offset + 8, 1);
        put16(&mut data, name_offset + 10, 0x409);
        put16(&mut data, name_offset + 12, 16);
        put16(&mut data, name_offset + 14, text.len() as u16);
        data[name_offset + 18..name_offset + 18 + text.len()].copy_from_slice(&text);
        put16(&mut data, os2_offset + 4, weight);
        put16(&mut data, os2_offset + 62, u16::from(italic));
        data
    }

    #[test]
    fn reads_names_and_styles_from_opentype_collection_without_splitting_it() {
        let first = face(20, "Alpha", 400, false);
        let second_offset = 20 + first.len();
        let second = face(second_offset, "Beta", 700, true);
        let mut collection = vec![0; 20];
        collection[..4].copy_from_slice(b"ttcf");
        put32(&mut collection, 4, 0x0001_0000);
        put32(&mut collection, 8, 2);
        put32(&mut collection, 12, 20);
        put32(&mut collection, 16, second_offset as u32);
        collection.extend(first);
        collection.extend(second);
        let faces = parse(&collection).unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].family.as_deref(), Some("Alpha"));
        assert_eq!(faces[1].family.as_deref(), Some("Beta"));
        assert_eq!((faces[1].weight, faces[1].italic), (700, true));

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("fonts");
        std::fs::create_dir(&input).unwrap();
        std::fs::write(input.join("collection.ttc"), &collection).unwrap();
        let index = FontIndex::scan(&[input], false).unwrap();
        let requests = BTreeSet::from([
            Request {
                name: "Alpha".into(),
                weight: 400,
                italic: false,
            },
            Request {
                name: "Beta".into(),
                weight: 700,
                italic: true,
            },
        ]);
        let collected = index.collect(&requests, &dir.path().join("out")).unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(std::fs::read(&collected[0].path).unwrap(), collection);
        assert_eq!(collected[0].path.extension().unwrap(), "ttc");
    }

    #[test]
    fn rejects_truncated_offsets() {
        let mut font = face(0, "Test", 400, false);
        put32(&mut font, 12 + 8, u32::MAX);
        assert!(parse(&font).is_err());
        assert!(parse(b"ttcf").is_err());
    }
}
