// Icône Windows de l'exécutable, écrite sans outil externe : logo TGA décodé, réduit aux tailles
// d'icônes de Windows, rangé dans un fichier de ressources (.res) que l'éditeur de liens MSVC
// prend tel quel. Partagé par build.rs (include!) et ses tests (bibliothèque, cfg(test)).

/// Image RGBA (alpha non prémultiplié), ligne du haut d'abord.
pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// Tailles rangées dans l'icône : barre des tâches, explorateur, zones de notification, à 100 %
/// comme à 250 % d'échelle d'affichage.
pub const ICON_SIZES: [usize; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// TGA vraies couleurs, 24 ou 32 bits, brut (type 2) ou compressé RLE (type 10).
pub fn decode_tga(b: &[u8]) -> Result<Rgba, String> {
    if b.len() < 18 {
        return Err("TGA trop court".into());
    }
    let (idlen, cmap, kind) = (b[0] as usize, b[1], b[2]);
    let w = u16::from_le_bytes([b[12], b[13]]) as usize;
    let h = u16::from_le_bytes([b[14], b[15]]) as usize;
    let (bpp, desc) = (b[16] as usize, b[17]);
    if cmap != 0 || !(kind == 2 || kind == 10) || !(bpp == 24 || bpp == 32) || w == 0 || h == 0 {
        return Err(format!("TGA non pris en charge (type {kind}, palette {cmap}, {bpp} bits)"));
    }
    let n = bpp / 8;
    let total = w * h * n;
    let mut pos = 18 + idlen;
    let mut raw = Vec::with_capacity(total);
    if kind == 2 {
        raw.extend_from_slice(b.get(pos..pos + total).ok_or("TGA tronqué")?);
    } else {
        while raw.len() < total {
            let c = *b.get(pos).ok_or("TGA tronqué")?;
            pos += 1;
            let count = (c & 0x7f) as usize + 1;
            if c & 0x80 != 0 {
                let p = b.get(pos..pos + n).ok_or("TGA tronqué")?;
                pos += n;
                for _ in 0..count {
                    raw.extend_from_slice(p);
                }
            } else {
                raw.extend_from_slice(b.get(pos..pos + n * count).ok_or("TGA tronqué")?);
                pos += n * count;
            }
        }
        raw.truncate(total);
    }
    // Bit 5 : première ligne en haut (sinon en bas) ; bit 4 : première colonne à droite.
    let (top_first, right_first) = (desc & 0x20 != 0, desc & 0x10 != 0);
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let sy = if top_first { y } else { h - 1 - y };
            let sx = if right_first { w - 1 - x } else { x };
            let s = (sy * w + sx) * n;
            let d = (y * w + x) * 4;
            px[d] = raw[s + 2];
            px[d + 1] = raw[s + 1];
            px[d + 2] = raw[s];
            px[d + 3] = if n == 4 { raw[s + 3] } else { 255 };
        }
    }
    Ok(Rgba { w, h, px })
}

