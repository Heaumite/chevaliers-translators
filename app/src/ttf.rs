//! Lecture minimale d'une police TrueType / OpenType (ou du premier visage d'une
//! collection .ttc) : son nom et la liste des caractères qu'elle dessine.

pub struct Font {
    pub family: String,
    pub full: String,
    /// Plages de caractères couverts, triées et fusionnées.
    ranges: Vec<(u32, u32)>,
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn u16(&self, i: usize) -> Result<u16, String> {
        self.0
            .get(i..i + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .ok_or_else(|| "police tronquée".to_string())
    }
    fn u32(&self, i: usize) -> Result<u32, String> {
        self.0
            .get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "police tronquée".to_string())
    }
    fn bytes(&self, i: usize, n: usize) -> Result<&[u8], String> {
        self.0
            .get(i..i + n)
            .ok_or_else(|| "police tronquée".to_string())
    }
}

impl Font {
    pub fn parse(data: &[u8]) -> Result<Font, String> {
        let r = R(data);
        let base = if data.starts_with(b"ttcf") {
            r.u32(12)? as usize
        } else {
            0
        };
        match r.u32(base)? {
            0x0001_0000 | 0x4F54_544F | 0x7472_7565 => {} // 1.0, "OTTO", "true"
            v => return Err(format!("pas une police TrueType/OpenType (0x{v:08X})")),
        }
        let n = r.u16(base + 4)? as usize;
        let mut name = None;
        let mut cmap = None;
        for i in 0..n {
            let rec = base + 12 + 16 * i;
            let tag = r.bytes(rec, 4)?;
            let off = r.u32(rec + 8)? as usize;
            let len = r.u32(rec + 12)? as usize;
            let table = r.bytes(off, len)?;
            match tag {
                b"name" => name = Some(table),
                b"cmap" => cmap = Some(table),
                _ => {}
            }
        }
        let (family, full) = match name {
            Some(t) => names(t)?,
            None => (String::new(), String::new()),
        };
        let ranges = cmap_ranges(cmap.ok_or("table cmap absente")?)?;
        Ok(Font {
            family,
            full,
            ranges,
        })
    }

    pub fn has(&self, c: char) -> bool {
        let c = c as u32;
        let i = self.ranges.partition_point(|(_, end)| *end < c);
        self.ranges.get(i).is_some_and(|(start, _)| *start <= c)
    }

