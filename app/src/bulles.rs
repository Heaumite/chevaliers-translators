//! Bulles de dialogue traduites (0.15), sous-module du chat miroir (polices et mise en page).
//!
//! L'addon envoie, pour chaque bulle visible au-dessus d'un joueur, le numéro du message relayé
//! qu'elle affiche, la place de son texte à l'écran, sa couleur et sa police. Si ce message a une
//! traduction, une petite fenêtre est posée sur la bulle : plaque sombre, texte traduit centré
//! comme dans le jeu, précédé de l'acronyme de la langue d'origine (« [DE] ... »).
//!
//! 0.16.3 : la bulle de Blizzard est effacée par l'addon (plus de double bulle) ; la nôtre est
//! dessinée en entier, avec sa pointe, traduite ou non (texte d'origine tant que rien n'est à
//! traduire ou que la traduction n'est pas arrivée).
//!
//! Les bulles suivent le personnage qui parle : entre deux relevés de l'addon (10 par seconde au
//! plus, et toute la chaîne capture-affichage), la place est prolongée selon la vitesse mesurée,
//! pour que la traduction colle à la bulle quand la caméra tourne.

use std::time::{Duration, Instant};

use super::{advances, parse_markup, wrap, Canvas, FontBook, Item, Layout, Rgb, Update, PLATE};

/// Première fenêtre des bulles (les fenêtres de chat sont numérotées de 1 à ~10, les panneaux
/// à partir de 248).
pub const FIRST_FRAME: u8 = 200;
/// Bord de la bulle (et de sa pointe).
const BUBBLE_EDGE: Rgb = [150, 140, 118];
pub const MAX_BUBBLES: usize = 8;
/// Bulle sans nouvelles de l'addon depuis ce temps : retirée (il renvoie toutes les 2 s).
const STALE: Duration = Duration::from_millis(3500);
/// Avance donnée à la place prédite (retard de la capture et de l'affichage).
const LEAD: f32 = 0.06;
/// Prédiction limitée à ce temps après le dernier relevé.
const HORIZON: f32 = 0.2;

/// Bulle vue par l'addon (pixels physiques depuis le bas à gauche de la zone de jeu).
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub id: u16,
    pub left: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
    pub color: Rgb,
    pub px: f32,
}

/// Trame des bulles : [nombre] puis, par bulle, numéro, gauche, bas, largeur, hauteur (u16),
/// couleur (3 octets), taille de la police en dixièmes de pixel (u16) ; enfin la police
/// (longueur, chemin).
pub fn parse(p: &[u8]) -> Result<(Vec<Seen>, String), String> {
    let n = *p.first().ok_or("trame vide")? as usize;
    let need = 1 + 15 * n;
    if p.len() < need + 1 {
        return Err(format!("trame de {} octets pour {n} bulle(s)", p.len()));
    }
    let u = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]);
    let list = (0..n)
        .map(|j| {
            let o = 1 + 15 * j;
            Seen {
                id: u(o),
                left: u(o + 2) as i32,
                bottom: u(o + 4) as i32,
                width: u(o + 6) as i32,
                height: u(o + 8) as i32,
                color: [p[o + 10], p[o + 11], p[o + 12]],
                px: u(o + 13) as f32 / 10.0,
            }
        })
        .collect();
    let len = p[need] as usize;
    let path = p.get(need + 1..need + 1 + len).ok_or("chemin de police tronqué")?;
    Ok((list, String::from_utf8_lossy(path).into_owned()))
}

/// Textes des bulles placés après la police (addon 0.16.3), tant qu'il restait de la place :
/// [rang] [longueur] [texte]. Un texte par bulle au plus ; `None` : pas envoyé.
pub fn texts(p: &[u8], n: usize) -> Vec<Option<String>> {
    let mut out = vec![None; n];
    let need = 1 + 15 * n;
    let Some(&len) = p.get(need) else { return out };
    let mut i = need + 1 + len as usize;
    while i + 2 <= p.len() {
        let (k, l) = (p[i] as usize, p[i + 1] as usize);
        let Some(t) = p.get(i + 2..i + 2 + l) else { break };
        if k < n && out[k].is_none() {
            out[k] = Some(String::from_utf8_lossy(t).into_owned());
        }
        i += 2 + l;
    }
    out
}

/// Image d'une bulle traduite, et décalage de son coin bas-gauche par rapport à celui du texte
/// d'origine.
pub struct Drawn {
    pub text: String,
    pub canvas: Canvas,
    pub dx: i32,
    pub dy: i32,
}

