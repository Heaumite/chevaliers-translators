//! Auto-test hors jeu : encode, rend avec distorsions, décode, compare.
//! Deux séries : conditions réalistes (doivent toutes passer) et stress (réglages d'image
//! extrêmes, rapportés pour information). Une trame altérée ne doit jamais être acceptée.

use std::collections::BTreeMap;

use crate::decode::{decode, Image};
use crate::proto::{self, Frame};
use crate::render::{render, Distortion, Rng};

struct Series {
    gammas: [f32; 3],
    contrasts: [f32; 3],
    brights: [f32; 3],
    max_noise: u64,
}

const REALISTIC: Series = Series {
    gammas: [0.8, 1.0, 1.25],
    contrasts: [0.85, 1.0, 1.15],
    brights: [-0.05, 0.0, 0.05],
    max_noise: 4,
};

const STRESS: Series = Series {
    gammas: [0.6, 1.0, 1.8],
    contrasts: [0.7, 1.0, 1.2],
    brights: [-0.08, 0.0, 0.08],
    max_noise: 6,
};

struct Outcome {
    ok: usize,
    wrong: usize,
    failed: usize,
    reasons: BTreeMap<&'static str, usize>,
    worst: BTreeMap<String, usize>,
}

fn series(s: &Series, frames: usize, rng: &mut Rng) -> Outcome {
    let screens = [1920usize, 2560, 3840];
    let blocks = [4usize, 6, 8];
    let mut o = Outcome {
        ok: 0,
        wrong: 0,
        failed: 0,
        reasons: BTreeMap::new(),
        worst: BTreeMap::new(),
    };
    for n in 0..frames {
        let sw = screens[n % screens.len()];
        let block = blocks[(n / 3) % blocks.len()];
        let cols = (sw as f32 * 0.25 / block as f32) as usize;
        let d = Distortion {
            gamma: s.gammas[rng.below(3) as usize],
            contrast: s.contrasts[rng.below(3) as usize],
            brightness: s.brights[rng.below(3) as usize],
            noise: rng.below(s.max_noise + 1) as i32,
            blur: rng.below(3) == 0,
            offset_x: rng.below(3) as usize,
            offset_y: rng.below(3) as usize,
        };
        let maxp = proto::capacity(cols).2;
        let seq = rng.below(65536) as u16;
        let want = if n % 2 == 0 {
            Frame {
                ftype: proto::FRAME_TEST,
                seq,
                payload: proto::test_payload(seq, maxp),
            }
        } else {
            let text = "Hans:[DE] Guten Abend zusammen - Ça marche ? Привет ✓";
            let frag = proto::chat_fragments(seq, 10, "Hans-Ysondre", text, maxp).remove(0);
            Frame {
                ftype: proto::FRAME_CHAT,
                seq,
                payload: frag,
            }
        };
        let syms = proto::build_banner(want.ftype, want.seq, &want.payload, cols).unwrap();
        // Zone capturée un peu plus grande que le bandeau, comme en direct.
        let (w, h) = (cols * block + 24, 2 * block + 16);
        let img = render(&syms, cols, block, w, h, &d, rng);
        match decode(&Image {
            w,
            h,
            stride: w * 4,
            bgra: &img,
        }) {
            Ok(got) if got.frame == want => o.ok += 1,
            Ok(_) => o.wrong += 1,
            Err(e) => {
                o.failed += 1;
                *o.reasons.entry(e.label()).or_default() += 1;
                let key = format!(
                    "gamma {} contraste {} luminosité {:+}",
                    d.gamma, d.contrast, d.brightness
                );
                *o.worst.entry(key).or_default() += 1;
            }
        }
    }
    o
}

fn corruption(frames: usize, rng: &mut Rng) -> (usize, usize, usize) {
    let (mut rejected, mut harmless, mut accepted_wrong) = (0, 0, 0);
    let (block, cols) = (4usize, 120usize);
    let maxp = proto::capacity(cols).2;
    for n in 0..frames {
        let seq = (n % 65536) as u16;
        let want = Frame {
            ftype: proto::FRAME_TEST,
            seq,
            payload: proto::test_payload(seq, maxp),
        };
        let mut syms = proto::build_banner(want.ftype, seq, &want.payload, cols).unwrap();
        for _ in 0..=rng.below(3) {
            let i =
                proto::FIXED_BLOCKS + rng.below((2 * cols - proto::FIXED_BLOCKS) as u64) as usize;
            syms[i] ^= 1 + rng.below(63) as u8;
        }
        let (w, h) = (cols * block + 8, 2 * block + 8);
        let img = render(&syms, cols, block, w, h, &Distortion::NONE, rng);
        match decode(&Image {
            w,
            h,
            stride: w * 4,
            bgra: &img,
        }) {
            Ok(got) if got.frame == want => harmless += 1, // altération dans le remplissage
            Ok(_) => accepted_wrong += 1,
            Err(_) => rejected += 1,
        }
    }
    (rejected, harmless, accepted_wrong)
}

pub fn run(frames: usize) -> bool {
    let mut rng = Rng::new(0x5EED_C0DE);
    let real = series(&REALISTIC, frames, &mut rng);
    let stress = series(&STRESS, frames, &mut rng);
    let (rejected, harmless, accepted_wrong) = corruption(frames, &mut rng);

    crate::say!(
        "Auto-test CT-Pixel v1 ({frames} trames par série, blocs 4/6/8 px, écrans 1080p/1440p/4K)"
    );
    crate::say!("  Conditions réalistes (gamma 0,8-1,25, contraste ±15 %, luminosité ±5 %, bruit, flou, décalage 0-2 px)");
    crate::say!("    décodées à l'identique : {}", real.ok);
    crate::say!(
        "    contenu faux : {}   non décodées : {} {:?}",
        real.wrong, real.failed, real.reasons
    );
    crate::say!("  Stress (gamma 0,6-1,8, contraste ±30 %, luminosité ±8 %) - pour information");
    crate::say!("    décodées à l'identique : {}", stress.ok);
    crate::say!(
        "    contenu faux : {}   non décodées : {} {:?}",
        stress.wrong, stress.failed, stress.reasons
    );
    let mut worst: Vec<_> = stress.worst.into_iter().collect();
    worst.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in worst.iter().take(3) {
        crate::say!("      échecs sous {k} : {n}");
    }
    crate::say!("  Trames volontairement altérées : {frames}");
    crate::say!("    rejetées : {rejected}   sans effet (remplissage) : {harmless}");
    crate::say!("    ACCEPTÉES AVEC UN CONTENU FAUX : {accepted_wrong}");

    let pass = real.failed == 0 && real.wrong == 0 && stress.wrong == 0 && accepted_wrong == 0;
    crate::say!(
        "{}",
        if pass {
            "RÉSULTAT : OK"
        } else {
            "RÉSULTAT : ÉCHEC"
        }
    );
    pass
}