/// Réduction à `size` x `size` par moyenne des pixels couverts, couleurs pondérées par l'alpha
/// (pas de liseré sombre au bord du logo).
pub fn shrink(src: &Rgba, size: usize) -> Rgba {
    let mut px = vec![0u8; size * size * 4];
    for y in 0..size {
        let y0 = y * src.h / size;
        let y1 = ((y + 1) * src.h / size).max(y0 + 1);
        for x in 0..size {
            let x0 = x * src.w / size;
            let x1 = ((x + 1) * src.w / size).max(x0 + 1);
            let (mut acc, mut n) = ([0f64; 4], 0f64);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &src.px[(sy * src.w + sx) * 4..][..4];
                    let a = p[3] as f64 / 255.0;
                    for i in 0..3 {
                        acc[i] += p[i] as f64 * a;
                    }
                    acc[3] += a;
                    n += 1.0;
                }
            }
            let d = (y * size + x) * 4;
            if acc[3] > 0.0 {
                for i in 0..3 {
                    px[d + i] = (acc[i] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
            px[d + 3] = (acc[3] / n * 255.0).round() as u8;
        }
    }
    Rgba { w: size, h: size, px }
}

/// Image d'icône au format DIB : en-tête BITMAPINFOHEADER (hauteur doublée), pixels BGRA de bas
/// en haut, puis masque d'un bit par pixel (1 = transparent).
pub fn icon_dib(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let mask_row = w.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + w * h * 4 + mask_row * h);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&((h * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4 + mask_row * h) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &img.px[(y * w + x) * 4..][..4];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..h).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..w {
            if img.px[(y * w + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;

/// Une entrée du fichier .res : type et nom numériques, données alignées sur 4 octets.
fn push_entry(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes()); // taille de l'en-tête
    res.extend_from_slice(&[0xff, 0xff]);
    res.extend_from_slice(&kind.to_le_bytes());
    res.extend_from_slice(&[0xff, 0xff]);
    res.extend_from_slice(&name.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&0u16.to_le_bytes()); // langue neutre
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    while res.len() & 3 != 0 {
        res.push(0);
    }
}

/// Fichier .res : une image RT_ICON par taille (numéros 1..n) et le groupe RT_GROUP_ICON numéro 1,
/// premier groupe de l'exécutable, donc son icône dans l'explorateur et la barre des tâches.
pub fn icon_res(images: &[Rgba]) -> Vec<u8> {
    let mut res = Vec::new();
    // Entrée vide de tête : marque des fichiers .res 32 bits.
    push_entry(&mut res, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes()); // icône
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, img) in images.iter().enumerate() {
        let dib = icon_dib(img);
        push_entry(&mut res, RT_ICON, (i + 1) as u16, 0x1010, &dib);
        let side = |v: usize| if v >= 256 { 0u8 } else { v as u8 };
        group.extend_from_slice(&[side(img.w), side(img.h), 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }
    push_entry(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);
    res
}

/// Du logo TGA au fichier .res.
pub fn logo_res(tga: &[u8]) -> Result<Vec<u8>, String> {
    let logo = decode_tga(tga)?;
    let images: Vec<Rgba> = ICON_SIZES.iter().map(|&s| shrink(&logo, s)).collect();
    Ok(icon_res(&images))
}

// --- Informations de version (0.14.2) : éditeur « Chevaliers » -------------------------------

const RT_VERSION: u16 = 16;

/// Informations de version de l'exécutable : numéro (majeur, mineur, correctif, construction) et
/// textes (CompanyName, FileDescription...), lus par Windows (Propriétés > Détails, Gestionnaire
/// des tâches, antivirus).
pub struct VersionInfo<'a> {
    pub version: [u16; 4],
    pub strings: &'a [(&'a str, &'a str)],
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect()
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() & 3 != 0 {
        v.push(0);
    }
}

/// Un bloc de VS_VERSIONINFO : longueur, longueur de la valeur, texte ou non, clé, valeur, puis
/// les blocs enfants, chacun aligné sur 4 octets (longueur sans le bourrage final, comme rc.exe).
fn version_node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend_from_slice(&value_len.to_le_bytes());
    b.extend_from_slice(&(text as u16).to_le_bytes());
    b.extend(utf16z(key));
    pad4(&mut b);
    b.extend_from_slice(value);
    for c in children {
        pad4(&mut b);
        b.extend_from_slice(c);
    }
    let n = b.len() as u16;
    b[..2].copy_from_slice(&n.to_le_bytes());
    b
}

/// Données de la ressource RT_VERSION : textes en français (040C), Unicode (04B0).
pub fn version_block(v: &VersionInfo) -> Vec<u8> {
    let [a, b, c, d] = v.version.map(|x| x as u32);
    let mut fixed = Vec::new();
    for x in [
        0xFEEF_04BD,      // signature
        0x0001_0000,      // version de la structure
        (a << 16) | b,    // version du fichier
        (c << 16) | d,
        (a << 16) | b,    // version du produit
        (c << 16) | d,
        0x3F,             // masque des indicateurs
        0,                // indicateurs
        0x0004_0004,      // Windows NT, 32 bits et plus
        1,                // application
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&x.to_le_bytes());
    }
    let strings: Vec<Vec<u8>> = v
        .strings
        .iter()
        .map(|(k, val)| version_node(k, &utf16z(val), (val.encode_utf16().count() + 1) as u16, true, &[]))
        .collect();
    let table = version_node("040C04B0", &[], 0, true, &strings);
    let sfi = version_node("StringFileInfo", &[], 0, true, &[table]);
    let translation = version_node("Translation", &[0x0c, 0x04, 0xb0, 0x04], 4, false, &[]);
    let vfi = version_node("VarFileInfo", &[], 0, true, &[translation]);
    version_node("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[sfi, vfi])
}

/// Fichier .res d'un exécutable : icône (logo) et informations de version.
pub fn app_res(tga: &[u8], v: &VersionInfo) -> Result<Vec<u8>, String> {
    let mut res = logo_res(tga)?;
    push_entry(&mut res, RT_VERSION, 1, 0x30, &version_block(v));
    Ok(res)
}

/// Fichier .res avec seulement les informations de version (comparé à celui de llvm-rc).
pub fn version_res(v: &VersionInfo) -> Vec<u8> {
    let mut res = Vec::new();
    push_entry(&mut res, 0, 0, 0, &[]);
    push_entry(&mut res, RT_VERSION, 1, 0x30, &version_block(v));
    res
}

/// Textes de version d'un exécutable du projet (éditeur : Chevaliers).
pub fn project_strings<'a>(version: &'a str, description: &'a str, internal: &'a str, file: &'a str) -> [(&'a str, &'a str); 9] {
    [
        ("CompanyName", "Chevaliers"),
        ("FileDescription", description),
        ("FileVersion", version),
        ("InternalName", internal),
        ("LegalCopyright", "© 2026 Chevaliers"),
        ("OriginalFilename", file),
        ("ProductName", "Chevaliers Translators"),
        ("ProductVersion", version),
        ("Comments", "Traduction du chat de World of Warcraft sur le PC du joueur"),
    ]
}

