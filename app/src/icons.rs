//! Icônes du chat miroir (0.12.3) : textures `|T...|t` et atlas `|A...|a` du jeu (rang de qualité
//! des objets d'artisanat, marques de raid, pièces...), dessinées à leur taille dans WoW.
//!
//! Les images viennent des archives du jeu (BLP, lues par `casc`) et sont gardées en cache sur le
//! disque ; un atlas est décrit par l'addon (`C_Texture.GetAtlasInfo` : fichier et coordonnées).
//! En attendant l'image (ou sans accès aux archives), une forme dessinée la remplace : marque de
//! raid, gemme de qualité, pièce, ou un carré discret.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::casc::{self, Storage};

// --- Image ------------------------------------------------------------------------------------

/// Image RGBA (alpha non prémultiplié), lignes de haut en bas.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 4]>,
}

fn le32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

fn mix(a: [u8; 3], b: [u8; 3], wa: u32, wb: u32) -> [u8; 3] {
    let t = wa + wb;
    [
        ((a[0] as u32 * wa + b[0] as u32 * wb) / t) as u8,
        ((a[1] as u32 * wa + b[1] as u32 * wb) / t) as u8,
        ((a[2] as u32 * wa + b[2] as u32 * wb) / t) as u8,
    ]
}

/// Bloc couleur DXT (8 octets) : 16 pixels. `four` : toujours 4 couleurs (DXT3/5).
fn dxt_colors(b: &[u8], four: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let pal: [[u8; 4]; 4] = if four || c0 > c1 {
        let p2 = mix(p0, p1, 2, 1);
        let p3 = mix(p0, p1, 1, 2);
        [[p0[0], p0[1], p0[2], 255], [p1[0], p1[1], p1[2], 255], [p2[0], p2[1], p2[2], 255], [p3[0], p3[1], p3[2], 255]]
    } else {
        let p2 = mix(p0, p1, 1, 1);
        [[p0[0], p0[1], p0[2], 255], [p1[0], p1[1], p1[2], 255], [p2[0], p2[1], p2[2], 255], [0, 0, 0, 0]]
    };
    let idx = le32(b, 4);
    let mut out = [[0u8; 4]; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((idx >> (2 * i)) & 3) as usize];
    }
    out
}