struct State {
    seen: Seen,
    body: String,
    at: Instant,
    /// Vitesse mesurée (pixels par seconde).
    vel: (f32, f32),
    slot: u8,
    drawn: Option<Drawn>,
    /// Dernière place posée (gauche, bas, texte) : rien n'est renvoyé si elle ne change pas.
    posted: Option<(i32, i32, String)>,
}

#[derive(Default)]
pub struct Bubbles {
    states: Vec<State>,
    pub font: String,
    /// Fenêtres à cacher au prochain passage.
    to_hide: Vec<u8>,
    pub seen_total: usize,
}

impl Bubbles {
    /// Relevé de l'addon : `list` avec le texte d'origine de chaque bulle (`None` : message
    /// inconnu de l'application, bulle ignorée).
    pub fn update(&mut self, list: Vec<(Seen, Option<String>)>, font: String, now: Instant) {
        if !font.is_empty() {
            self.font = font;
        }
        let mut next: Vec<State> = Vec::new();
        for (s, body) in list.into_iter().take(MAX_BUBBLES) {
            let Some(body) = body else { continue };
            self.seen_total += 1;
            match self.states.iter().position(|o| o.seen.id == s.id) {
                Some(i) => {
                    let mut o = self.states.remove(i);
                    let dt = now.saturating_duration_since(o.at).as_secs_f32();
                    o.vel = if dt > 0.02 && dt < 0.5 {
                        ((s.left - o.seen.left) as f32 / dt, (s.bottom - o.seen.bottom) as f32 / dt)
                    } else {
                        (0.0, 0.0)
                    };
                    // Taille du texte d'origine changée (bulle recomposée) : image refaite.
                    if (o.seen.width, o.seen.height, o.seen.color, (o.seen.px * 10.0) as i32) != (s.width, s.height, s.color, (s.px * 10.0) as i32) {
                        o.drawn = None;
                    }
                    o.seen = s;
                    o.body = body;
                    o.at = now;
                    next.push(o);
                }
                None => next.push(State { seen: s, body, at: now, vel: (0.0, 0.0), slot: u8::MAX, drawn: None, posted: None }),
            }
        }
        // Bulles disparues : leurs fenêtres sont cachées.
        for o in self.states.drain(..) {
            if o.posted.is_some() {
                self.to_hide.push(FIRST_FRAME + o.slot);
            }
        }
        // Chaque bulle garde sa fenêtre ; les nouvelles prennent les premières libres.
        let used: Vec<u8> = next.iter().filter(|s| s.slot != u8::MAX).map(|s| s.slot).collect();
        let mut free = (0..MAX_BUBBLES as u8).filter(|k| !used.contains(k));
        for s in next.iter_mut().filter(|s| s.slot == u8::MAX) {
            s.slot = free.next().unwrap_or(0);
        }
        self.to_hide.retain(|f| !next.iter().any(|s| FIRST_FRAME + s.slot == *f && s.posted.is_some()));
        self.states = next;
    }

    /// Tout est à reposer (fenêtres cachées le temps d'un combat, WoW repassé devant).
    pub fn redraw_all(&mut self) {
        for s in &mut self.states {
            s.posted = None;
        }
    }

    pub fn active(&self) -> bool {
        !self.states.is_empty()
    }

    /// Fenêtres à poser, déplacer ou cacher. `translation` donne le texte traduit d'un message
    /// (« [DE] ... ») ; `occ` : rectangles où rien n'est dessiné (pixels depuis le bas à gauche).
    pub fn tick(
        &mut self,
        book: &mut FontBook,
        translation: &dyn Fn(&str) -> Option<String>,
        occ: &[(i32, i32, i32, i32)],
        now: Instant,
    ) -> Vec<Update> {
        let mut ups: Vec<Update> = self
            .to_hide
            .drain(..)
            .map(|frame| Update { frame, left: 0, bottom: 0, canvas: None, hits: Vec::new() })
            .collect();
        let font = self.font.clone();
        self.states.retain(|s| {
            let keep = now.saturating_duration_since(s.at) < STALE;
            if !keep && s.posted.is_some() {
                ups.push(Update { frame: FIRST_FRAME + s.slot, left: 0, bottom: 0, canvas: None, hits: Vec::new() });
            }
            keep
        });
        for s in &mut self.states {
            let frame = FIRST_FRAME + s.slot;
            // Pas (encore) de traduction, ou rien à traduire : la bulle porte le texte d'origine
            // (celle de Blizzard est effacée par l'addon).
            let text = translation(&s.body).unwrap_or_else(|| s.body.clone());
            if s.drawn.as_ref().is_none_or(|d| d.text != text) {
                s.drawn = Some(draw(book, &font, &s.seen, &text));
            }
            let d = s.drawn.as_ref().unwrap();
            let t = (now.saturating_duration_since(s.at).as_secs_f32() + LEAD).min(HORIZON);
            let left = s.seen.left + d.dx + (s.vel.0 * t).round() as i32;
            let bottom = s.seen.bottom + d.dy + (s.vel.1 * t).round() as i32;
            let key = (left, bottom, text.clone());
            if s.posted.as_ref() == Some(&key) {
                continue;
            }
            let mut cv = d.canvas.clone();
            clear_under(&mut cv, left, bottom, occ);
            s.posted = Some(key);
            ups.push(Update { frame, left, bottom, canvas: Some(cv), hits: Vec::new() });
        }
        ups
    }
}

