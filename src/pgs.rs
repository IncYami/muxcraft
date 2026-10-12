use anyhow::{Result, ensure};

struct Segment<'a> {
    pts: i64,
    dts: i64,
    kind: u8,
    body: &'a [u8],
}

fn segments(bytes: &[u8]) -> Result<Vec<Segment<'_>>> {
    let mut output = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        ensure!(
            bytes.len() - offset >= 13 && &bytes[offset..offset + 2] == b"PG",
            "Invalid or truncated PGS segment"
        );
        let pts = u32::from_be_bytes(bytes[offset + 2..offset + 6].try_into()?) as i64;
        let dts = u32::from_be_bytes(bytes[offset + 6..offset + 10].try_into()?) as i64;
        let kind = bytes[offset + 10];
        let length = u16::from_be_bytes(bytes[offset + 11..offset + 13].try_into()?) as usize;
        ensure!(offset + 13 + length <= bytes.len(), "Truncated PGS payload");
        output.push(Segment {
            pts,
            dts,
            kind,
            body: &bytes[offset + 13..offset + 13 + length],
        });
        offset += 13 + length;
    }
    ensure!(
        output.last().is_some_and(|segment| segment.kind == 0x80),
        "PGS ends inside a display set"
    );
    Ok(output)
}

/// Select complete PGS display sets and express their timestamps on the cut video timeline.
/// `source_first_pts` is the timestamp of the first PGS packet in the source container.
pub fn cut(
    full_sup: &[u8],
    source_first_pts: i64,
    start_pts: i64,
    end_pts: i64,
) -> Result<Vec<u8>> {
    ensure!(end_pts > start_pts, "Invalid PGS cut interval");
    let all = segments(full_sup)?;
    let sup_first_pts = all.first().unwrap().pts;
    let offset = source_first_pts - sup_first_pts;
    let mut output = Vec::new();
    let mut display = Vec::new();
    let mut active = false;
    let mut first_nonempty = true;
    let mut start_checked = false;
    let mut end_checked = false;
    for segment in all {
        display.push(segment);
        if display.last().unwrap().kind != 0x80 {
            continue;
        }
        let first = display.iter().map(|s| s.pts + offset).min().unwrap();
        let last = display.iter().map(|s| s.pts + offset).max().unwrap();
        ensure!(
            !(first < start_pts && last >= start_pts) && !(first < end_pts && last >= end_pts),
            "PGS display set crosses the video cut boundary"
        );
        if first >= start_pts && !start_checked {
            ensure!(
                !active,
                "PGS subtitle remains active across the start boundary"
            );
            start_checked = true;
        }
        if first >= end_pts && !end_checked {
            ensure!(
                !active,
                "PGS subtitle remains active across the end boundary"
            );
            end_checked = true;
        }
        if first >= start_pts && first < end_pts {
            if first_nonempty
                && let Some(pcs) = display
                    .iter()
                    .find(|s| s.kind == 0x16 && s.body.get(10).copied().unwrap_or(0) > 0)
            {
                let epoch_start = pcs.body.get(7) == Some(&0x80);
                let self_contained = display.iter().any(|s| s.kind == 0x14)
                    && display.iter().any(|s| s.kind == 0x15);
                ensure!(
                    epoch_start || self_contained,
                    "First cut PGS display depends on objects before the cut"
                );
                first_nonempty = false;
            }
            for s in &display {
                let pts = s.pts + offset - start_pts;
                let dts = if s.dts == 0 {
                    0
                } else {
                    (s.dts + offset - start_pts).max(0)
                };
                ensure!(
                    pts >= 0 && pts <= u32::MAX as i64 && dts <= u32::MAX as i64,
                    "PGS timestamp outside 32-bit range"
                );
                output.extend_from_slice(b"PG");
                output.extend_from_slice(&(pts as u32).to_be_bytes());
                output.extend_from_slice(&(dts as u32).to_be_bytes());
                output.push(s.kind);
                output.extend_from_slice(&(s.body.len() as u16).to_be_bytes());
                output.extend_from_slice(s.body);
            }
        }
        if let Some(pcs) = display.iter().find(|s| s.kind == 0x16) {
            ensure!(pcs.body.len() >= 11, "Truncated PGS composition");
            active = pcs.body[10] > 0;
        }
        display.clear();
    }
    if !start_checked {
        ensure!(
            !active,
            "PGS subtitle remains active across the start boundary"
        );
    }
    if !end_checked {
        ensure!(
            !active,
            "PGS subtitle remains active across the end boundary"
        );
    }
    ensure!(
        !output.is_empty(),
        "PGS track has no displays in the cut interval"
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display(sup: &mut Vec<u8>, pts: u32, objects: u8) {
        let mut body = [0_u8; 11];
        body[10] = objects;
        sup.extend(b"PG");
        sup.extend(pts.to_be_bytes());
        sup.extend(0_u32.to_be_bytes());
        sup.push(0x16);
        sup.extend(11_u16.to_be_bytes());
        sup.extend(body);
        sup.extend(b"PG");
        sup.extend(pts.to_be_bytes());
        sup.extend(0_u32.to_be_bytes());
        sup.push(0x80);
        sup.extend(0_u16.to_be_bytes());
    }
    #[test]
    fn rejects_active_display_at_boundary() {
        let mut sup = Vec::new();
        display(&mut sup, 90000, 1);
        display(&mut sup, 270000, 0);
        assert!(cut(&sup, 90000, 90000 + 45000, 270000).is_err());
    }
    #[test]
    fn keeps_complete_displays_and_rebases_timestamps() {
        let mut sup = Vec::new();
        display(&mut sup, 0, 0);
        display(&mut sup, 90000, 0);
        display(&mut sup, 180000, 0);
        let cropped = cut(&sup, 450000, 540000, 720000).unwrap();
        let selected = segments(&cropped).unwrap();
        assert_eq!(selected.len(), 4);
        assert_eq!(selected[0].pts, 0);
        assert_eq!(selected[2].pts, 90000);
    }
}
