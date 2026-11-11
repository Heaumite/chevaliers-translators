//! Lecture du bandeau dans une image capturée (coin bas-droit de l'image).
//! Auto-étalonnage : les 4 blocs gris de chaque trame fixent les seuils par canal,
//! ce qui rend la lecture insensible au gamma, au contraste et à la luminosité.

use crate::proto::{self, DecodeError, Frame, FIXED_BLOCKS};

/// Image BGRA 8 bits, lignes de haut en bas.
pub struct Image<'a> {
    pub w: usize,
    pub h: usize,
    pub stride: usize,
    pub bgra: &'a [u8],
}

impl Image<'_> {
    #[inline]
    fn rgb(&self, x: usize, y: usize) -> [i32; 3] {
        let i = y * self.stride + x * 4;
        [
            self.bgra[i + 2] as i32,
            self.bgra[i + 1] as i32,
            self.bgra[i] as i32,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub block: usize,
    pub cols: usize,
    /// Coin bas-droit du bandeau (pixel inclus).
    pub right: usize,
    pub bottom: usize,
}

#[derive(Debug, Clone)]
pub struct Decoded {
    pub frame: Frame,
    pub geometry: Geometry,
}

/// Positif sur le magenta, négatif sur le vert, proche de 0 sur un pixel de transition.
#[inline]
fn hue(c: [i32; 3]) -> i32 {
    (c[0] + c[2]) / 2 - c[1]
}

const STRONG: i32 = 60;

/// Cherche le motif magenta/vert/magenta/vert près du coin bas-droit.
/// Renvoie (taille de bloc, bord droit estimé, une ligne de la rangée du bas).
/// La taille de bloc vient des frontières entre couleurs, interpolées au sous-pixel :
/// elle résiste au flou et à un décor voisin de couleur proche.
fn find_sync(img: &Image, max: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    if img.w < 16 || img.h < 4 {
        return out;
    }
    for dy in 0..img.h.min(20) {
        let y = img.h - 1 - dy;
        for dx in 0..8.min(img.w) {
            let x0 = img.w - 1 - dx;
            if hue(img.rgb(x0, y)) <= STRONG {
                continue;
            }
            // Trois frontières vers la gauche : M|V, V|M, M|V.
            let mut crossings = [0f32; 3];
            let mut found = 0;
            let mut sign = 1;
            let mut strong_seen = true;
            let mut prev = hue(img.rgb(x0, y));
            let limit = x0.saturating_sub(4 * 17);
            let mut x = x0;
            while x > limit && found < 3 {
                x -= 1;
                let v = hue(img.rgb(x, y));
                if v * sign < 0 && v.abs() > 0 {
                    if !strong_seen {
                        break;
                    }
                    // Interpolation linéaire du passage par zéro entre x et x+1.
                    let t = prev as f32 / (prev - v) as f32;
                    crossings[found] = (x + 1) as f32 - t;
                    found += 1;
                    sign = -sign;
                    strong_seen = false;
                }
                if v * sign > STRONG {
                    strong_seen = true;
                }
                prev = v;
            }
            // Les segments entre frontières ont tous été vus bien colorés.
            if found < 3 {
                continue;
            }
            let b1 = crossings[0] - crossings[1];
            let b2 = crossings[1] - crossings[2];
            let bf = (b1 + b2) / 2.0;
            let block = bf.round() as usize;
            if !(2..=16).contains(&block) || (b1 - bf).abs() > 1.0 || (b2 - bf).abs() > 1.0 {
                continue;
            }
            let right = (crossings[0] + bf - 0.5).round() as usize;
            if right >= img.w {
                continue;
            }
            if !out.contains(&(block, right, y)) {
                out.push((block, right, y));
                if out.len() >= max {
                    return out;
                }
            }
            // Une ligne suffit : on passe à la suivante pour d'autres candidats.
            break;
        }
    }
    out
}

/// Couleur moyenne des 2x2 pixels centraux du bloc `k` (compté depuis la droite) de la rangée `row`.
fn sample(img: &Image, g: &Geometry, row: usize, k: usize) -> Option<[i32; 3]> {
    let b = g.block;
    let left = g.right.checked_sub((k + 1) * b - 1)?;
    let top = g.bottom.checked_sub((row + 1) * b - 1)?;
    let (o1, o2) = ((b - 1) / 2, b / 2);
    let mut sum = [0i32; 3];
    for (ox, oy) in [(o1, o1), (o2, o1), (o1, o2), (o2, o2)] {
        let c = img.rgb(left + ox, top + oy);
        for ch in 0..3 {
            sum[ch] += c[ch];
        }
    }
    Some([sum[0] / 4, sum[1] / 4, sum[2] / 4])
}

struct Calibration {
    /// Seuils entre niveaux 0|1, 1|2, 2|3 pour chaque canal.
    thresholds: [[i32; 3]; 3],
}

impl Calibration {
    fn from_samples(levels: [[i32; 3]; 4]) -> Option<Self> {
        let mut thresholds = [[0; 3]; 3];
        for ch in 0..3 {
            for l in 0..3 {
                let (a, b) = (levels[l][ch], levels[l + 1][ch]);
                if b - a < 10 {
                    return None;
                }
                thresholds[l][ch] = (a + b) / 2;
            }
        }
        Some(Self { thresholds })
    }

    fn symbol(&self, c: [i32; 3]) -> u8 {
        let mut s = 0u8;
        for ch in 0..3 {
            let level = self.thresholds.iter().filter(|t| c[ch] > t[ch]).count() as u8;
            s = (s << 2) | level;
        }
        s
    }
}

/// Décode le bandeau présent dans l'image, ou explique pourquoi c'est impossible.
/// La position estimée est corrigée d'un pixel si besoin : seul le CRC fait foi.
pub fn decode(img: &Image) -> Result<Decoded, DecodeError> {
    let mut best = DecodeError::NoSync;
    // Plusieurs lignes candidates : la première peut être brouillée par le décor voisin.
    for (block, right, row_y) in find_sync(img, 6) {
        // La ligne trouvée appartient à la rangée du bas : le bas du bandeau est entre
        // cette ligne et block-1 pixels plus bas.
        for db in 0..block {
            let bottom = row_y + db;
            if bottom >= img.h {
                break;
            }
            for dx in [0isize, -1, 1] {
                let Some(r) = right.checked_add_signed(dx) else {
                    continue;
                };
                if r >= img.w {
                    continue;
                }
                match decode_at(img, block, r, bottom) {
                    Ok(d) => return Ok(d),
                    Err(e) => {
                        if rank(e) > rank(best) {
                            best = e;
                        }
                    }
                }
            }
        }
    }
    Err(best)
}

/// Gravité d'un échec, pour rapporter le plus parlant.
fn rank(e: DecodeError) -> u8 {
    match e {
        DecodeError::NoSync => 0,
        DecodeError::BadGeometry => 1,
        DecodeError::BadCalibration => 2,
        DecodeError::BadVersion | DecodeError::BadLength => 3,
        DecodeError::Crc => 4,
    }
}

fn decode_at(
    img: &Image,
    block: usize,
    right: usize,
    bottom: usize,
) -> Result<Decoded, DecodeError> {
    let mut g = Geometry {
        block,
        cols: 0,
        right,
        bottom,
    };

    let mut cal = [[0i32; 3]; 4];
    for (l, slot) in cal.iter_mut().enumerate() {
        *slot = sample(img, &g, 0, 4 + l).ok_or(DecodeError::BadGeometry)?;
    }
    let cal = Calibration::from_samples(cal).ok_or(DecodeError::BadCalibration)?;

    for (k, &want) in proto::SYNC.iter().enumerate() {
        let got = cal.symbol(sample(img, &g, 0, k).ok_or(DecodeError::BadGeometry)?);
        if got != want {
            return Err(DecodeError::NoSync);
        }
    }

    let hi = cal.symbol(sample(img, &g, 0, 8).ok_or(DecodeError::BadGeometry)?) as usize;
    let lo = cal.symbol(sample(img, &g, 0, 9).ok_or(DecodeError::BadGeometry)?) as usize;
    g.cols = hi * 64 + lo;
    if g.cols < FIXED_BLOCKS || g.cols * block > right + 1 || 2 * block > bottom + 1 {
        return Err(DecodeError::BadGeometry);
    }

    let (nsym, nbytes, _) = proto::capacity(g.cols);
    let mut syms = Vec::with_capacity(nsym);
    for i in FIXED_BLOCKS..2 * g.cols {
        let (row, k) = if i < g.cols { (0, i) } else { (1, i - g.cols) };
        syms.push(cal.symbol(sample(img, &g, row, k).ok_or(DecodeError::BadGeometry)?));
    }
    let bytes = proto::symbols_to_bytes(&syms);
    let frame = proto::parse_frame(&bytes[..nbytes.min(bytes.len())])?;
    Ok(Decoded { frame, geometry: g })
}