fn dxt5_alpha(b: &[u8]) -> [u8; 16] {
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut pal = [0u32; 8];
    pal[0] = a0;
    pal[1] = a1;
    if a0 > a1 {
        for (i, p) in pal.iter_mut().enumerate().skip(2) {
            *p = ((8 - i as u32) * a0 + (i as u32 - 1) * a1) / 7;
        }
    } else {
        for (i, p) in pal.iter_mut().enumerate().take(6).skip(2) {
            *p = ((6 - i as u32) * a0 + (i as u32 - 1) * a1) / 5;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits: u64 = 0;
    for k in 0..6 {
        bits |= (b[2 + k] as u64) << (8 * k);
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((bits >> (3 * i)) & 7) as usize] as u8;
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Dxt {
    One,
    Three,
    Five,
}

fn decode_dxt(d: &[u8], w: usize, h: usize, kind: Dxt) -> Result<Vec<[u8; 4]>, String> {
    let bsize = if kind == Dxt::One { 8 } else { 16 };
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    if d.len() < bw * bh * bsize {
        return Err("BLP : données DXT tronquées".into());
    }
    let mut px = vec![[0u8; 4]; w * h];
    for by in 0..bh {
        for bx in 0..bw {
            let b = &d[(by * bw + bx) * bsize..][..bsize];
            let (cb, alpha): (&[u8], Option<[u8; 16]>) = match kind {
                Dxt::One => (b, None),
                Dxt::Three => {
                    let mut a = [0u8; 16];
                    for (i, v) in a.iter_mut().enumerate() {
                        *v = ((b[i / 2] >> (4 * (i % 2))) & 15) * 17;
                    }
                    (&b[8..], Some(a))
                }
                Dxt::Five => (&b[8..], Some(dxt5_alpha(b))),
            };
            let colors = dxt_colors(cb, kind != Dxt::One);
            for i in 0..16 {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < w && y < h {
                    let mut c = colors[i];
                    if let Some(a) = alpha {
                        c[3] = a[i];
                    }
                    px[y * w + x] = c;
                }
            }
        }
    }
    Ok(px)
}

/// Décode le premier niveau d'une image BLP2 (palette, DXT1/3/5 ou BGRA).
pub fn decode_blp(d: &[u8]) -> Result<Rgba, String> {
    if d.len() < 148 || &d[..4] != b"BLP2" {
        return Err("pas une image BLP2".into());
    }
    if le32(d, 4) != 1 {
        return Err("BLP compressée en JPEG (non prise en charge)".into());
    }
    let (comp, alpha_depth, alpha_type) = (d[8], d[9], d[10]);
    let (w, h) = (le32(d, 12) as usize, le32(d, 16) as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err(format!("BLP : taille {w}x{h} refusée"));
    }
    let off = le32(d, 20) as usize;
    let size = le32(d, 84) as usize;
    let data = d.get(off..off.saturating_add(size).min(d.len())).ok_or("BLP : niveau 0 hors du fichier")?;
    let px = match comp {
        1 => {
            if d.len() < 148 + 1024 || data.len() < w * h {
                return Err("BLP : palette tronquée".into());
            }
            let pal = |i: usize| {
                let o = 148 + 4 * i;
                [d[o + 2], d[o + 1], d[o]]
            };
            let a = &data[w * h..];
            let alpha = |i: usize| -> u8 {
                match alpha_depth {
                    1 => a.get(i / 8).map_or(255, |b| if (b >> (i % 8)) & 1 != 0 { 255 } else { 0 }),
                    4 => a.get(i / 2).map_or(255, |b| ((b >> (4 * (i % 2))) & 15) * 17),
                    8 => a.get(i).copied().unwrap_or(255),
                    _ => 255,
                }
            };
            (0..w * h)
                .map(|i| {
                    let c = pal(data[i] as usize);
                    [c[0], c[1], c[2], alpha(i)]
                })
                .collect()
        }
        2 => {
            let kind = match (alpha_depth, alpha_type) {
                (_, 7) => Dxt::Five,
                (0 | 1, _) => Dxt::One,
                _ => Dxt::Three,
            };
            decode_dxt(data, w, h, kind)?
        }
        3 => {
            if data.len() < w * h * 4 {
                return Err("BLP : données BGRA tronquées".into());
            }
            data.chunks_exact(4).take(w * h).map(|c| [c[2], c[1], c[0], c[3]]).collect()
        }
        c => return Err(format!("BLP : compression {c} inconnue")),
    };
    Ok(Rgba { w, h, px })
}

/// Image mise à la taille `tw` x `th`, partie `crop` (gauche, droite, haut, bas, de 0 à 1) :
/// moyenne de points pris sur chaque pixel d'arrivée (couleurs prémultipliées : pas de liseré).
pub fn scale(src: &Rgba, crop: [f32; 4], tw: usize, th: usize) -> Rgba {
    let (sw, sh) = (src.w as f32, src.h as f32);
    let (cw, ch) = ((crop[1] - crop[0]) * sw, (crop[3] - crop[2]) * sh);
    let n = ((cw.abs() / tw.max(1) as f32).ceil() as usize).clamp(1, 8).max(((ch.abs() / th.max(1) as f32).ceil() as usize).clamp(1, 8));
    // Pixels de la partie découpée : le filtre ne déborde pas sur l'icône voisine de l'atlas.
    let lim = |a: f32, b: f32, size: f32| {
        let lo = (a.min(b) * size).floor().clamp(0.0, size - 1.0);
        let hi = ((a.max(b) * size).ceil() - 1.0).clamp(lo, size - 1.0);
        (lo, hi)
    };
    let (xlo, xhi) = lim(crop[0], crop[1], sw);
    let (ylo, yhi) = lim(crop[2], crop[3], sh);
    let at = |x: f32, y: f32| -> [f32; 4] {
        // Bilinéaire, bords répétés.
        let x = (x - 0.5).clamp(xlo, xhi);
        let y = (y - 0.5).clamp(ylo, yhi);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(src.w - 1), (y0 + 1).min(src.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let mut out = [0f32; 4];
        for (xx, yy, wgt) in [(x0, y0, (1.0 - fx) * (1.0 - fy)), (x1, y0, fx * (1.0 - fy)), (x0, y1, (1.0 - fx) * fy), (x1, y1, fx * fy)] {
            let p = src.px[yy * src.w + xx];
            let a = p[3] as f32 / 255.0;
            out[0] += p[0] as f32 * a * wgt;
            out[1] += p[1] as f32 * a * wgt;
            out[2] += p[2] as f32 * a * wgt;
            out[3] += a * wgt;
        }
        out
    };
    let mut px = Vec::with_capacity(tw * th);
    for y in 0..th {
        for x in 0..tw {
            let mut acc = [0f32; 4];
            for sy in 0..n {
                for sx in 0..n {
                    let u = (x as f32 + (sx as f32 + 0.5) / n as f32) / tw as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / n as f32) / th as f32;
                    let p = at((crop[0] + u * (crop[1] - crop[0])) * sw, (crop[2] + v * (crop[3] - crop[2])) * sh);
                    for k in 0..4 {
                        acc[k] += p[k];
                    }
                }
            }
            let m = (n * n) as f32;
            let a = acc[3] / m;
            let c = |v: f32| if a > 0.0 { (v / m / a).round().clamp(0.0, 255.0) as u8 } else { 0 };
            px.push([c(acc[0]), c(acc[1]), c(acc[2]), (a * 255.0).round() as u8]);
        }
    }
    Rgba { w: tw, h: th, px }
}

// --- Balisage --------------------------------------------------------------------------------

/// D'où vient l'image d'une icône.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Chemin du jeu (« Interface\TargetingFrame\UI-RaidTargetingIcon_1 »).
    Path(String),
    Fdid(u32),
    /// Atlas : fichier et coordonnées donnés par l'addon.
    Atlas(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub src: Source,
    /// Partie de l'image (gauche, droite, haut, bas, de 0 à 1) donnée par le balisage |T.
    pub crop: Option<[f32; 4]>,
}

/// Icône placée dans une ligne : identifiant de sa description, taille et décalage en unités de
/// l'interface (hauteur 0 : celle de la police ; largeur 0 : celle de la hauteur, ou proportions de
/// l'atlas pour |A).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconRef {
    pub id: u32,
    pub h: f32,
    pub w: f32,
    pub dx: f32,
    pub dy: f32,
}

/// Atlas décrit par l'addon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atlas {
    pub fdid: u32,
    pub crop: [f32; 4],
    pub w: u16,
    pub h: u16,
}

fn num(s: Option<&&str>) -> f32 {
    s.and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0)
}