/// Efface ce qui est sous une autre fenêtre (bulle d'aide du jeu, autre application).
fn clear_under(cv: &mut Canvas, left: i32, bottom: i32, occ: &[(i32, i32, i32, i32)]) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    for &(l, b, ow, oh) in occ {
        let x0 = (l - left).clamp(0, w);
        let x1 = (l + ow - left).clamp(0, w);
        let y0 = (bottom + h - (b + oh)).clamp(0, h);
        let y1 = (bottom + h - b).clamp(0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                cv.px[(y * w + x) as usize] = 0;
            }
        }
    }
}

/// Plaque de la bulle : au moins la taille du texte d'origine, centrée sur lui, s'agrandissant vers
/// le haut si la traduction est plus longue ; texte centré ligne par ligne, comme les bulles du
/// jeu ; pointe en bas à gauche vers celui qui parle (0.16.3, la bulle de Blizzard étant effacée).
pub fn draw(book: &mut FontBook, font: &str, s: &Seen, text: &str) -> Drawn {
    let px = s.px.clamp(8.0, 64.0);
    let base = book.base(font);
    let (ascent, natural) = book.line_metrics(base, px);
    let line_h = natural.max(px);
    let layout = Layout::new(book, base, px, line_h, None);
    let items = parse_markup(text, s.color);
    let advs = advances(book, base, px, &items, None, &layout);
    // Largeur de coupe : celle du texte d'origine, au moins 8 caractères.
    let wrap_w = (s.width as f32).max(px * 8.0);
    let rows = wrap(&items, &advs, wrap_w, 0.0);
    let mut row_w = |row: &[(f32, Item)]| -> f32 {
        let its: Vec<Item> = row.iter().map(|(_, it)| *it).collect();
        let a = advances(book, base, px, &its, None, &layout);
        row.first().map_or(0.0, |(x0, _)| row.last().map_or(0.0, |(x, _)| x - x0) + a.last().copied().unwrap_or(0.0))
    };
    let widths: Vec<f32> = rows.iter().map(|r| row_w(r)).collect();
    let content_w = widths.iter().copied().fold(0.0f32, f32::max);
    let pad = (px * 0.45).round().max(3.0);
    let w = (content_w.max(s.width as f32) + 2.0 * pad).ceil() as usize;
    let text_h = rows.len().max(1) as f32 * line_h;
    let plate_h = (text_h.max(s.height as f32) + 2.0 * pad).ceil() as usize;
    // Pointe sous la plaque, à gauche.
    let tail_h = (px * 0.6).round().max(5.0) as usize;
    let h = plate_h + tail_h;
    let mut cv = Canvas::new(w, h);
    // Plaque aux coins arrondis, bord clair discret.
    let r = (px * 0.5).max(4.0);
    let ph = plate_h;
    for y in 0..ph {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, w as f32 - r);
            let cy = fy.clamp(r, ph as f32 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let inside = (r - d + 0.5).clamp(0.0, 1.0);
            if inside > 0.0 {
                let edge = (d - (r - 1.5)).clamp(0.0, 1.0).max(if x == 0 || y == 0 || x == w - 1 || y == ph - 1 { 1.0 } else { 0.0 });
                let c = if edge > 0.0 { BUBBLE_EDGE } else { PLATE };
                cv.blend(x as i32, y as i32, c, 0.93 * inside);
            }
        }
    }
    // Pointe : triangle dont la base est sur le bas de la plaque, sommet en bas à gauche.
    let base_l = (r + px * 0.4).min(w as f32 * 0.3);
    let base_r = base_l + px * 0.9;
    let tip = (base_l - px * 0.25, (ph + tail_h) as f32);
    for y in ph.saturating_sub(1)..h {
        let fy = y as f32 + 0.5;
        // Bords gauche et droit du triangle à cette hauteur.
        let t = ((fy - ph as f32) / tail_h as f32).clamp(0.0, 1.0);
        let xl = base_l + (tip.0 - base_l) * t;
        let xr = base_r + (tip.0 - base_r) * t;
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let cov = ((fx - xl + 0.5).min(xr - fx + 0.5)).clamp(0.0, 1.0);
            if cov > 0.0 {
                let border = fx - xl < 1.5 || xr - fx < 1.5 || fy > tip.1 - 1.5;
                cv.blend(x as i32, y as i32, if border { BUBBLE_EDGE } else { PLATE }, 0.93 * cov);
            }
        }
    }
    // Texte centré verticalement dans la plaque, chaque ligne centrée.
    let top = (ph as f32 - text_h) / 2.0;
    let asc = ascent * line_h / natural.max(1.0);
    for (j, row) in rows.iter().enumerate() {
        let x_off = (w as f32 - widths[j]) / 2.0 - row.first().map_or(0.0, |(x, _)| *x);
        let baseline = (top + j as f32 * line_h + asc).round() as i32;
        for (x, it) in row {
            let Item::Char(c, col) = it else { continue };
            let Some(f) = book.pick(base, *c) else { continue };
            let (m, bmp) = book.glyph(f, *c, px).clone();
            let gx = (x + x_off + m.xmin as f32).round() as i32;
            let gy = baseline - (m.ymin + m.height as i32);
            for (ox, oy, color, a) in [(1, 1, [0u8, 0, 0], 0.7f32), (0, 0, *col, 1.0)] {
                for by in 0..m.height {
                    for bx in 0..m.width {
                        let cov = bmp[by * m.width + bx];
                        if cov > 0 {
                            cv.blend(gx + bx as i32 + ox, gy + by as i32 + oy, color, a * cov as f32 / 255.0);
                        }
                    }
                }
            }
        }
    }
    let dx = -((w as i32 - s.width) / 2);
    // Coin bas-gauche de l'image : sous la plaque, la pointe.
    let dy = -(pad as i32) - tail_h as i32;
    Drawn { text: text.to_string(), canvas: cv, dx, dy }
}

