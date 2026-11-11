//! Lecture seule du stockage local CASC de World of Warcraft (dossier `Data`),
//! limitée à ce qu'il faut pour extraire un fichier par son chemin de jeu :
//!
//!   .build.info -> configuration de build -> fichier d'encodage (CKey -> EKey)
//!   -> index locaux `*.idx` (EKey -> data.NNN + position) -> BLTE -> contenu,
//!
//! le fichier racine (root) donnant la CKey d'un chemin grâce à son hash Jenkins.
//! Chaque contenu extrait est vérifié : son MD5 doit être égal à sa CKey.
//! Rien n'est jamais écrit dans le dossier du jeu.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::install;
use crate::md5::{hex, md5, unhex};

pub type Key = [u8; 16];

/// Drapeaux de langue des entrées du fichier racine.
pub const LOCALES: &[(&str, u32)] = &[
    ("enUS", 0x2),
    ("koKR", 0x4),
    ("frFR", 0x10),
    ("deDE", 0x20),
    ("zhCN", 0x40),
    ("esES", 0x80),
    ("zhTW", 0x100),
    ("enGB", 0x200),
    ("enCN", 0x400),
    ("enTW", 0x800),
    ("esMX", 0x1000),
    ("ruRU", 0x2000),
    ("ptBR", 0x4000),
    ("itIT", 0x8000),
    ("ptPT", 0x10000),
];

pub fn locale_flag(name: &str) -> Option<u32> {
    LOCALES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, f)| *f)
}

pub fn locale_names(flags: u32) -> String {
    if flags == 0xFFFF_FFFF {
        return "toutes".into();
    }
    let v: Vec<&str> = LOCALES
        .iter()
        .filter(|(_, f)| flags & f != 0)
        .map(|(n, _)| *n)
        .collect();
    if v.is_empty() {
        format!("0x{flags:X}")
    } else if v.len() > 4 {
        format!("{} langues", v.len())
    } else {
        v.join("+")
    }
}

const NO_NAME_HASH: u32 = 0x1000_0000;
const LOAD_ON_WINDOWS: u32 = 0x8;
const LOAD_ON_MACOS: u32 = 0x10;
const LOW_VIOLENCE: u32 = 0x80;
const MFST_LE: u32 = 0x4D46_5354; // "TSFM" sur le disque
const MFST_BE: u32 = 0x5453_464D;

// --- Hash Jenkins (lookup3 hashlittle2) des chemins ---------------------------

fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(4);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(6);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(8);
    *b = b.wrapping_add(*a);
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(16);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(19);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(4);
    *b = b.wrapping_add(*a);
}

fn final_mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(14));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(11));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(25));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(16));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(4));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(14));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(24));
}

/// lookup3 `hashlittle2` : renvoie (c, b) comme `*pc, *pb` dans la version C.
pub fn hashlittle2(key: &[u8], pc: u32, pb: u32) -> (u32, u32) {
    let word = |s: &[u8], i: usize| -> u32 {
        (0..4).fold(0u32, |acc, j| {
            acc | (*s.get(i + j).unwrap_or(&0) as u32) << (8 * j)
        })
    };
    let mut a = 0xdead_beefu32
        .wrapping_add(key.len() as u32)
        .wrapping_add(pc);
    let mut b = a;
    let mut c = a.wrapping_add(pb);
    let mut k = key;
    while k.len() > 12 {
        a = a.wrapping_add(word(k, 0));
        b = b.wrapping_add(word(k, 4));
        c = c.wrapping_add(word(k, 8));
        mix(&mut a, &mut b, &mut c);
        k = &k[12..];
    }
    if k.is_empty() {
        return (c, b);
    }
    a = a.wrapping_add(word(k, 0));
    b = b.wrapping_add(word(k, 4));
    c = c.wrapping_add(word(k, 8));
    final_mix(&mut a, &mut b, &mut c);
    (c, b)
}

pub fn normalize_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_uppercase()
}

/// Les deux ordres possibles (c:b et b:c) du hash 64 bits d'un chemin.
/// Les outils de référence ne s'accordent pas : on accepte les deux,
/// une collision fortuite sur 64 bits étant négligeable.
pub fn path_hashes(path: &str) -> [u64; 2] {
    let (c, b) = hashlittle2(normalize_path(path).as_bytes(), 0, 0);
    [
        ((c as u64) << 32) | b as u64,
        ((b as u64) << 32) | c as u64,
    ]
}

// --- BLTE -----------------------------------------------------------------------

pub fn decode_blte(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 || &data[..4] != b"BLTE" {
        return Err("BLTE : signature absente".into());
    }
    let hsize = be32(data, 4) as usize;
    if hsize == 0 {
        return decode_block(&data[8..]);
    }
    if data.len() < 12 || hsize > data.len() {
        return Err("BLTE : en-tête tronqué".into());
    }
    if data[8] != 0x0F {
        return Err(format!("BLTE : table de blocs 0x{:02X} inconnue", data[8]));
    }
    let n = ((data[9] as usize) << 16) | ((data[10] as usize) << 8) | data[11] as usize;
    if 12 + n * 24 > hsize {
        return Err("BLTE : table de blocs tronquée".into());
    }
    let mut out = Vec::new();
    let mut pos = hsize;
    for i in 0..n {
        let e = 12 + i * 24;
        let csize = be32(data, e) as usize;
        let dsize = be32(data, e + 4) as usize;
        let block = data
            .get(pos..pos + csize)
            .ok_or_else(|| format!("BLTE : bloc {i} tronqué"))?;
        let dec = decode_block(block)?;
        if dec.len() != dsize {
            return Err(format!(
                "BLTE : bloc {i} de {} octets au lieu de {dsize}",
                dec.len()
            ));
        }
        out.extend_from_slice(&dec);
        pos += csize;
    }
    Ok(out)
}