/// Contenu de `|T...|t` (sans les balises).
pub fn parse_texture(inner: &str) -> IconRef {
    let f: Vec<&str> = inner.split(':').collect();
    let path = f.first().copied().unwrap_or_default().trim();
    let src = match path.parse::<u32>() {
        Ok(id) => Source::Fdid(id),
        Err(_) => Source::Path(path.to_string()),
    };
    let (tw, th) = (num(f.get(5)), num(f.get(6)));
    let crop = (tw > 0.0 && th > 0.0 && f.len() >= 11).then(|| [num(f.get(7)) / tw, num(f.get(8)) / tw, num(f.get(9)) / th, num(f.get(10)) / th]);
    let h = num(f.get(1));
    let w = if f.len() >= 3 { num(f.get(2)) } else { h };
    IconRef { id: intern(Spec { src, crop }), h, w, dx: num(f.get(3)), dy: num(f.get(4)) }
}

/// Contenu de `|A...|a` (sans les balises) : « :atlas:hauteur:largeur:dx:dy ».
pub fn parse_atlas(inner: &str) -> IconRef {
    let f: Vec<&str> = inner.trim_start_matches(':').split(':').collect();
    let name = f.first().copied().unwrap_or_default().trim().to_string();
    IconRef { id: intern(Spec { src: Source::Atlas(name), crop: None }), h: num(f.get(1)), w: num(f.get(2)), dx: num(f.get(3)), dy: num(f.get(4)) }
}

// --- État partagé ----------------------------------------------------------------------------

enum Slot {
    Pending,
    Ready(Arc<Rgba>),
    Failed,
}

