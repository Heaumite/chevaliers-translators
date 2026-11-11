//! Rendu synthétique du bandeau, pour l'auto-test sans le jeu.

/// Générateur pseudo-aléatoire minimal (xorshift64*), sans dépendance.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Distortion {
    pub gamma: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub noise: i32,
    pub blur: bool,
    pub offset_x: usize,
    pub offset_y: usize,
}

impl Distortion {
    pub const NONE: Distortion = Distortion {
        gamma: 1.0,
        contrast: 1.0,
        brightness: 0.0,
        noise: 0,
        blur: false,
        offset_x: 0,
        offset_y: 0,
    };
}

/// Image BGRA w*h avec un décor aléatoire et le bandeau collé au coin bas-droit.
pub fn render(
    syms: &[u8],
    cols: usize,
    block: usize,
    w: usize,
    h: usize,
    d: &Distortion,
    rng: &mut Rng,
) -> Vec<u8> {
    let mut img = vec![0u8; w * h * 4];
    // Décor : grands aplats colorés façon interface de jeu.
    let tile = 7;
    for ty in (0..h).step_by(tile) {
        for tx in (0..w).step_by(tile) {
            let c = [
                rng.below(256) as u8,
                rng.below(256) as u8,
                rng.below(256) as u8,
            ];
            for y in ty..(ty + tile).min(h) {
                for x in tx..(tx + tile).min(w) {
                    let i = (y * w + x) * 4;
                    img[i..i + 3].copy_from_slice(&c);
                    img[i + 3] = 255;
                }
            }
        }
    }
    let level = |l: u8| ((l as u32 * 255 + 1) / 3) as u8;
    let right = w - 1 - d.offset_x;
    let bottom = h - 1 - d.offset_y;
    for (i, &s) in syms.iter().enumerate() {
        let (row, k) = if i < cols { (0, i) } else { (1, i - cols) };
        let (r, g, b) = (level(s >> 4), level((s >> 2) & 3), level(s & 3));
        for y in (bottom + 1 - (row + 1) * block)..=(bottom - row * block) {
            for x in (right + 1 - (k + 1) * block)..=(right - k * block) {
                let p = (y * w + x) * 4;
                img[p] = b;
                img[p + 1] = g;
                img[p + 2] = r;
            }
        }
    }
    if d.blur {
        let src = img.clone();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                for ch in 0..3 {
                    let mut sum = 0u32;
                    for yy in y - 1..=y + 1 {
                        for xx in x - 1..=x + 1 {
                            sum += src[(yy * w + xx) * 4 + ch] as u32;
                        }
                    }
                    img[(y * w + x) * 4 + ch] = (sum / 9) as u8;
                }
            }
        }
    }
    for p in img.chunks_exact_mut(4) {
        for v in p.iter_mut().take(3) {
            let x = (*v as f32 / 255.0).powf(d.gamma);
            let mut y = 255.0 * (d.contrast * (x - 0.5) + 0.5 + d.brightness);
            if d.noise > 0 {
                y += (rng.below(2 * d.noise as u64 + 1) as i32 - d.noise) as f32;
            }
            *v = y.round().clamp(0.0, 255.0) as u8;
        }
    }
    img
}