fn decode_block(b: &[u8]) -> Result<Vec<u8>, String> {
    match b.first() {
        None => Ok(Vec::new()),
        Some(b'N') => Ok(b[1..].to_vec()),
        Some(b'Z') => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(&b[1..])
                .read_to_end(&mut out)
                .map_err(|e| format!("BLTE : zlib : {e}"))?;
            Ok(out)
        }
        Some(b'F') => decode_blte(&b[1..]),
        Some(b'E') => Err("BLTE : contenu chiffré".into()),
        Some(b'4') => Err("BLTE : compression LZ4 non prise en charge".into()),
        Some(m) => Err(format!("BLTE : mode {:?} inconnu", *m as char)),
    }
}

fn be32(d: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

fn le32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

fn key_at(d: &[u8], i: usize) -> Key {
    let mut k = [0u8; 16];
    k.copy_from_slice(&d[i..i + 16]);
    k
}

// --- Fichier d'encodage (CKey -> EKey) ------------------------------------------

/// EKeys d'une CKey ; vide si la CKey est absente.
pub fn encoding_find(enc: &[u8], ckey: &Key) -> Result<Vec<Key>, String> {
    if enc.len() < 22 || &enc[..2] != b"EN" {
        return Err("encodage : signature absente".into());
    }
    if enc[2] != 1 || enc[3] != 16 || enc[4] != 16 {
        return Err(format!(
            "encodage : version {} / tailles de clés {} {} inconnues",
            enc[2], enc[3], enc[4]
        ));
    }
    let page = u16::from_be_bytes([enc[5], enc[6]]) as usize * 1024;
    let pages = be32(enc, 9) as usize;
    let espec = be32(enc, 18) as usize;
    let table = 22 + espec;
    let first = table + pages * 32;
    if page == 0 || first + pages * page > enc.len() {
        return Err("encodage : fichier tronqué".into());
    }
    let page_at = |i: usize| &enc[first + i * page..first + (i + 1) * page];
    // Pages triées : la bonne est la dernière dont la première clé est <= ckey.
    let (mut lo, mut hi) = (0usize, pages);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if enc[table + mid * 32..table + mid * 32 + 16] <= ckey[..] {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo > 0 {
        if let Some(v) = scan_page(page_at(lo - 1), ckey) {
            return Ok(v);
        }
    }
    // Repli : parcours complet, au cas où les pages ne seraient pas triées.
    for i in 0..pages {
        if let Some(v) = scan_page(page_at(i), ckey) {
            return Ok(v);
        }
    }
    Ok(Vec::new())
}

fn scan_page(p: &[u8], ckey: &Key) -> Option<Vec<Key>> {
    let mut pos = 0;
    while pos + 22 <= p.len() {
        let n = p[pos] as usize;
        if n == 0 {
            break;
        }
        let end = pos + 22 + n * 16;
        if end > p.len() {
            break;
        }
        if p[pos + 6..pos + 22] == ckey[..] {
            return Some((0..n).map(|i| key_at(p, pos + 22 + i * 16)).collect());
        }
        pos = end;
    }
    None
}

// --- Fichier racine (chemin -> CKey) ----------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootFormat {
    Legacy,
    MfstV1,
    MfstV2,
}

impl RootFormat {
    pub fn label(self) -> &'static str {
        match self {
            RootFormat::Legacy => "ancien",
            RootFormat::MfstV1 => "MFST v1",
            RootFormat::MfstV2 => "MFST v2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub fdid: u32,
    pub ckey: Key,
    pub locale: u32,
    pub content: u32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RootStats {
    pub blocks: usize,
    pub entries: usize,
    pub named: usize,
}

/// Résultat du parcours de la racine : format, statistiques, entrées trouvées
/// (avec l'indice de ce qui a été trouvé : hash `want[i]`, ou `want.len() + j` pour `fdids[j]`).
pub type RootScan = (RootFormat, RootStats, Vec<(usize, Hit)>);

/// Parcourt le fichier racine ; renvoie les entrées dont le hash de nom est dans `want`
/// ou dont le FileDataID est dans `fdids`.
pub fn scan_root(root: &[u8], want: &[u64], fdids: &[u32]) -> Result<RootScan, String> {
    let mut stats = RootStats::default();
    let mut hits = Vec::new();
    let (format, mut pos) = if root.len() >= 4 && matches!(le32(root, 0), MFST_LE | MFST_BE) {
        if root.len() < 12 {
            return Err("racine : en-tête MFST tronqué".into());
        }
        let f4 = le32(root, 4);
        if (12..=1024).contains(&f4) && root.len() >= f4 as usize {
            match le32(root, 8) {
                1 => (RootFormat::MfstV1, f4 as usize),
                2 => (RootFormat::MfstV2, f4 as usize),
                v => return Err(format!("racine : MFST version {v} inconnue")),
            }
        } else {
            // 8.2 à 10.1.5 : signature + nombre total + nombre nommé.
            (RootFormat::MfstV1, 12)
        }
    } else {
        (RootFormat::Legacy, 0)
    };

    while pos < root.len() {
        let (n, content, locale, hlen) = match format {
            RootFormat::MfstV2 => {
                if pos + 17 > root.len() {
                    return Err("racine : en-tête de bloc tronqué".into());
                }
                let f1 = le32(root, pos + 8);
                let f2 = le32(root, pos + 12);
                let f3 = root[pos + 16] as u32;
                (le32(root, pos), f1 | f2 | (f3 << 17), le32(root, pos + 4), 17)
            }
            _ => {
                if pos + 12 > root.len() {
                    return Err("racine : en-tête de bloc tronqué".into());
                }
                (le32(root, pos), le32(root, pos + 4), le32(root, pos + 8), 12)
            }
        };
        pos += hlen;
        let n = n as usize;
        let named = content & NO_NAME_HASH == 0 || format == RootFormat::Legacy;
        let deltas = pos;
        let after = deltas + 4 * n;
        let end = match format {
            RootFormat::Legacy => after + 24 * n,
            _ => after + 16 * n + if named { 8 * n } else { 0 },
        };
        if end > root.len() {
            return Err(format!("racine : bloc {} tronqué", stats.blocks + 1));
        }
        stats.blocks += 1;
        stats.entries += n;
        if named {
            stats.named += n;
        }
        if (named && !want.is_empty()) || !fdids.is_empty() {
            let mut fdid: i64 = 0;
            for i in 0..n {
                let d = le32(root, deltas + 4 * i) as i32 as i64;
                fdid = if i == 0 { d } else { fdid + 1 + d };
                let ck = match format {
                    RootFormat::Legacy => after + 24 * i,
                    _ => after + 16 * i,
                };
                let found = if named {
                    let h = match format {
                        RootFormat::Legacy => after + 24 * i + 16,
                        _ => after + 16 * n + 8 * i,
                    };
                    let hash = u64::from_le_bytes(root[h..h + 8].try_into().unwrap());
                    want.iter().position(|x| *x == hash)
                } else {
                    None
                }
                .or_else(|| {
                    fdids
                        .iter()
                        .position(|f| *f as i64 == fdid)
                        .map(|j| want.len() + j)
                });
                if let Some(w) = found {
                    hits.push((
                        w,
                        Hit {
                            fdid: fdid as u32,
                            ckey: key_at(root, ck),
                            locale,
                            content,
                        },
                    ));
                }
            }
        }
        pos = end;
    }
    Ok((format, stats, hits))
}

/// Meilleure entrée pour la langue voulue : la langue du client, sinon l'anglais,
/// en évitant les variantes macOS seules et basse violence.
pub fn pick(hits: &[Hit], locale: u32) -> Option<Hit> {
    let score = |h: &Hit| -> i32 {
        let mut s = 0;
        if h.locale & locale != 0 {
            s += 4;
        } else if h.locale & 0x202 != 0 {
            s += 2;
        }
        if h.content & LOAD_ON_MACOS != 0 && h.content & LOAD_ON_WINDOWS == 0 {
            s -= 8;
        }
        if h.content & LOW_VIOLENCE != 0 {
            s -= 1;
        }
        s
    };
    let mut best: Option<Hit> = None;
    for h in hits {
        if best.map_or(true, |b| score(h) > score(&b)) {
            best = Some(*h);
        }
    }
    best
}

// --- Stockage ---------------------------------------------------------------------

pub struct Storage {
    data_dir: PathBuf,
    idx: Vec<Option<PathBuf>>,
    idx_cache: HashMap<usize, Vec<u8>>,
    encoding: Vec<u8>,
    root: Vec<u8>,
    pub product: String,
    pub version: String,
    pub build_key: String,
    pub root_format: RootFormat,
    pub root_stats: RootStats,
    /// Ordre du hash de nom constaté (0 = c:b, 1 = b:c).
    pub hash_order: Option<usize>,
}

fn parse_config(text: &str) -> HashMap<String, Vec<String>> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((
                k.trim().to_lowercase(),
                v.split_whitespace().map(String::from).collect(),
            ))
        })
        .collect()
}