    /// Caractères de `s` absents de la police (sans doublons, dans l'ordre).
    pub fn missing(&self, s: &str) -> Vec<char> {
        let mut out: Vec<char> = Vec::new();
        for c in s.chars() {
            if !c.is_whitespace() && !self.has(c) && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    pub fn char_count(&self) -> u32 {
        self.ranges.iter().map(|(a, b)| b - a + 1).sum()
    }
}

fn names(t: &[u8]) -> Result<(String, String), String> {
    let r = R(t);
    let count = r.u16(2)? as usize;
    let strings = r.u16(4)? as usize;
    let mut best: [(i32, String); 2] = [(-1, String::new()), (-1, String::new())];
    for i in 0..count {
        let rec = 6 + 12 * i;
        let (platform, lang, id) = (r.u16(rec)?, r.u16(rec + 4)?, r.u16(rec + 6)?);
        let slot = match id {
            1 => 0,
            4 => 1,
            _ => continue,
        };
        let score = match (platform, lang) {
            (3, 0x409) => 3,
            (3, _) | (0, _) => 2,
            (1, 0) => 1,
            _ => continue,
        };
        if score <= best[slot].0 {
            continue;
        }
        let (len, off) = (r.u16(rec + 8)? as usize, r.u16(rec + 10)? as usize);
        let Ok(raw) = r.bytes(strings + off, len) else {
            continue;
        };
        let text = if platform == 1 {
            raw.iter().map(|b| *b as char).collect()
        } else {
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        };
        best[slot] = (score, text);
    }
    let [(_, family), (_, full)] = best;
    Ok((family, full))
}

fn cmap_ranges(t: &[u8]) -> Result<Vec<(u32, u32)>, String> {
    let r = R(t);
    let n = r.u16(2)? as usize;
    // Sous-table préférée : Unicode complet (format 12), sinon BMP (format 4).
    let mut best: Option<(i32, usize)> = None;
    for i in 0..n {
        let rec = 4 + 8 * i;
        let (platform, encoding, off) = (r.u16(rec)?, r.u16(rec + 2)?, r.u32(rec + 4)? as usize);
        let Ok(format) = r.u16(off) else {
            continue;
        };
        let unicode = matches!((platform, encoding), (0, _) | (3, 1) | (3, 10));
        let score = match format {
            12 if unicode => 2,
            4 if unicode => 1,
            _ => continue,
        };
        if best.map_or(true, |(s, _)| score > s) {
            best = Some((score, off));
        }
    }
    let (_, off) = best.ok_or("aucune table de caractères Unicode")?;
    let mut chars: Vec<(u32, u32)> = Vec::new();
    let mut push = |a: u32, b: u32| match chars.last_mut() {
        Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
        _ => chars.push((a, b)),
    };
    if r.u16(off)? == 12 {
        let groups = r.u32(off + 12)? as usize;
        let mut v = Vec::with_capacity(groups);
        for g in 0..groups {
            let e = off + 16 + 12 * g;
            let (start, end, glyph) = (r.u32(e)?, r.u32(e + 4)?, r.u32(e + 8)?);
            // Le premier caractère d'un groupe commençant au glyphe 0 n'est pas dessiné.
            let start = if glyph == 0 { start + 1 } else { start };
            if start <= end {
                v.push((start, end));
            }
        }
        v.sort();
        for (a, b) in v {
            push(a, b);
        }
    } else {
        let seg = r.u16(off + 6)? as usize / 2;
        let ends = off + 14;
        let starts = ends + 2 * seg + 2;
        let deltas = starts + 2 * seg;
        let roffs = deltas + 2 * seg;
        let mut v = Vec::new();
        for s in 0..seg {
            let (end, start) = (r.u16(ends + 2 * s)? as u32, r.u16(starts + 2 * s)? as u32);
            let delta = r.u16(deltas + 2 * s)? as u32;
            let ro = r.u16(roffs + 2 * s)? as usize;
            if start > end {
                continue;
            }
            let mut run: Option<u32> = None;
            for c in start..=end.min(0xFFFE) {
                let glyph = if ro == 0 {
                    (c + delta) & 0xFFFF
                } else {
                    let addr = roffs + 2 * s + ro + 2 * (c - start) as usize;
                    match r.u16(addr) {
                        Ok(0) | Err(_) => 0,
                        Ok(g) => (g as u32 + delta) & 0xFFFF,
                    }
                };
                match (glyph != 0, run) {
                    (true, None) => run = Some(c),
                    (false, Some(a)) => {
                        v.push((a, c - 1));
                        run = None;
                    }
                    _ => {}
                }
            }
            if let Some(a) = run {
                v.push((a, end.min(0xFFFE)));
            }
        }
        v.sort();
        for (a, b) in v {
            push(a, b);
        }
    }
    Ok(chars)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Police minimale : nom « CT Test » et caractères A-Z (format 4, delta),
    /// é (format 4, tableau de glyphes), Ω et 😀 (format 12).
    pub fn tiny_font(with12: bool) -> Vec<u8> {
        fn be16(v: &mut Vec<u8>, x: u16) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        fn be32(v: &mut Vec<u8>, x: u32) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        // name : un enregistrement Windows anglais (nameID 1) et un Mac (nameID 4).
        let fam: Vec<u8> = "CT Test"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        let full = b"CT Test Regular";
        let mut name = Vec::new();
        be16(&mut name, 0);
        be16(&mut name, 2);
        be16(&mut name, 6 + 24);
        for (p, e, l, id, len, off) in [
            (3u16, 1u16, 0x409u16, 1u16, fam.len() as u16, 0u16),
            (1, 0, 0, 4, full.len() as u16, fam.len() as u16),
        ] {
            for x in [p, e, l, id, len, off] {
                be16(&mut name, x);
            }
        }
        name.extend_from_slice(&fam);
        name.extend_from_slice(full);

        // cmap format 4 : [A-Z] delta, [é] via tableau, segment final 0xFFFF.
        let mut f4 = Vec::new();
        let seg = 3u16;
        be16(&mut f4, 4);
        be16(&mut f4, 0); // longueur, corrigée plus bas
        be16(&mut f4, 0);
        be16(&mut f4, seg * 2);
        be16(&mut f4, 4);
        be16(&mut f4, 1);
        be16(&mut f4, 2);
        for e in [0x5A, 0xE9, 0xFFFF] {
            be16(&mut f4, e);
        }
        be16(&mut f4, 0);
        for s in [0x41, 0xE9, 0xFFFF] {
            be16(&mut f4, s);
        }
        for d in [(1u16).wrapping_sub(0x41), 0, 1] {
            be16(&mut f4, d);
        }
        // idRangeOffset du 2e segment : pointe sur le tableau qui suit (2 mots plus loin).
        for ro in [0u16, 4, 0] {
            be16(&mut f4, ro);
        }
        be16(&mut f4, 30); // glyphe de é
        let l = f4.len() as u16;
        f4[2..4].copy_from_slice(&l.to_be_bytes());

        let mut f12 = Vec::new();
        be16(&mut f12, 12);
        be16(&mut f12, 0);
        be32(&mut f12, 16 + 12 * 3);
        be32(&mut f12, 0);
        be32(&mut f12, 3);
        for (a, b, g) in [(0x41u32, 0x5Au32, 1u32), (0x3A9, 0x3A9, 40), (0x1F600, 0x1F600, 41)] {
            be32(&mut f12, a);
            be32(&mut f12, b);
            be32(&mut f12, g);
        }

        let mut cmap = Vec::new();
        be16(&mut cmap, 0);
        be16(&mut cmap, if with12 { 2 } else { 1 });
        let first = 4 + 8 * if with12 { 2 } else { 1 };
        be16(&mut cmap, 3);
        be16(&mut cmap, 1);
        be32(&mut cmap, first as u32);
        if with12 {
            be16(&mut cmap, 3);
            be16(&mut cmap, 10);
            be32(&mut cmap, (first + f4.len()) as u32);
        }
        cmap.extend_from_slice(&f4);
        if with12 {
            cmap.extend_from_slice(&f12);
        }

        let mut font = Vec::new();
        be32(&mut font, 0x0001_0000);
        be16(&mut font, 2);
        be16(&mut font, 32);
        be16(&mut font, 1);
        be16(&mut font, 0);
        let off_cmap = 12 + 32;
        let off_name = off_cmap + cmap.len();
        for (tag, off, len) in [
            (b"cmap", off_cmap, cmap.len()),
            (b"name", off_name, name.len()),
        ] {
            font.extend_from_slice(tag);
            be32(&mut font, 0);
            be32(&mut font, off as u32);
            be32(&mut font, len as u32);
        }
        font.extend_from_slice(&cmap);
        font.extend_from_slice(&name);
        font
    }

    #[test]
    fn format4_and_names() {
        let f = Font::parse(&tiny_font(false)).unwrap();
        assert_eq!(f.family, "CT Test");
        assert_eq!(f.full, "CT Test Regular");
        assert!(f.has('A') && f.has('Z') && f.has('é'));
        assert!(!f.has('a') && !f.has('Ω') && !f.has('@'));
        assert_eq!(f.char_count(), 27);
        assert_eq!(f.missing("AéaΩa Z"), vec!['a', 'Ω']);
    }

    #[test]
    fn format12_preferred() {
        let f = Font::parse(&tiny_font(true)).unwrap();
        assert!(f.has('A') && f.has('Ω') && f.has('😀'));
        assert!(!f.has('é'));
    }

    #[test]
    fn rejects_non_fonts() {
        assert!(Font::parse(b"hello world, not a font").is_err());
        assert!(Font::parse(&[0, 1, 0, 0, 0]).is_err());
    }

    /// Police système réelle si présente (DejaVu sous Linux, Arial sous Windows).
    #[test]
    fn system_font_if_available() {
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ];
        for p in candidates {
            if let Ok(d) = std::fs::read(p) {
                let f = Font::parse(&d).unwrap();
                assert!(!f.family.is_empty());
                assert!(f.has('a') && f.has('é') && f.has('Ж') && f.has('Ω'));
                assert!(f.char_count() > 1000);
            }
        }
    }
}