/// Bulles de plusieurs relevés gardées par numéro (tests).
#[cfg(test)]
pub fn slots(b: &Bubbles) -> std::collections::HashMap<u16, u8> {
    b.states.iter().map(|s| (s.seen.id, s.slot)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(id: u16, left: i32, bottom: i32) -> Seen {
        Seen { id, left, bottom, width: 180, height: 30, color: [255, 255, 255], px: 16.0 }
    }

    pub fn payload(list: &[Seen], font: &str) -> Vec<u8> {
        let mut p = vec![list.len() as u8];
        for s in list {
            for v in [s.id, s.left as u16, s.bottom as u16, s.width as u16, s.height as u16] {
                p.extend_from_slice(&v.to_be_bytes());
            }
            p.extend_from_slice(&s.color);
            p.extend_from_slice(&((s.px * 10.0) as u16).to_be_bytes());
        }
        p.push(font.len() as u8);
        p.extend_from_slice(font.as_bytes());
        p
    }

    #[test]
    fn bubble_texts_after_the_font() {
        let list = vec![seen(1, 800, 500), seen(2, 300, 400)];
        let mut p = payload(&list, "Fonts\\FRIZQT__.TTF");
        assert_eq!(texts(&p, 2), vec![None, None], "addon d'avant 0.16.3");
        let t = "Привет всем";
        p.extend_from_slice(&[1, t.len() as u8]);
        p.extend_from_slice(t.as_bytes());
        p.extend_from_slice(&[0, 3]);
        p.extend_from_slice(b"Hi!");
        assert_eq!(texts(&p, 2), vec![Some("Hi!".to_string()), Some(t.to_string())]);
        assert_eq!(parse(&p).unwrap().0, list, "trame toujours lue par l'ancien format");
        // Texte tronqué en fin de trame : ignoré.
        p.extend_from_slice(&[0, 50, b'x']);
        assert_eq!(texts(&p, 2)[0].as_deref(), Some("Hi!"));
    }

    #[test]
    fn bubble_frame_round_trip() {
        let list = vec![seen(0x2A, 812, 540), Seen { color: [255, 64, 64], px: 13.3, ..seen(7, 1500, 610) }];
        let (got, font) = parse(&payload(&list, "Fonts\\FRIZQT__.TTF")).unwrap();
        assert_eq!(got, list);
        assert_eq!(font, "Fonts\\FRIZQT__.TTF");
        assert_eq!(parse(&[0, 0]).unwrap().0, vec![]);
        assert!(parse(&[1, 0, 1]).is_err());
    }

    #[test]
    fn translated_bubbles_follow_their_speaker_and_go_away() {
        let mut book = FontBook::new();
        for (name, data) in crate::fonts::fallback_fonts() {
            let _ = book.add_fallback(&name, data);
        }
        let mut b = Bubbles::default();
        let t0 = Instant::now();
        let tr = |body: &str| (body == "Guten Abend zusammen!").then(|| "[DE] Bonsoir à tous !".to_string());
        // Une bulle traduite, une bulle dans la langue du client : toutes deux dessinées (0.16.3,
        // la bulle de Blizzard est effacée), la seconde avec son texte d'origine.
        b.update(
            vec![(seen(1, 800, 500), Some("Guten Abend zusammen!".into())), (seen(2, 300, 400), Some("Salut".into()))],
            "Fonts\\FRIZQT__.TTF".into(),
            t0,
        );
        let ups = b.tick(&mut book, &tr, &[], t0);
        assert_eq!(ups.len(), 2, "chaque bulle a sa fenêtre");
        let u = ups.iter().find(|u| u.frame == FIRST_FRAME + slots(&b)[&1]).unwrap();
        let plain = ups.iter().find(|u| u.frame == FIRST_FRAME + slots(&b)[&2]).unwrap();
        assert!(plain.canvas.as_ref().is_some_and(|c| c.px.iter().any(|p| (p >> 16) & 0xFF > 200)), "texte d'origine absent");
        let cv = u.canvas.as_ref().unwrap();
        // La plaque recouvre le texte d'origine (centrée, débordant de la marge).
        assert!(cv.w as i32 >= 180 && cv.h as i32 >= 30);
        assert!(u.left <= 800 && u.left + cv.w as i32 >= 980 && u.bottom <= 500);
        // Pointe en bas à gauche : pixels dessinés sous la plaque à gauche, rien à droite.
        let row = cv.h - 3;
        let painted = |x: usize| cv.px[row * cv.w + x] >> 24 != 0;
        assert!((0..cv.w / 3).any(painted), "pointe absente");
        assert!(!(cv.w * 2 / 3..cv.w).any(painted), "pointe mal placée");
        // Du texte clair est dessiné sur la plaque sombre.
        assert!(cv.px.iter().any(|p| (p >> 16) & 0xFF > 200), "texte absent");
        // Même place : rien de renvoyé.
        assert!(b.tick(&mut book, &tr, &[], t0).is_empty());
        // La bulle se déplace de 50 px en 0,1 s : la fenêtre la suit, en avance sur le relevé.
        let t1 = t0 + Duration::from_millis(100);
        b.update(vec![(seen(1, 850, 500), Some("Guten Abend zusammen!".into()))], String::new(), t1);
        let ups = b.tick(&mut book, &tr, &[], t1);
        let moved = ups.iter().find(|u| u.canvas.is_some()).expect("bulle déplacée");
        assert!(moved.left > u.left + 50, "prédiction de la place : {} puis {}", u.left, moved.left);
        // Sous une autre fenêtre : effacée à cet endroit.
        let t2 = t1 + Duration::from_millis(50);
        let ups = b.tick(&mut book, &tr, &[(0, 0, 4000, 4000)], t2);
        if let Some(u) = ups.iter().find(|u| u.canvas.is_some()) {
            assert!(u.canvas.as_ref().unwrap().px.iter().all(|p| *p == 0));
        }
        // Plus de nouvelles de l'addon : cachée.
        let ups = b.tick(&mut book, &tr, &[], t1 + STALE + Duration::from_millis(10));
        assert!(ups.iter().any(|u| u.canvas.is_none() && u.frame == FIRST_FRAME + 0), "bulle oubliée");
        assert!(!b.active());
        // Bulle retirée par l'addon (liste vide) : cachée au passage suivant.
        b.update(vec![(seen(3, 100, 100), Some("Guten Abend zusammen!".into()))], String::new(), t2);
        assert_eq!(b.tick(&mut book, &tr, &[], t2).len(), 1);
        b.update(Vec::new(), String::new(), t2);
        let ups = b.tick(&mut book, &tr, &[], t2);
        assert_eq!(ups.len(), 1);
        assert!(ups[0].canvas.is_none());
    }
}