fn key16(s: &str, what: &str) -> Result<Key, String> {
    unhex(s)
        .filter(|v| v.len() == 16)
        .map(|v| v.try_into().unwrap())
        .ok_or_else(|| format!("clé {what} invalide : {s}"))
}

/// Seau d'index d'une EKey : XOR des 9 premiers octets, replié sur 4 bits.
pub fn bucket(ekey: &[u8]) -> usize {
    let x = ekey[..9].iter().fold(0u8, |a, b| a ^ b);
    ((x & 0xF) ^ (x >> 4)) as usize
}

/// Entrée d'un fichier .idx : (archive, position, taille avec l'en-tête de 30 octets).
pub fn idx_find(data: &[u8], ekey: &Key) -> Result<Option<(u32, u64, u32)>, String> {
    if data.len() < 0x28 {
        return Err("index : fichier trop court".into());
    }
    let version = u16::from_le_bytes([data[8], data[9]]);
    if version != 7 {
        return Err(format!("index : version {version} inconnue"));
    }
    let (size_len, off_len, key_len, off_bits) = (
        data[0x0C] as usize,
        data[0x0D] as usize,
        data[0x0E] as usize,
        data[0x0F] as u32,
    );
    if !(1..=16).contains(&key_len)
        || !(1..=8).contains(&off_len)
        || !(1..=8).contains(&size_len)
        || off_bits >= 64
    {
        return Err("index : format d'entrée inconnu".into());
    }
    let start = 0x28;
    let end = (start + le32(data, 0x20) as usize).min(data.len());
    let elen = key_len + off_len + size_len;
    for e in data[start..end].chunks_exact(elen) {
        if e[..key_len] == ekey[..key_len] {
            let raw = e[key_len..key_len + off_len]
                .iter()
                .fold(0u64, |a, b| (a << 8) | *b as u64);
            let size = e[key_len + off_len..]
                .iter()
                .rev()
                .fold(0u64, |a, b| (a << 8) | *b as u64);
            return Ok(Some((
                (raw >> off_bits) as u32,
                raw & ((1u64 << off_bits) - 1),
                size as u32,
            )));
        }
    }
    Ok(None)
}