#[derive(Default)]
struct Shared {
    specs: Vec<Spec>,
    keys: HashMap<String, u32>,
    atlases: HashMap<String, Atlas>,
    /// Chemin -> FileDataID donné par l'addon (chemin sans nom dans les archives).
    path_ids: HashMap<String, u32>,
    images: HashMap<Load, Slot>,
    scaled: HashMap<(u32, u16, u16), Arc<Rgba>>,
    changed: bool,
    tx: Option<Sender<Load>>,
    loaded: usize,
    failed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Load {
    Fdid(u32),
    Path(String),
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Shared> {
    shared().lock().unwrap_or_else(|e| e.into_inner())
}

fn key_of(s: &Spec) -> String {
    let c = s.crop.map(|c| format!("{:.4},{:.4},{:.4},{:.4}", c[0], c[1], c[2], c[3])).unwrap_or_default();
    match &s.src {
        Source::Path(p) => format!("p:{}|{c}", p.to_ascii_lowercase()),
        Source::Fdid(f) => format!("f:{f}|{c}"),
        Source::Atlas(a) => format!("a:{}", a.to_ascii_lowercase()),
    }
}

pub fn intern(s: Spec) -> u32 {
    let k = key_of(&s);
    let mut g = lock();
    if let Some(id) = g.keys.get(&k) {
        return *id;
    }
    let id = g.specs.len() as u32;
    g.specs.push(s);
    g.keys.insert(k, id);
    id
}

pub fn spec(id: u32) -> Option<Spec> {
    lock().specs.get(id as usize).cloned()
}

/// Atlas ou chemin décrit par l'addon (trame d'icône).
pub fn learn_atlas(name: &str, a: Atlas) {
    let mut g = lock();
    if g.atlases.get(&name.to_ascii_lowercase()) != Some(&a) {
        g.atlases.insert(name.to_ascii_lowercase(), a);
        g.scaled.clear();
        g.changed = true;
    }
}

pub fn learn_path(path: &str, fdid: u32) {
    if fdid == 0 {
        return;
    }
    let mut g = lock();
    if g.path_ids.insert(path.to_ascii_lowercase(), fdid) != Some(fdid) {
        g.changed = true;
    }
}

/// Trame d'icône de l'addon : nature (0 atlas, 1 chemin), nom, FileDataID, coordonnées ×65535,
/// largeur et hauteur de l'atlas.
pub fn on_frame(p: &[u8]) -> Result<String, String> {
    let kind = *p.first().ok_or("trame d'icône vide")?;
    let n = *p.get(1).ok_or("trame d'icône tronquée")? as usize;
    if p.len() < 2 + n + 16 {
        return Err("trame d'icône tronquée".into());
    }
    let name = String::from_utf8_lossy(&p[2..2 + n]).into_owned();
    let o = 2 + n;
    let fdid = u32::from_be_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let c = |i: usize| u16::from_be_bytes([p[o + 4 + 2 * i], p[o + 5 + 2 * i]]) as f32 / 65535.0;
    let u = |i: usize| u16::from_be_bytes([p[o + i], p[o + i + 1]]);
    match kind {
        0 => {
            let a = Atlas { fdid, crop: [c(0), c(1), c(2), c(3)], w: u(12), h: u(14) };
            learn_atlas(&name, a);
            Ok(format!("atlas {name} : fichier {fdid}, {}x{}", a.w, a.h))
        }
        _ => {
            learn_path(&name, fdid);
            Ok(format!("texture {name} : fichier {fdid}"))
        }
    }
}

/// Une image nouvelle est prête depuis le dernier appel (le miroir se redessine).
pub fn take_changed() -> bool {
    std::mem::take(&mut lock().changed)
}

/// Taille en pixels d'une icône (`px` : taille de la police en pixels, `k` : pixels par unité de
/// l'interface).
pub fn size_px(r: &IconRef, px: f32, k: f32) -> (f32, f32) {
    let h = if r.h > 0.0 { r.h * k } else { px };
    let w = if r.w > 0.0 {
        r.w * k
    } else {
        let aspect = match spec(r.id).map(|s| s.src) {
            Some(Source::Atlas(a)) => lock().atlases.get(&a.to_ascii_lowercase()).filter(|a| a.h > 0).map(|a| a.w as f32 / a.h as f32),
            _ => None,
        };
        h * aspect.unwrap_or(1.0)
    };
    (w.clamp(1.0, 256.0), h.clamp(1.0, 256.0))
}

/// Image de l'icône à la taille voulue, si elle est prête (sinon son chargement est demandé).
pub fn image(id: u32, w: usize, h: usize) -> Option<Arc<Rgba>> {
    let mut g = lock();
    if let Some(i) = g.scaled.get(&(id, w as u16, h as u16)) {
        return Some(i.clone());
    }
    let s = g.specs.get(id as usize)?.clone();
    let (load, crop) = match &s.src {
        Source::Fdid(f) => (Load::Fdid(*f), s.crop),
        Source::Path(p) => match g.path_ids.get(&p.to_ascii_lowercase()) {
            Some(f) => (Load::Fdid(*f), s.crop),
            None => (Load::Path(p.clone()), s.crop),
        },
        Source::Atlas(a) => {
            let at = *g.atlases.get(&a.to_ascii_lowercase())?;
            if at.fdid == 0 {
                return None;
            }
            (Load::Fdid(at.fdid), Some(at.crop))
        }
    };
    match g.images.get(&load) {
        Some(Slot::Ready(img)) => {
            let img = img.clone();
            drop(g);
            let out = Arc::new(scale(&img, crop.unwrap_or([0.0, 1.0, 0.0, 1.0]), w.max(1), h.max(1)));
            let mut g = lock();
            if g.scaled.len() > 2000 {
                g.scaled.clear();
            }
            g.scaled.insert((id, w as u16, h as u16), out.clone());
            Some(out)
        }
        Some(_) => None,
        None => {
            if let Some(tx) = g.tx.clone() {
                g.images.insert(load.clone(), Slot::Pending);
                let _ = tx.send(load);
            }
            None
        }
    }
}

// --- Chargement depuis les archives du jeu ----------------------------------------------------

fn cache_dir(wow: &Path, flavor: &str) -> Option<PathBuf> {
    let products = crate::install::products_for(flavor)?;
    let rows = crate::install::build_info_rows(wow);
    let row = rows
        .iter()
        .filter(|r| r.get("product").is_some_and(|p| products.contains(&p.as_str())))
        .max_by_key(|r| r.get("active").map(String::as_str) != Some("0"))?;
    let get = |k: &str| row.get(k).cloned().unwrap_or_default();
    Some(crate::fonts::cache_root().join("icones").join(format!("{}-{}", get("product"), get("version"))))
}

fn file_name(l: &Load) -> String {
    match l {
        Load::Fdid(f) => format!("f{f}.rgba"),
        Load::Path(p) => {
            let (a, b) = casc::hashlittle2(casc::normalize_path(p).as_bytes(), 0, 0);
            format!("p{a:08x}{b:08x}.rgba")
        }
    }
}

fn write_cached(path: &Path, img: &Rgba) {
    let mut b = Vec::with_capacity(8 + img.px.len() * 4);
    b.extend_from_slice(&(img.w as u32).to_le_bytes());
    b.extend_from_slice(&(img.h as u32).to_le_bytes());
    for p in &img.px {
        b.extend_from_slice(p);
    }
    let _ = std::fs::write(path, b);
}

fn read_cached(path: &Path) -> Option<Rgba> {
    let d = std::fs::read(path).ok()?;
    if d.len() < 8 {
        return None;
    }
    let (w, h) = (le32(&d, 0) as usize, le32(&d, 4) as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 || d.len() != 8 + w * h * 4 {
        return None;
    }
    Some(Rgba { w, h, px: d[8..].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect() })
}

/// Chemin du jeu tel que rangé dans les archives (extension .blp ajoutée si absente).
fn archive_path(p: &str) -> String {
    let last = p.rsplit(['\\', '/']).next().unwrap_or(p);
    if last.contains('.') {
        p.to_string()
    } else {
        format!("{p}.blp")
    }
}

/// Lance le chargement des images en tâche de fond (archives de `wow`, version `flavor`).
pub fn start(wow: PathBuf, flavor: String) {
    let (tx, rx) = mpsc::channel::<Load>();
    lock().tx = Some(tx);
    std::thread::spawn(move || loader(wow, flavor, rx));
}

fn finish(l: Load, r: Result<Rgba, String>) {
    let mut g = lock();
    match r {
        Ok(img) => {
            g.images.insert(l, Slot::Ready(Arc::new(img)));
            g.loaded += 1;
        }
        Err(e) => {
            g.failed += 1;
            // Les premiers échecs seulement (console lisible).
            if g.failed <= 10 {
                crate::say!("[icône] {} : {e} (forme de remplacement)", match &l {
                    Load::Fdid(f) => format!("fichier {f}"),
                    Load::Path(p) => p.clone(),
                });
            }
            g.images.insert(l, Slot::Failed);
        }
    }
    g.changed = true;
}

fn loader(wow: PathBuf, flavor: String, rx: Receiver<Load>) {
    let dir = cache_dir(&wow, &flavor);
    if let Some(d) = &dir {
        let _ = std::fs::create_dir_all(d);
    }
    let mut storage: Option<Storage> = None;
    let mut last_use = Instant::now();
    loop {
        let first = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(l) => l,
            Err(RecvTimeoutError::Timeout) => {
                // Archives refermées après 2 min sans demande (mémoire).
                if storage.is_some() && last_use.elapsed() > Duration::from_secs(120) {
                    storage = None;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Les demandes arrivées ensemble partent en un seul parcours des archives.
        std::thread::sleep(Duration::from_millis(30));
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut todo = Vec::new();
        for l in batch {
            match dir.as_ref().and_then(|d| read_cached(&d.join(file_name(&l)))) {
                Some(img) => finish(l, Ok(img)),
                None => todo.push(l),
            }
        }
        if todo.is_empty() {
            continue;
        }
        last_use = Instant::now();
        if storage.is_none() {
            let products = crate::install::products_for(&flavor).unwrap_or(&["wow"]);
            match Storage::open(&wow, products) {
                Ok(s) => storage = Some(s),
                Err(e) => {
                    for l in todo {
                        finish(l, Err(format!("archives du jeu : {e}")));
                    }
                    continue;
                }
            }
        }
        let st = storage.as_mut().unwrap();
        let paths: Vec<String> = todo.iter().filter_map(|l| if let Load::Path(p) = l { Some(archive_path(p)) } else { None }).collect();
        let fdids: Vec<u32> = todo.iter().filter_map(|l| if let Load::Fdid(f) = l { Some(*f) } else { None }).collect();
        let hits = match st.find_batch(&paths, &fdids) {
            Ok(h) => h,
            Err(e) => {
                for l in todo {
                    finish(l, Err(e.clone()));
                }
                continue;
            }
        };
        let mut pi = 0usize;
        for l in todo {
            // Par chemin : ses entrées ; par FileDataID : toute entrée de ce fichier (une entrée
            // trouvée par son nom n'est rendue qu'une fois).
            let mine: Vec<casc::Hit> = match &l {
                Load::Path(_) => {
                    pi += 1;
                    hits.iter().filter(|(i, _)| *i == pi - 1).map(|(_, h)| *h).collect()
                }
                Load::Fdid(f) => hits.iter().filter(|(_, h)| h.fdid == *f).map(|(_, h)| *h).collect(),
            };
            let r = match casc::pick(&mine, 0x2) {
                None => Err("absente des archives du jeu".to_string()),
                Some(h) => st.read_ckey(&h.ckey).and_then(|d| decode_blp(&d)),
            };
            if let (Ok(img), Some(d)) = (&r, &dir) {
                write_cached(&d.join(file_name(&l)), img);
            }
            finish(l, r);
        }
    }
}

// --- Formes de remplacement ------------------------------------------------------------------

/// Forme dessinée à la place d'une image absente.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Marque de raid 1 à 8 (étoile, cercle, losange, triangle, lune, carré, croix, crâne).
    Raid(u8),
    /// Rang de qualité d'un objet d'artisanat.
    Quality([u8; 3]),
    Coin([u8; 3]),
    Unknown,
}

fn digit_after(s: &str, pat: &str) -> Option<u8> {
    let i = s.find(pat)? + pat.len();
    s[i..].chars().next()?.to_digit(10).map(|d| d as u8)
}

pub fn shape_of(id: u32) -> Shape {
    let Some(s) = spec(id) else { return Shape::Unknown };
    let name = match &s.src {
        Source::Path(p) => p.to_ascii_lowercase(),
        Source::Atlas(a) => a.to_ascii_lowercase(),
        Source::Fdid(_) => String::new(),
    };
    if let Some(n) = digit_after(&name, "raidtargetingicon_").filter(|n| (1..=8).contains(n)) {
        return Shape::Raid(n);
    }
    if let Some(n) = digit_after(&name, "quality-12-tier") {
        return Shape::Quality(if n <= 1 { [200, 205, 215] } else { [255, 205, 60] });
    }
    if let Some(n) = digit_after(&name, "quality-tier") {
        return Shape::Quality(match n {
            1 => [205, 127, 50],
            2 => [200, 205, 215],
            3 => [255, 205, 60],
            4 => [90, 225, 170],
            _ => [255, 140, 30],
        });
    }
    if name.contains("goldicon") || name.contains("coin-gold") {
        return Shape::Coin([255, 210, 40]);
    }
    if name.contains("silvericon") || name.contains("coin-silver") {
        return Shape::Coin([200, 200, 210]);
    }
    if name.contains("coppericon") || name.contains("coin-copper") {
        return Shape::Coin([200, 110, 50]);
    }
    Shape::Unknown
}

fn in_poly(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Couleur (et opacité) de la forme au point (x, y) du carré unité, s'il est dedans.
pub fn shape_at(s: Shape, x: f32, y: f32) -> Option<([u8; 3], f32)> {
    let (cx, cy) = (x - 0.5, y - 0.5);
    let r = (cx * cx + cy * cy).sqrt();
    match s {
        Shape::Raid(n) => {
            let col: [u8; 3] = match n {
                1 => [255, 230, 60],
                2 => [255, 150, 40],
                3 => [200, 90, 255],
                4 => [60, 220, 60],
                5 => [210, 220, 240],
                6 => [60, 140, 255],
                7 => [240, 50, 40],
                _ => [245, 245, 245],
            };
            let inside = match n {
                1 => {
                    let pts: Vec<(f32, f32)> = (0..10)
                        .map(|k| {
                            let a = std::f32::consts::PI * (k as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
                            let rr = if k % 2 == 0 { 0.48 } else { 0.2 };
                            (0.5 + rr * a.cos(), 0.53 + rr * a.sin())
                        })
                        .collect();
                    in_poly(x, y, &pts)
                }
                2 => r <= 0.42,
                3 => cx.abs() / 0.36 + cy.abs() / 0.47 <= 1.0,
                4 => in_poly(x, y, &[(0.5, 0.06), (0.95, 0.9), (0.05, 0.9)]),
                5 => r <= 0.43 && ((cx - 0.17).powi(2) + (cy + 0.1).powi(2)).sqrt() > 0.36,
                6 => cx.abs() <= 0.36 && cy.abs() <= 0.36,
                7 => r <= 0.5 && ((cx - cy).abs() <= 0.15 || (cx + cy).abs() <= 0.15),
                _ => {
                    let head = ((cx).powi(2) + (cy + 0.06).powi(2)).sqrt() <= 0.4;
                    let jaw = cx.abs() <= 0.2 && (0.2..=0.45).contains(&cy);
                    let eye = ((cx.abs() - 0.15).powi(2) + (cy + 0.02).powi(2)).sqrt() <= 0.1;
                    if eye && head {
                        return Some(([20, 20, 20], 1.0));
                    }
                    head || jaw
                }
            };
            inside.then_some((col, 1.0))
        }
        Shape::Quality(col) => {
            let d = cx.abs() / 0.34 + cy.abs() / 0.44;
            if d > 1.0 {
                return None;
            }
            // Facette claire en haut à gauche.
            let light = cx < 0.0 && cy < 0.0;
            let c = if light { [col[0].saturating_add(40), col[1].saturating_add(40), col[2].saturating_add(40)] } else { col };
            Some((c, 1.0))
        }
        Shape::Coin(col) => (r <= 0.36).then_some((col, 1.0)),
        Shape::Unknown => (cx.abs() <= 0.4 && cy.abs() <= 0.4).then_some(([150, 150, 150], 0.35)),
    }
}

/// Forme rendue à la taille `w` x `h` (4 x 4 points par pixel).
pub fn shape_image(s: Shape, w: usize, h: usize) -> Rgba {
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let u = (x as f32 + (sx as f32 + 0.5) / 4.0) / w as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / 4.0) / h as f32;
                    if let Some((c, a)) = shape_at(s, u, v) {
                        acc[0] += c[0] as f32 * a;
                        acc[1] += c[1] as f32 * a;
                        acc[2] += c[2] as f32 * a;
                        acc[3] += a;
                    }
                }
            }
            let a = acc[3] / 16.0;
            let c = |v: f32| if acc[3] > 0.0 { (v / acc[3]).round().clamp(0.0, 255.0) as u8 } else { 0 };
            px.push([c(acc[0]), c(acc[1]), c(acc[2]), (a * 255.0).round() as u8]);
        }
    }
    Rgba { w, h, px }
}

/// Image à dessiner pour une icône : la vraie si elle est prête, sinon sa forme de remplacement.
pub fn image_or_shape(id: u32, w: usize, h: usize) -> Arc<Rgba> {
    if let Some(i) = image(id, w, h) {
        return i;
    }
    let key = (u32::MAX - shape_key(shape_of(id)), w as u16, h as u16);
    if let Some(i) = lock().scaled.get(&key) {
        return i.clone();
    }
    let img = Arc::new(shape_image(shape_of(id), w, h));
    lock().scaled.insert(key, img.clone());
    img
}

fn shape_key(s: Shape) -> u32 {
    match s {
        Shape::Raid(n) => n as u32,
        Shape::Quality(c) => 100 + c[0] as u32 + 7 * c[1] as u32 + 13 * c[2] as u32,
        Shape::Coin(c) => 20000 + c[0] as u32 + 7 * c[1] as u32,
        Shape::Unknown => 50000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blp(comp: u8, alpha_depth: u8, alpha_type: u8, w: u32, h: u32, data: &[u8], palette: &[[u8; 4]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"BLP2");
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&[comp, alpha_depth, alpha_type, 0]);
        d.extend_from_slice(&w.to_le_bytes());
        d.extend_from_slice(&h.to_le_bytes());
        let off = 148 + 1024;
        let mut offs = [0u32; 16];
        let mut sizes = [0u32; 16];
        offs[0] = off as u32;
        sizes[0] = data.len() as u32;
        for o in offs {
            d.extend_from_slice(&o.to_le_bytes());
        }
        for s in sizes {
            d.extend_from_slice(&s.to_le_bytes());
        }
        for i in 0..256 {
            let p = palette.get(i).copied().unwrap_or([0, 0, 0, 0]);
            d.extend_from_slice(&p);
        }
        d.extend_from_slice(data);
        d
    }

    #[test]
    fn blp_images_of_every_kind_decode() {
        // Palette (BGRA) avec alpha 1 bit : rouge opaque, puis bleu transparent.
        let d = blp(1, 1, 0, 2, 1, &[0, 1, 0b01], &[[0, 0, 255, 0], [255, 0, 0, 0]]);
        let i = decode_blp(&d).unwrap();
        assert_eq!(i.px, vec![[255, 0, 0, 255], [0, 0, 255, 0]]);
        // DXT1 : bloc uni blanc (c0 = c1 = 0xFFFF, indices 0).
        let d = blp(2, 0, 0, 4, 4, &[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0], &[]);
        let i = decode_blp(&d).unwrap();
        assert!(i.px.iter().all(|p| *p == [255, 255, 255, 255]));
        // DXT1, c0 <= c1 : indice 3 transparent.
        let d = blp(2, 1, 0, 4, 4, &[0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], &[]);
        let i = decode_blp(&d).unwrap();
        assert!(i.px.iter().all(|p| p[3] == 0));
        // DXT5 : alpha 0 et 255 interpolés, couleur rouge pur (0xF800).
        let mut b = vec![255u8, 0, 0, 0, 0, 0, 0, 0];
        b.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        let d = blp(2, 8, 7, 4, 4, &b, &[]);
        let i = decode_blp(&d).unwrap();
        assert!(i.px.iter().all(|p| *p == [255, 0, 0, 255]));
        // BGRA direct.
        let d = blp(3, 8, 0, 1, 1, &[10, 20, 30, 40], &[]);
        assert_eq!(decode_blp(&d).unwrap().px, vec![[30, 20, 10, 40]]);
        assert!(decode_blp(b"BLP1....").is_err());
    }

    #[test]
    fn markup_gives_the_size_and_the_source() {
        let r = parse_texture("Interface\\TargetingFrame\\UI-RaidTargetingIcon_8:0");
        assert_eq!((r.h, r.w), (0.0, 0.0));
        assert_eq!(shape_of(r.id), Shape::Raid(8));
        let r2 = parse_texture("Interface\\TargetingFrame\\UI-RaidTargetingIcon_8:0");
        assert_eq!(r.id, r2.id);
        let c = parse_texture("Interface\\ChatFrame\\UI-ChatIcon-Blizz:12:20:0:0:32:16:4:28:0:16");
        assert_eq!((c.h, c.w), (12.0, 20.0));
        assert_eq!(spec(c.id).unwrap().crop, Some([4.0 / 32.0, 28.0 / 32.0, 0.0, 1.0]));
        let f = parse_texture("134400:16");
        assert_eq!(spec(f.id).unwrap().src, Source::Fdid(134400));
        assert_eq!((f.h, f.w), (16.0, 16.0));
        let a = parse_atlas(":Professions-ChatIcon-Quality-Tier3:17:15::1");
        assert_eq!((a.h, a.w, a.dy), (17.0, 15.0, 1.0));
        assert!(matches!(shape_of(a.id), Shape::Quality(_)));
        // Taille : unités de l'interface fois l'échelle ; hauteur 0 : la police.
        assert_eq!(size_px(&a, 14.0, 2.0), (30.0, 34.0));
        assert_eq!(size_px(&r, 14.0, 2.0), (14.0, 14.0));
        // Largeur 0 d'un atlas : proportions données par l'addon.
        let b = parse_atlas(":Wide-Thing-Test:10:0");
        learn_atlas("Wide-Thing-Test", Atlas { fdid: 0, crop: [0.0, 1.0, 0.0, 1.0], w: 40, h: 20 });
        assert_eq!(size_px(&b, 14.0, 1.0), (20.0, 10.0));
    }

    #[test]
    fn icon_frames_from_the_addon_describe_atlases() {
        let name = b"Professions-ChatIcon-Quality-Tier2";
        let mut p = vec![0u8, name.len() as u8];
        p.extend_from_slice(name);
        p.extend_from_slice(&4_559_321u32.to_be_bytes());
        for v in [0u16, 32767, 0, 65535, 34, 30] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        let s = on_frame(&p).unwrap();
        assert!(s.contains("4559321"), "{s}");
        let g = lock();
        let a = g.atlases.get("professions-chaticon-quality-tier2").unwrap();
        assert_eq!((a.fdid, a.w, a.h), (4_559_321, 34, 30));
        assert!((a.crop[1] - 0.5).abs() < 0.001);
        drop(g);
        assert!(on_frame(&[0, 9, 1]).is_err());
    }

    #[test]
    fn scaling_averages_and_crops() {
        // 2x1 : moitié gauche rouge opaque, droite transparente ; ramenée à 1x1 : rouge à demi.
        let src = Rgba { w: 2, h: 1, px: vec![[255, 0, 0, 255], [0, 255, 0, 0]] };
        let s = scale(&src, [0.0, 1.0, 0.0, 1.0], 1, 1);
        assert_eq!(s.px[0][0], 255);
        assert_eq!(s.px[0][1], 0);
        assert!((s.px[0][3] as i32 - 128).abs() <= 2);
        // Partie gauche seule : rouge plein.
        let s = scale(&src, [0.0, 0.5, 0.0, 1.0], 2, 2);
        assert!(s.px.iter().all(|p| p[0] == 255 && p[3] > 200), "{:?}", s.px);
    }

    #[test]
    fn shapes_stand_in_for_missing_images() {
        let star = shape_image(Shape::Raid(1), 16, 16);
        let filled = star.px.iter().filter(|p| p[3] > 128).count();
        assert!(filled > 40 && filled < 200, "{filled}");
        // Centre plein, coins vides.
        assert!(star.px[8 * 16 + 8][3] > 200);
        assert_eq!(star.px[0][3], 0);
        let skull = shape_image(Shape::Raid(8), 16, 16);
        assert!(skull.px.iter().any(|p| p[3] > 200 && p[0] < 50), "yeux noirs");
        assert_eq!(archive_path("Interface\\Icons\\INV_Misc_QuestionMark"), "Interface\\Icons\\INV_Misc_QuestionMark.blp");
        assert_eq!(archive_path("Interface\\Foo.tga"), "Interface\\Foo.tga");
    }

    #[test]
    fn images_are_read_from_the_game_archives_and_cached() {
        use crate::casc::fixture;
        let dir = std::env::temp_dir().join(format!("ct-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut entries = fixture::default_entries();
        let img = blp(3, 8, 0, 1, 1, &[10, 20, 30, 255], &[]);
        entries.push(fixture::Entry {
            path: Some("Interface\\TargetingFrame\\UI-RaidTargetingIcon_3.blp".into()),
            fdid: 9_000_003,
            locale: 0xFFFF_FFFF,
            content: img,
        });
        fixture::build(&dir, &entries).unwrap();
        let mut st = Storage::open(&dir, &["wowf"]).unwrap();
        let hits = st.find_batch(&[archive_path("Interface\\TargetingFrame\\UI-RaidTargetingIcon_3")], &[9_000_003]).unwrap();
        assert_eq!(hits.iter().filter(|(i, _)| *i == 0).count(), 1, "{hits:?}");
        assert!(hits.iter().all(|(_, h)| h.fdid == 9_000_003));
        let by_id = st.find_batch(&[], &[9_000_003]).unwrap();
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].0, 0);
        let h = casc::pick(&hits.iter().map(|(_, h)| *h).collect::<Vec<_>>(), 0x2).unwrap();
        let data = st.read_ckey(&h.ckey).unwrap();
        assert_eq!(decode_blp(&data).unwrap().px, vec![[30, 20, 10, 255]]);
        let f = dir.join("c.rgba");
        write_cached(&f, &decode_blp(&data).unwrap());
        assert_eq!(read_cached(&f).unwrap().px, vec![[30, 20, 10, 255]]);
        std::fs::remove_dir_all(dir).ok();
    }
}