impl Storage {
    /// Ouvre le stockage d'une installation (`<wow>\Data`) pour l'un des produits donnés.
    pub fn open(wow: &Path, products: &[&str]) -> Result<Storage, String> {
        let rows = install::build_info_rows(wow);
        if rows.is_empty() {
            return Err(format!("{} : .build.info absent ou illisible", wow.display()));
        }
        let get = |r: &HashMap<String, String>, k: &str| r.get(k).cloned().unwrap_or_default();
        let row = rows
            .iter()
            .filter(|r| products.contains(&get(r, "product").as_str()))
            .max_by_key(|r| get(r, "active") != "0")
            .ok_or_else(|| {
                format!(
                    "produit {} absent de .build.info (présents : {})",
                    products.join("/"),
                    rows.iter()
                        .map(|r| get(r, "product"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let bk = get(row, "build key").to_lowercase();
        if bk.len() < 4 {
            return Err("clé de build absente de .build.info".into());
        }
        let data = wow.join("Data");
        let cfg_path = data.join("config").join(&bk[0..2]).join(&bk[2..4]).join(&bk);
        let cfg = fs::read_to_string(&cfg_path)
            .map_err(|e| format!("configuration de build {} : {e}", cfg_path.display()))?;
        let cfg = parse_config(&cfg);
        let enc = cfg
            .get("encoding")
            .filter(|v| v.len() >= 2)
            .ok_or("configuration de build : ligne encoding absente")?;
        let enc_ckey = key16(&enc[0], "encoding")?;
        let enc_ekey = key16(&enc[1], "encoding")?;
        let root_ckey = key16(
            cfg.get("root")
                .and_then(|v| v.first())
                .ok_or("configuration de build : ligne root absente")?,
            "root",
        )?;

        let data_dir = data.join("data");
        let mut best: Vec<Option<(u32, PathBuf)>> = vec![None; 16];
        let dir = fs::read_dir(&data_dir).map_err(|e| format!("{} : {e}", data_dir.display()))?;
        for e in dir.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let Some(stem) = name.strip_suffix(".idx") else {
                continue;
            };
            if stem.len() != 10 {
                continue;
            }
            let (Ok(b), Ok(v)) = (
                usize::from_str_radix(&stem[0..2], 16),
                u32::from_str_radix(&stem[2..], 16),
            ) else {
                continue;
            };
            if b < 16 && best[b].as_ref().map_or(true, |(bv, _)| v >= *bv) {
                best[b] = Some((v, e.path()));
            }
        }
        if best.iter().all(Option::is_none) {
            return Err(format!("aucun index .idx dans {}", data_dir.display()));
        }

        let mut s = Storage {
            data_dir,
            idx: best.into_iter().map(|o| o.map(|(_, p)| p)).collect(),
            idx_cache: HashMap::new(),
            encoding: Vec::new(),
            root: Vec::new(),
            product: get(row, "product"),
            version: get(row, "version"),
            build_key: bk,
            root_format: RootFormat::Legacy,
            root_stats: RootStats::default(),
            hash_order: None,
        };
        let encoding = s
            .read_ekey(&enc_ekey)
            .map_err(|e| format!("fichier d'encodage : {e}"))?;
        if md5(&encoding) != enc_ckey {
            return Err("fichier d'encodage : contrôle MD5 faux".into());
        }
        s.encoding = encoding;
        let root = s
            .read_ckey(&root_ckey)
            .map_err(|e| format!("fichier racine : {e}"))?;
        let (format, stats, _) = scan_root(&root, &[], &[])?;
        s.root = root;
        s.root_format = format;
        s.root_stats = stats;
        Ok(s)
    }

    fn idx_data(&mut self, b: usize) -> Result<Option<&Vec<u8>>, String> {
        let Some(path) = self.idx[b].clone() else {
            return Ok(None);
        };
        if let std::collections::hash_map::Entry::Vacant(v) = self.idx_cache.entry(b) {
            v.insert(fs::read(&path).map_err(|e| format!("{} : {e}", path.display()))?);
        }
        Ok(self.idx_cache.get(&b))
    }

    fn locate(&mut self, ekey: &Key) -> Result<Option<(u32, u64, u32)>, String> {
        let first = bucket(ekey);
        let order = std::iter::once(first).chain((0..16).filter(move |b| *b != first));
        for b in order {
            if let Some(d) = self.idx_data(b)? {
                if let Some(e) = idx_find(d, ekey)? {
                    return Ok(Some(e));
                }
            }
        }
        Ok(None)
    }

    /// Contenu décodé d'une EKey (sans contrôle MD5).
    pub fn read_ekey(&mut self, ekey: &Key) -> Result<Vec<u8>, String> {
        let (archive, offset, size) = self
            .locate(ekey)?
            .ok_or_else(|| format!("EKey {} absente des index locaux", hex(ekey)))?;
        let path = self.data_dir.join(format!("data.{archive:03}"));
        let mut f = File::open(&path).map_err(|e| format!("{} : {e}", path.display()))?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("{} : {e}", path.display()))?;
        let mut buf = vec![0u8; size as usize];
        f.read_exact(&mut buf)
            .map_err(|e| format!("{} (position {offset}, {size} octets) : {e}", path.display()))?;
        if buf.len() < 38 {
            return Err("entrée de données trop courte".into());
        }
        decode_blte(&buf[30..])
    }

    /// Contenu d'une CKey, vérifié par MD5.
    pub fn read_ckey(&mut self, ckey: &Key) -> Result<Vec<u8>, String> {
        let ekeys = encoding_find(&self.encoding, ckey)?;
        if ekeys.is_empty() {
            return Err(format!("CKey {} absente du fichier d'encodage", hex(ckey)));
        }
        let mut last = String::new();
        for ek in &ekeys {
            match self.read_ekey(ek) {
                Ok(d) if md5(&d) == *ckey => return Ok(d),
                Ok(_) => last = format!("contrôle MD5 faux pour {}", hex(ckey)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Entrées du fichier racine pour un chemin de jeu.
    pub fn find_all(&mut self, path: &str) -> Result<Vec<Hit>, String> {
        let want = path_hashes(path);
        let (_, _, hits) = scan_root(&self.root, &want, &[])?;
        if let Some((w, _)) = hits.first() {
            self.hash_order = Some(*w);
        }
        Ok(hits.into_iter().map(|(_, h)| h).collect())
    }

    /// Entrées du fichier racine pour un FileDataID (repli si le nom est absent).
    pub fn find_fdid(&mut self, fdid: u32) -> Result<Vec<Hit>, String> {
        let (_, _, hits) = scan_root(&self.root, &[], &[fdid])?;
        Ok(hits.into_iter().map(|(_, h)| h).collect())
    }

    /// Entrées du fichier racine pour plusieurs chemins et FileDataID en un seul parcours :
    /// (indice dans `paths`, ou `paths.len() + j` pour `fdids[j]`, entrée).
    pub fn find_batch(&mut self, paths: &[String], fdids: &[u32]) -> Result<Vec<(usize, Hit)>, String> {
        // Deux hash par chemin (ordre des moitiés selon la version) : ramenés à l'indice du chemin.
        let want: Vec<u64> = paths.iter().flat_map(|p| path_hashes(p)).collect();
        let (_, _, hits) = scan_root(&self.root, &want, fdids)?;
        Ok(hits
            .into_iter()
            .map(|(w, h)| if w < want.len() { (w / 2, h) } else { (w - want.len() + paths.len(), h) })
            .collect())
    }

    #[cfg(test)]
    pub fn find(&mut self, path: &str, locale: u32) -> Result<Option<Hit>, String> {
        Ok(pick(&self.find_all(path)?, locale))
    }
}

// --- Stockage synthétique (tests et validation croisée) -------------------------

pub mod fixture {
    //! Construit une fausse installation de WoW au format CASC local : sert aux tests,
    //! et à la validation croisée du format avec une bibliothèque indépendante.

    use super::*;
    use std::io::Write;

    pub struct Entry {
        pub path: Option<String>,
        pub fdid: u32,
        pub locale: u32,
        pub content: Vec<u8>,
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(data).unwrap();
        z.finish().unwrap()
    }

    /// Encode en BLTE : 0 = bloc unique brut, 1 = bloc unique zlib, 2+ = blocs multiples.
    pub fn blte(data: &[u8], style: usize) -> Vec<u8> {
        let mut out = b"BLTE".to_vec();
        match style {
            0 | 1 => {
                out.extend_from_slice(&0u32.to_be_bytes());
                if style == 0 {
                    out.push(b'N');
                    out.extend_from_slice(data);
                } else {
                    out.push(b'Z');
                    out.extend(zlib(data));
                }
            }
            _ => {
                let chunks: Vec<&[u8]> = if data.is_empty() {
                    vec![data]
                } else {
                    data.chunks(700).collect()
                };
                let blocks: Vec<Vec<u8>> = chunks
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let mut b = vec![if i % 2 == 0 { b'Z' } else { b'N' }];
                        if i % 2 == 0 {
                            b.extend(zlib(c));
                        } else {
                            b.extend_from_slice(c);
                        }
                        b
                    })
                    .collect();
                let hsize = 12 + 24 * blocks.len();
                out.extend_from_slice(&(hsize as u32).to_be_bytes());
                out.push(0x0F);
                out.extend_from_slice(&(blocks.len() as u32).to_be_bytes()[1..]);
                for (b, c) in blocks.iter().zip(&chunks) {
                    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                    out.extend_from_slice(&(c.len() as u32).to_be_bytes());
                    out.extend_from_slice(&md5(b));
                }
                for b in &blocks {
                    out.extend_from_slice(b);
                }
            }
        }
        out
    }

    /// Entrées de démonstration : polices en anglais et en français, fichiers de remplissage.
    pub fn default_entries() -> Vec<Entry> {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut v = vec![
            Entry {
                path: Some("Fonts\\ARIALN.TTF".into()),
                fdid: 1_000,
                locale: 0x2,
                content: b"ARIALN enUS".iter().chain(&rng.bytes(5000)).copied().collect(),
            },
            Entry {
                path: Some("Fonts\\ARIALN.TTF".into()),
                fdid: 1_000,
                locale: 0x10,
                content: b"ARIALN frFR".iter().chain(&rng.bytes(4000)).copied().collect(),
            },
            Entry {
                path: Some("Fonts\\FRIZQT__.TTF".into()),
                fdid: 1_001,
                locale: 0xFFFF_FFFF,
                content: b"FRIZQT".iter().chain(&rng.bytes(3000)).copied().collect(),
            },
        ];
        let mut fdid = 2_000;
        for i in 0..400 {
            fdid += 1 + (rng.next() % 3) as u32;
            let n = (rng.next() % 2500) as usize;
            v.push(Entry {
                path: if i < 300 {
                    Some(format!("World\\Fake\\File{i:04}.bin"))
                } else {
                    None
                },
                fdid,
                locale: if i % 2 == 0 { 0x2 } else { 0xFFFF_FFFF },
                content: rng.bytes(n),
            });
        }
        v
    }

    /// Écrit l'installation. Renvoie la CKey de chaque entrée, dans l'ordre de `entries`.
    pub fn build(dir: &Path, entries: &[Entry]) -> Result<Vec<Key>, String> {
        let io = |e: std::io::Error| e.to_string();
        let data_dir = dir.join("Data").join("data");
        fs::create_dir_all(&data_dir).map_err(io)?;

        // Blocs de la racine (MFST v2) : par langue et selon la présence de noms.
        let mut groups: Vec<((u32, bool), Vec<usize>)> = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let k = (e.locale, e.path.is_some());
            match groups.iter_mut().find(|(g, _)| *g == k) {
                Some((_, v)) => v.push(i),
                None => groups.push((k, vec![i])),
            }
        }
        let ckeys: Vec<Key> = entries.iter().map(|e| md5(&e.content)).collect();
        let mut root = Vec::new();
        root.extend_from_slice(&MFST_LE.to_le_bytes());
        root.extend_from_slice(&24u32.to_le_bytes());
        root.extend_from_slice(&2u32.to_le_bytes());
        root.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let named = entries.iter().filter(|e| e.path.is_some()).count();
        root.extend_from_slice(&(named as u32).to_le_bytes());
        root.extend_from_slice(&0u32.to_le_bytes());
        for ((locale, has_names), mut idx) in groups {
            idx.sort_by_key(|i| entries[*i].fdid);
            root.extend_from_slice(&(idx.len() as u32).to_le_bytes());
            root.extend_from_slice(&locale.to_le_bytes());
            let f1 = if has_names { 0 } else { NO_NAME_HASH };
            root.extend_from_slice(&f1.to_le_bytes());
            root.extend_from_slice(&0u32.to_le_bytes());
            root.push(0);
            let mut prev: i64 = -1;
            for i in &idx {
                let f = entries[*i].fdid as i64;
                let d = if prev < 0 { f } else { f - prev - 1 };
                root.extend_from_slice(&(d as i32).to_le_bytes());
                prev = f;
            }
            for i in &idx {
                root.extend_from_slice(&ckeys[*i]);
            }
            if has_names {
                for i in &idx {
                    let h = path_hashes(entries[*i].path.as_deref().unwrap())[0];
                    root.extend_from_slice(&h.to_le_bytes());
                }
            }
        }
        let root_ckey = md5(&root);

        // Blobs BLTE : (ckey, ekey, blte).
        let mut blobs: Vec<(Key, Key, Vec<u8>)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, e) in entries.iter().enumerate() {
            if seen.insert(ckeys[i]) {
                let b = blte(&e.content, i % 3);
                blobs.push((ckeys[i], md5(&b), b));
            }
        }
        let b = blte(&root, 2);
        blobs.push((root_ckey, md5(&b), b));

        // Encodage : pages de 1 Ko triées par CKey. La 2e police a deux EKeys,
        // la première absente du disque (repli à tester).
        let missing: Key = md5(b"absente");
        let mut ce: Vec<(Key, Vec<Key>, u64)> = blobs
            .iter()
            .map(|(c, e, _)| (*c, vec![*e], 0))
            .collect();
        for (i, e) in entries.iter().enumerate() {
            if let Some(x) = ce.iter_mut().find(|x| x.0 == ckeys[i]) {
                x.2 = e.content.len() as u64;
                if e.path.as_deref() == Some("Fonts\\FRIZQT__.TTF") && x.1.len() == 1 {
                    x.1.insert(0, missing);
                }
            }
        }
        if let Some(x) = ce.iter_mut().find(|x| x.0 == root_ckey) {
            x.2 = root.len() as u64;
        }
        ce.sort_by_key(|x| x.0);
        let page = 1024;
        let mut pages: Vec<Vec<u8>> = vec![Vec::new()];
        let mut firsts: Vec<Key> = Vec::new();
        for (c, eks, size) in &ce {
            let rec = 22 + 16 * eks.len();
            if pages.last().unwrap().len() + rec > page {
                pages.push(Vec::new());
            }
            let p = pages.last_mut().unwrap();
            if p.is_empty() {
                firsts.push(*c);
            }
            p.push(eks.len() as u8);
            p.extend_from_slice(&size.to_be_bytes()[3..]);
            p.extend_from_slice(c);
            for e in eks {
                p.extend_from_slice(e);
            }
        }
        for p in &mut pages {
            p.resize(page, 0);
        }
        let espec = b"z\0n\0";
        let mut enc = b"EN".to_vec();
        enc.extend_from_slice(&[1, 16, 16]);
        enc.extend_from_slice(&1u16.to_be_bytes());
        enc.extend_from_slice(&1u16.to_be_bytes());
        enc.extend_from_slice(&(pages.len() as u32).to_be_bytes());
        enc.extend_from_slice(&0u32.to_be_bytes());
        enc.push(0);
        enc.extend_from_slice(&(espec.len() as u32).to_be_bytes());
        enc.extend_from_slice(espec);
        for (f, p) in firsts.iter().zip(&pages) {
            enc.extend_from_slice(f);
            enc.extend_from_slice(&md5(p));
        }
        for p in &pages {
            enc.extend_from_slice(p);
        }
        let enc_ckey = md5(&enc);
        let enc_blte = blte(&enc, 2);
        let enc_ekey = md5(&enc_blte);
        blobs.push((enc_ckey, enc_ekey, enc_blte));

        // Archives data.000 et data.001, puis index : version 1 fausse, version 2 juste.
        let mut archives: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
        let mut locs: Vec<(Key, u32, u64, u32)> = Vec::new();
        let half = blobs.len() / 2;
        for (i, (_, ek, b)) in blobs.iter().enumerate() {
            let a = usize::from(i >= half);
            let off = archives[a].len() as u64;
            let mut rev = *ek;
            rev.reverse();
            archives[a].extend_from_slice(&rev);
            archives[a].extend_from_slice(&((30 + b.len()) as u32).to_le_bytes());
            archives[a].extend_from_slice(&[0u8; 10]);
            archives[a].extend_from_slice(b);
            locs.push((*ek, a as u32, off, (30 + b.len()) as u32));
        }
        for (a, d) in archives.iter().enumerate() {
            fs::write(data_dir.join(format!("data.{a:03}")), d).map_err(io)?;
        }
        for bkt in 0..16usize {
            let mut mine: Vec<&(Key, u32, u64, u32)> =
                locs.iter().filter(|l| bucket(&l.0) == bkt).collect();
            mine.sort_by(|x, y| x.0[..9].cmp(&y.0[..9]));
            for version in [1u32, 2] {
                let mut d = vec![0u8; 0x28];
                d[0..4].copy_from_slice(&0x10u32.to_le_bytes());
                d[8..10].copy_from_slice(&7u16.to_le_bytes());
                d[0x0A] = bkt as u8;
                d[0x0C] = 4;
                d[0x0D] = 5;
                d[0x0E] = 9;
                d[0x0F] = 30;
                d[0x10..0x18].copy_from_slice(&0x40_0000_0000u64.to_le_bytes());
                d[0x20..0x24].copy_from_slice(&((mine.len() * 18) as u32).to_le_bytes());
                for (ek, a, off, size) in &mine {
                    d.extend_from_slice(&ek[..9]);
                    let a = if version == 1 { 9 } else { *a };
                    let raw = ((a as u64) << 30) | off;
                    d.extend_from_slice(&raw.to_be_bytes()[3..]);
                    d.extend_from_slice(&size.to_le_bytes());
                }
                fs::write(data_dir.join(format!("{bkt:02x}{version:08x}.idx")), d)
                    .map_err(io)?;
            }
        }

        // Configuration de build et .build.info (un produit retail sans configuration,
        // pour vérifier le choix par produit).
        let bk = hex(&md5(b"build fixture"));
        let cfg_dir = dir.join("Data").join("config").join(&bk[0..2]).join(&bk[2..4]);
        fs::create_dir_all(&cfg_dir).map_err(io)?;
        fs::write(
            cfg_dir.join(&bk),
            format!(
                "# Build Configuration\n\nroot = {}\ninstall = {} {}\nencoding = {} {}\nencoding-size = {} {}\nbuild-name = FIXTURE\n",
                hex(&root_ckey),
                hex(&[0u8; 16]),
                hex(&[0u8; 16]),
                hex(&enc_ckey),
                hex(&enc_ekey),
                enc.len(),
                enc.len()
            ),
        )
        .map_err(io)?;
        let other = hex(&md5(b"retail"));
        let z = hex(&[0u8; 16]);
        fs::write(
            dir.join(".build.info"),
            format!(
                "Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|CDN Key!HEX:16|Install Key!HEX:16|IM Size!DEC:4|CDN Path!STRING:0|CDN Hosts!STRING:0|CDN Servers!STRING:0|Tags!STRING:0|Armadillo!STRING:0|Last Activated!STRING:0|Version!STRING:0|KeyRing!HEX:16|Product!STRING:0\n\
                 eu|1|{other}|{z}|{z}||tpr/wow|eu.cdn||Windows x86_64 EU? enUS speech?:Windows x86_64 EU? enUS text?|||12.1.5.66789||wow\n\
                 eu|1|{bk}|{z}|{z}||tpr/wow|eu.cdn||Windows x86_64 EU? frFR speech?:Windows x86_64 EU? frFR text?|||1.60.1.70862||wowf\n"
            ),
        )
        .map_err(io)?;
        let wtf = dir.join("_forever_").join("WTF");
        fs::create_dir_all(&wtf).map_err(io)?;
        fs::write(
            wtf.join("Config.wtf"),
            "SET portal \"EU\"\nSET textLocale \"frFR\"\nSET audioLocale \"frFR\"\n",
        )
        .map_err(io)?;
        Ok(ckeys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup3_reference_vectors() {
        // Valeurs du programme de test de lookup3.c (driver5).
        assert_eq!(hashlittle2(b"", 0, 0), (0xdeadbeef, 0xdeadbeef));
        assert_eq!(hashlittle2(b"", 0, 0xdeadbeef), (0xbd5b7dde, 0xdeadbeef));
        assert_eq!(
            hashlittle2(b"", 0xdeadbeef, 0xdeadbeef),
            (0x9c093ccd, 0xbd5b7dde)
        );
        let s = b"Four score and seven years ago";
        assert_eq!(hashlittle2(s, 0, 0), (0x17770551, 0xce7226e6));
        assert_eq!(hashlittle2(s, 0, 1), (0xe3607cae, 0xbd371de4));
        assert_eq!(hashlittle2(s, 1, 0), (0xcd628161, 0x6cbea4b3));
    }

    #[test]
    fn path_normalization() {
        assert_eq!(path_hashes("fonts/arialn.ttf"), path_hashes("Fonts\\ARIALN.TTF"));
        let [a, b] = path_hashes("Fonts\\ARIALN.TTF");
        assert_eq!(a.rotate_left(32), b);
    }

    #[test]
    fn blte_styles_roundtrip() {
        for n in [0usize, 1, 699, 700, 701, 5000] {
            let data: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
            for style in 0..3 {
                assert_eq!(decode_blte(&fixture::blte(&data, style)).unwrap(), data);
            }
        }
        assert!(decode_blte(b"BLTX\0\0\0\0").is_err());
        assert!(decode_blte(b"BLTE\0\0\0\0E....").is_err());
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ct-casc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn fixture_storage_reads_every_named_file() {
        let dir = temp("all");
        let entries = fixture::default_entries();
        let ckeys = fixture::build(&dir, &entries).unwrap();
        let mut s = Storage::open(&dir, &["wowf"]).unwrap();
        assert_eq!(s.product, "wowf");
        assert_eq!(s.version, "1.60.1.70862");
        assert_eq!(s.root_format, RootFormat::MfstV2);
        assert_eq!(s.root_stats.entries, entries.len());
        assert_eq!(s.root_stats.named, 303);

        // Police : la variante de la langue du client, sinon l'anglais.
        let fr = s.find("Fonts\\ARIALN.TTF", 0x10).unwrap().unwrap();
        assert_eq!(fr.fdid, 1000);
        assert!(s.read_ckey(&fr.ckey).unwrap().starts_with(b"ARIALN frFR"));
        let de = s.find("fonts/arialn.ttf", 0x20).unwrap().unwrap();
        assert!(s.read_ckey(&de.ckey).unwrap().starts_with(b"ARIALN enUS"));
        assert_eq!(s.hash_order, Some(0));
        // Première EKey absente du disque : repli sur la seconde.
        let fq = s.find("Fonts\\FRIZQT__.TTF", 0x10).unwrap().unwrap();
        assert!(s.read_ckey(&fq.ckey).unwrap().starts_with(b"FRIZQT"));
        assert!(s.find("Fonts\\ABSENTE.TTF", 0x10).unwrap().is_none());
        // Par FileDataID : variantes de langue, et fichier d'un bloc sans noms.
        assert_eq!(s.find_fdid(1000).unwrap().len(), 2);
        let unnamed = entries.iter().zip(&ckeys).find(|(e, _)| e.path.is_none()).unwrap();
        let h = pick(&s.find_fdid(unnamed.0.fdid).unwrap(), 0x10).unwrap();
        assert_eq!(h.ckey, *unnamed.1);
        assert_eq!(s.read_ckey(&h.ckey).unwrap(), unnamed.0.content);
        assert!(s.find_fdid(999_999).unwrap().is_empty());

        for (e, ck) in entries.iter().zip(&ckeys) {
            if let Some(p) = &e.path {
                let h = s.find(p, e.locale).unwrap().unwrap();
                assert_eq!(h.fdid, e.fdid, "{p}");
                if p.starts_with("World") {
                    assert_eq!(h.ckey, *ck);
                    assert_eq!(s.read_ckey(ck).unwrap(), e.content);
                }
            }
        }
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn product_must_exist_and_corruption_is_detected() {
        let dir = temp("bad");
        let entries = fixture::default_entries();
        fixture::build(&dir, &entries).unwrap();
        let err = Storage::open(&dir, &["wow_classic"]).err().unwrap();
        assert!(err.contains("wowf"), "{err}");
        // Le produit retail existe mais sa configuration de build manque.
        assert!(Storage::open(&dir, &["wow"]).is_err());

        let mut s = Storage::open(&dir, &["wowf"]).unwrap();
        let h = s.find("Fonts\\ARIALN.TTF", 0x10).unwrap().unwrap();
        let ekey = encoding_find(&s.encoding, &h.ckey).unwrap()[0];
        let (a, off, _) = s.locate(&ekey).unwrap().unwrap();
        // Octet modifié dans le contenu brut (bloc 'N' ou 'Z') : refusé.
        let path = dir.join("Data").join("data").join(format!("data.{a:03}"));
        let mut d = fs::read(&path).unwrap();
        let i = off as usize + 30 + 8 + 1 + 20;
        d[i] ^= 0xFF;
        fs::write(&path, d).unwrap();
        assert!(s.read_ckey(&h.ckey).is_err());
        fs::remove_dir_all(dir).ok();
    }
}
