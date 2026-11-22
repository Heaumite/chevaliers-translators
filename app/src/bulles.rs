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
//!
//! 0.17.0 : une bulle vit 60 s, apparaît en fondu pendant ses 10 premières secondes et s'efface
//! pendant ses 10 dernières. Plusieurs bulles qui se chevauchent s'empilent : la plus ancienne en
//! bas, les plus récentes au-dessus ; quand une bulle s'en va, celles du dessus redescendent en
//! glissant. Nouveau dessin : plaque sombre en dégradé, ombre douce, liseré fin, barre d'accent
//! aux couleurs de la classe, acronyme de la langue en couleur d'accent.

use std::time::{Duration, Instant};

use super::{advances, parse_markup, wrap, Canvas, FontBook, Item, Layout, Rgb, Update};

/// Première fenêtre des bulles (les fenêtres de chat sont numérotées de 1 à ~10, les panneaux
/// à partir de 248).
pub const FIRST_FRAME: u8 = 200;
pub const MAX_BUBBLES: usize = 8;
/// Bulle sans nouvelles de l'addon depuis ce temps : retirée (il renvoie toutes les 2 s).
const STALE: Duration = Duration::from_millis(3500);
/// Vie d'une bulle (0.17.0) : 60 s après son apparition (l'addon la garde après celle du jeu),
/// apparue en fondu pendant ses 10 premières secondes, effacée pendant ses 10 dernières.
pub const LIFE: Duration = Duration::from_secs(60);
pub const FADE_IN: f32 = 10.0;
pub const FADE_OUT: f32 = 10.0;
/// Paliers de transparence du fondu (une image redessinée par palier).
const FADE_STEPS: f32 = 24.0;
/// Glissement des bulles empilées vers leur place (part de l'écart comblée par seconde).
const SLIDE: f32 = 9.0;
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
    /// Classe et spécialisation de celui qui parle (addon 0.16.4).
    pub style: Option<Style>,
}

/// Couleur de la classe (bord et teinte de la bulle) et icône de la spécialisation (numéro de
/// fichier du jeu, 0 : inconnue).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub class: Rgb,
    pub icon: u32,
}

/// Styles des bulles placés après la police (addon 0.16.4) : [64 + rang] [7] [r g b] [icône u32].
pub fn styles(p: &[u8], n: usize) -> Vec<Option<Style>> {
    let mut out = vec![None; n];
    let need = 1 + 15 * n;
    let Some(&len) = p.get(need) else { return out };
    let mut i = need + 1 + len as usize;
    while i + 2 <= p.len() {
        let (k, l) = (p[i] as usize, p[i + 1] as usize);
        let Some(b) = p.get(i + 2..i + 2 + l) else { break };
        if (64..64 + n).contains(&k) && l >= 7 {
            out[k - 64] = Some(Style { class: [b[0], b[1], b[2]], icon: u32::from_be_bytes([b[3], b[4], b[5], b[6]]) });
        }
        i += 2 + l;
    }
    out
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
                style: None,
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
    /// Marge de l'ombre autour de la bulle dans l'image (0.17.0).
    pub margin: i32,
}

struct State {
    seen: Seen,
    body: String,
    at: Instant,
    /// Apparition : la bulle s'efface en fin de vie (LIFE).
    born: Instant,
    /// Vitesse mesurée (pixels par seconde).
    vel: (f32, f32),
    slot: u8,
    drawn: Option<Drawn>,
    /// Dernière place posée (gauche, bas, texte, palier de transparence) : rien n'est renvoyé si
    /// elle ne change pas.
    posted: Option<(i32, i32, String, u8)>,
    /// Hauteur dont la bulle est remontée au-dessus des plus anciennes qu'elle chevauche (0.17.0),
    /// telle qu'affichée (elle glisse vers sa place).
    lift: f32,
}

/// Opacité d'une bulle selon son âge (0.17.0) : apparition en fondu pendant FADE_IN secondes
/// (lisible tôt : la montée ralentit vers la fin), pleine, puis effacée pendant les FADE_OUT
/// dernières secondes.
pub fn fade_level(age: Duration) -> u8 {
    let a = age.as_secs_f32();
    let t_in = (a / FADE_IN).clamp(0.0, 1.0);
    let fade_in = 1.0 - (1.0 - t_in) * (1.0 - t_in);
    let fade_out = ((LIFE.as_secs_f32() - a) / FADE_OUT).clamp(0.0, 1.0);
    (fade_in.min(fade_out) * FADE_STEPS).ceil() as u8
}

/// Hauteurs de remontée des bulles empilées (0.17.0). `rects` : (gauche, bas, largeur, hauteur)
/// de chaque bulle à sa place naturelle, de la plus ancienne à la plus récente. Chaque bulle monte
/// juste au-dessus des plus anciennes qu'elle chevaucherait.
pub fn stack_lifts(rects: &[(i32, i32, i32, i32)], gap: i32) -> Vec<i32> {
    let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut lifts = Vec::with_capacity(rects.len());
    for &(l, b, w, h) in rects {
        let mut lift = 0;
        // Au plus une montée par bulle déjà placée : la boucle s'arrête toujours.
        for _ in 0..=placed.len() {
            let bottom = b + lift;
            let hit = placed.iter().filter(|&&(pl, pb, pw, ph)| l < pl + pw && pl < l + w && bottom < pb + ph + gap && pb < bottom + h).map(|&(_, pb, _, ph)| pb + ph + gap).max();
            match hit {
                Some(top) => lift = top - b,
                None => break,
            }
        }
        placed.push((l, b + lift, w, h));
        lifts.push(lift);
    }
    lifts
}

#[derive(Default)]
pub struct Bubbles {
    states: Vec<State>,
    pub font: String,
    /// Fenêtres à cacher au prochain passage.
    to_hide: Vec<u8>,
    pub seen_total: usize,
    /// Bulles arrivées en fin de vie : plus reprises si l'addon les renvoie encore (une minute).
    expired: Vec<(u16, Instant)>,
    /// Passage précédent (glissement des bulles empilées).
    last_tick: Option<Instant>,
}

impl Bubbles {
    /// Relevé de l'addon : `list` avec le texte d'origine de chaque bulle (`None` : message
    /// inconnu de l'application, bulle ignorée).
    pub fn update(&mut self, list: Vec<(Seen, Option<String>)>, font: String, now: Instant) {
        if !font.is_empty() {
            self.font = font;
        }
        let mut next: Vec<State> = Vec::new();
        self.expired.retain(|(_, t)| now.saturating_duration_since(*t) < LIFE + Duration::from_secs(30));
        for (s, body) in list.into_iter().take(MAX_BUBBLES) {
            let Some(body) = body else { continue };
            if self.expired.iter().any(|(id, _)| *id == s.id) {
                continue;
            }
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
                    if (o.seen.width, o.seen.height, o.seen.color, (o.seen.px * 10.0) as i32, o.seen.style)
                        != (s.width, s.height, s.color, (s.px * 10.0) as i32, s.style)
                    {
                        o.drawn = None;
                    }
                    o.seen = s;
                    o.body = body;
                    o.at = now;
                    next.push(o);
                }
                None => next.push(State { seen: s, body, at: now, born: now, vel: (0.0, 0.0), slot: u8::MAX, drawn: None, posted: None, lift: 0.0 }),
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

    /// Icône arrivée des archives du jeu : les bulles qui l'attendaient sont redessinées.
    pub fn icons_changed(&mut self) {
        for s in &mut self.states {
            s.drawn = None;
            s.posted = None;
        }
    }

    /// Écran de chargement (0.17.0) : les bulles d'avant ne sont plus à leur place ; toutes
    /// retirées, et pas reprises si l'addon les renvoie.
    pub fn reset(&mut self, now: Instant) {
        for s in self.states.drain(..) {
            self.expired.push((s.seen.id, now));
            if s.posted.is_some() {
                self.to_hide.push(FIRST_FRAME + s.slot);
            }
        }
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
        let expired = &mut self.expired;
        self.states.retain(|s| {
            let alive = now.saturating_duration_since(s.born) < LIFE;
            if !alive {
                expired.push((s.seen.id, now));
            }
            let keep = alive && now.saturating_duration_since(s.at) < STALE;
            if !keep && s.posted.is_some() {
                ups.push(Update { frame: FIRST_FRAME + s.slot, left: 0, bottom: 0, canvas: None, hits: Vec::new() });
            }
            keep
        });
        let dt = self.last_tick.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32()).min(0.25);
        self.last_tick = Some(now);
        // Place naturelle de chaque bulle (prédite), puis empilement : la plus ancienne en bas.
        let mut natural: Vec<(i32, i32, String)> = Vec::with_capacity(self.states.len());
        for s in &mut self.states {
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
            natural.push((left, bottom, text));
        }
        let mut order: Vec<usize> = (0..self.states.len()).collect();
        order.sort_by_key(|&i| self.states[i].born);
        // Rectangle de chaque bulle sans son ombre (gauche, bas, largeur, hauteur).
        let rects: Vec<(i32, i32, i32, i32)> = order
            .iter()
            .map(|&i| {
                let d = self.states[i].drawn.as_ref().unwrap();
                let m = d.margin;
                (natural[i].0 + m, natural[i].1 + m, d.canvas.w as i32 - 2 * m, d.canvas.h as i32 - 2 * m)
            })
            .collect();
        let lifts = stack_lifts(&rects, 3);
        for (k, &i) in order.iter().enumerate() {
            let s = &mut self.states[i];
            let target = lifts[k] as f32;
            // Nouvelle bulle : directement à sa place ; ensuite, glissement vers la place.
            if s.posted.is_none() && s.lift == 0.0 {
                s.lift = target;
            } else {
                s.lift += (target - s.lift) * (dt * SLIDE).min(1.0);
                if (target - s.lift).abs() < 0.5 {
                    s.lift = target;
                }
            }
        }
        for (i, s) in self.states.iter_mut().enumerate() {
            let frame = FIRST_FRAME + s.slot;
            let d = s.drawn.as_ref().unwrap();
            let (left, bottom0, ref text) = natural[i];
            let text = text.clone();
            let bottom = bottom0 + s.lift.round() as i32;
            let level = fade_level(now.saturating_duration_since(s.born));
            let key = (left, bottom, text.clone(), level);
            if s.posted.as_ref() == Some(&key) {
                continue;
            }
            let mut cv = d.canvas.clone();
            if (level as f32) < FADE_STEPS {
                fade(&mut cv, level as f32 / FADE_STEPS);
            }
            clear_under(&mut cv, left, bottom, occ);
            s.posted = Some(key);
            ups.push(Update { frame, left, bottom, canvas: Some(cv), hits: Vec::new() });
        }
        ups
    }
}

/// Transparence appliquée à toute l'image (pixels prémultipliés : chaque canal réduit).
fn fade(cv: &mut Canvas, a: f32) {
    let k = (a.clamp(0.0, 1.0) * 256.0) as u32;
    for p in &mut cv.px {
        let c = *p;
        let ch = |shift: u32| ((((c >> shift) & 0xFF) * k) >> 8) << shift;
        *p = ch(24) | ch(16) | ch(8) | ch(0);
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

/// Distance signée d'un point à un rectangle aux coins arrondis (négative dedans).
fn round_box(fx: f32, fy: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (qx, qy) = ((fx - cx).abs() - hx, (fy - cy).abs() - hy);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

/// « [DE] Bonsoir » -> (« DE », « Bonsoir ») : l'acronyme de la langue d'origine.
fn lang_tag(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('[')?;
    let (code, after) = rest.split_once("] ")?;
    ((2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_uppercase())).then_some((code, after))
}

/// Haut et bas du dégradé de la plaque (0.17.0).
const PLATE_TOP: Rgb = [38, 40, 48];
const PLATE_BOTTOM: Rgb = [17, 18, 23];
/// Accent sans classe connue : or doux.
const ACCENT: Rgb = [214, 178, 110];

/// Bulle (0.17.0) : plaque sombre en dégradé aux coins arrondis, ombre douce, liseré fin, barre
/// d'accent à gauche aux couleurs de la classe, pointe en bas à gauche vers celui qui parle. Au
/// moins la taille du texte d'origine, centrée sur lui, s'agrandissant vers le haut si la
/// traduction est plus longue ; texte centré ligne par ligne, comme les bulles du jeu ; acronyme
/// de la langue d'origine en couleur d'accent.
pub fn draw(book: &mut FontBook, font: &str, s: &Seen, text: &str) -> Drawn {
    let px = s.px.clamp(8.0, 64.0);
    let base = book.base(font);
    let (ascent, natural) = book.line_metrics(base, px);
    let line_h = natural.max(px);
    let layout = Layout::new(book, base, px, line_h, None);
    let accent = s.style.map_or(ACCENT, |st| st.class);
    let tag_c = mix(accent, [255, 255, 255], 0.35);
    // Acronyme en couleur d'accent, icône de la spécialisation en tête (0.16.4).
    let tagged = match lang_tag(text) {
        Some((code, rest)) => format!("|cff{:02x}{:02x}{:02x}[{code}]|r {rest}", tag_c[0], tag_c[1], tag_c[2]),
        None => text.to_string(),
    };
    let text_in = match s.style {
        Some(st) if st.icon > 0 => format!("|T{}:0|t {tagged}", st.icon),
        _ => tagged,
    };
    let items = parse_markup(&text_in, s.color);
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
    // Mesures de la bulle.
    let pad_x = (px * 0.7).round().max(5.0);
    let pad_y = (px * 0.45).round().max(4.0);
    let bar = (px * 0.2).round().clamp(2.0, 4.0);
    let inset = (px * 0.35).round().max(3.0);
    let bar_space = inset + bar;
    let plate_w = (content_w.max(s.width as f32) + 2.0 * pad_x + bar_space).ceil();
    let text_h = rows.len().max(1) as f32 * line_h;
    let plate_h = (text_h.max(s.height as f32) + 2.0 * pad_y).ceil();
    let tail_h = (px * 0.65).round().max(6.0);
    // Marge de l'ombre autour de la bulle.
    let m = (px * 0.55).round().clamp(4.0, 12.0);
    let shadow_dy = (m / 3.0).max(1.0);
    let w = (plate_w + 2.0 * m) as usize;
    let h = (plate_h + tail_h + 2.0 * m) as usize;
    let mut cv = Canvas::new(w, h);
    let (x0, y0, x1, y1) = (m, m, m + plate_w, m + plate_h);
    let r = (px * 0.65).max(5.0).min(plate_h / 2.0);
    // Pointe : base sur le bas de la plaque (prolongée dedans pour s'y raccorder), sommet en bas
    // à gauche.
    let base_l = (x0 + r + px * 0.25).min(x0 + plate_w * 0.3);
    let base_r = base_l + px * 1.05;
    let tip = (base_l - px * 0.25, y1 + tail_h);
    let tail_cov = |fx: f32, fy: f32, grow: f32| -> f32 {
        if fy < y1 - 3.0 || fy > tip.1 + grow + 0.5 {
            return 0.0;
        }
        let t = ((fy - y1) / tail_h).clamp(0.0, 1.0);
        let xl = base_l + (tip.0 - base_l) * t - grow;
        let xr = base_r + (tip.0 - base_r) * t + grow;
        ((fx - xl + 0.5).min(xr - fx + 0.5)).clamp(0.0, 1.0) * (tip.1 + grow + 0.5 - fy).clamp(0.0, 1.0)
    };
    let tint = |c: Rgb| match s.style {
        Some(st) => mix(c, st.class, 0.12),
        None => c,
    };
    let (top_c, bottom_c) = (tint(PLATE_TOP), tint(PLATE_BOTTOM));
    let (line_c, line_a) = match s.style {
        Some(st) => (st.class, 0.6),
        None => ([255, 255, 255], 0.16),
    };
    // 1. Ombre douce, un peu décalée vers le bas.
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5 - shadow_dy);
            let d = round_box(fx, fy, x0, y0, x1, y1, r);
            let mut a = 0.40 * (1.0 - (d / m).clamp(0.0, 1.0)).powi(2);
            a = a.max(0.30 * tail_cov(fx, fy, 1.0));
            if a > 0.004 {
                cv.blend(x as i32, y as i32, [0, 0, 0], a);
            }
        }
    }
    // 2. Plaque en dégradé, liseré fin au bord.
    for y in 0..h {
        let fy = y as f32 + 0.5;
        let g = ((fy - y0) / plate_h).clamp(0.0, 1.0);
        let fill = mix(top_c, bottom_c, g);
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let d = round_box(fx, fy, x0, y0, x1, y1, r);
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            cv.blend(x as i32, y as i32, fill, 0.92 * cov);
            let line = cov - (0.5 - (d + 1.2)).clamp(0.0, 1.0);
            if line > 0.0 {
                cv.blend(x as i32, y as i32, line_c, line_a * line);
            }
        }
    }
    // 3. Pointe, du ton du bas de la plaque, liseré sur ses deux bords obliques.
    for y in (y1 as usize).saturating_sub(3)..h {
        let fy = y as f32 + 0.5;
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let cov = tail_cov(fx, fy, 0.0);
            if cov <= 0.0 {
                continue;
            }
            cv.blend(x as i32, y as i32, bottom_c, if fy < y1 { 1.0 } else { 0.92 } * cov);
            if fy > y1 + 0.5 {
                let inner = tail_cov(fx, fy, -1.2);
                let line = (cov - inner).clamp(0.0, 1.0);
                if line > 0.0 {
                    cv.blend(x as i32, y as i32, line_c, line_a * line);
                }
            }
        }
    }
    // 4. Barre d'accent à gauche, bouts arrondis.
    let (bx0, bx1) = (x0 + inset, x0 + inset + bar);
    let (by0, by1) = (y0 + pad_y * 0.8, (y1 - pad_y * 0.8).max(y0 + pad_y * 0.8 + bar));
    for y in by0.floor() as usize..(by1.ceil() as usize).min(h) {
        for x in bx0.floor() as usize..(bx1.ceil() as usize).min(w) {
            let d = round_box(x as f32 + 0.5, y as f32 + 0.5, bx0, by0, bx1, by1, bar / 2.0);
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(x as i32, y as i32, accent, cov);
            }
        }
    }
    // 5. Texte centré verticalement dans la plaque, chaque ligne centrée à droite de la barre.
    let top = y0 + (plate_h - text_h) / 2.0;
    let asc = ascent * line_h / natural.max(1.0);
    let inner_l = x0 + bar_space;
    let inner_w = plate_w - bar_space;
    for (j, row) in rows.iter().enumerate() {
        let x_off = inner_l + (inner_w - widths[j]) / 2.0 - row.first().map_or(0.0, |(x, _)| *x);
        let baseline = (top + j as f32 * line_h + asc).round() as i32;
        for (x, it) in row {
            if let Item::Icon(ic) = it {
                // Icône (spécialisation) : centrée sur la ligne.
                let (iw, ih) = crate::icons::size_px(ic, px, 1.0);
                let (iw, ih) = (iw.round().max(1.0) as usize, ih.round().max(1.0) as usize);
                let ix = (x + x_off).round() as i32;
                let iy = (top + j as f32 * line_h + (line_h - ih as f32) / 2.0).round() as i32;
                let img = crate::icons::image_or_shape(ic.id, iw, ih);
                for yy in 0..ih {
                    for xx in 0..iw {
                        let p = img.px[yy * iw + xx];
                        if p[3] > 0 {
                            cv.blend(ix + xx as i32, iy + yy as i32, [p[0], p[1], p[2]], p[3] as f32 / 255.0);
                        }
                    }
                }
                continue;
            }
            let Item::Char(c, col) = it else { continue };
            let Some(f) = book.pick(base, *c) else { continue };
            let (gm, bmp) = book.glyph(f, *c, px).clone();
            let gx = (x + x_off + gm.xmin as f32).round() as i32;
            let gy = baseline - (gm.ymin + gm.height as i32);
            for (ox, oy, color, a) in [(1, 1, [0u8, 0, 0], 0.55f32), (0, 0, *col, 1.0)] {
                for by in 0..gm.height {
                    for bx in 0..gm.width {
                        let cov = bmp[by * gm.width + bx];
                        if cov > 0 {
                            cv.blend(gx + bx as i32 + ox, gy + by as i32 + oy, color, a * cov as f32 / 255.0);
                        }
                    }
                }
            }
        }
    }
    // Plaque centrée sur le texte d'origine ; coin bas-gauche de l'image sous la pointe et l'ombre.
    let margin = m as i32;
    let dx = -((plate_w as i32 - s.width) / 2) - margin;
    let dy = -(pad_y as i32) - tail_h as i32 - margin;
    Drawn { text: text.to_string(), canvas: cv, dx, dy, margin }
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
        Seen { id, left, bottom, width: 180, height: 30, color: [255, 255, 255], px: 16.0, style: None }
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
    fn bubble_styles_after_the_font() {
        let list = vec![seen(1, 800, 500), seen(2, 300, 400)];
        let mut p = payload(&list, "Fonts\\FRIZQT__.TTF");
        // Style de la seconde bulle (chasseur, icône 461112), texte de la première.
        p.extend_from_slice(&[65, 7, 0xAA, 0xD3, 0x72]);
        p.extend_from_slice(&461112u32.to_be_bytes());
        p.extend_from_slice(&[0, 3]);
        p.extend_from_slice(b"Hi!");
        assert_eq!(styles(&p, 2), vec![None, Some(Style { class: [0xAA, 0xD3, 0x72], icon: 461112 })]);
        assert_eq!(texts(&p, 2), vec![Some("Hi!".to_string()), None], "les styles ne gênent pas les textes");
        // Bulle aux couleurs de la classe : bord de cette couleur.
        let mut book = FontBook::new();
        for (name, data) in crate::fonts::fallback_fonts() {
            let _ = book.add_fallback(&name, data);
        }
        let s = Seen { style: Some(Style { class: [0xAA, 0xD3, 0x72], icon: 0 }), ..seen(1, 0, 0) };
        let d = draw(&mut book, "x", &s, "Hello");
        // Barre d'accent aux couleurs de la classe (0.17.0) : à gauche, à mi-hauteur de la plaque.
        let row = d.margin as usize + (d.canvas.h - 2 * d.margin as usize) / 3;
        let classy = (0..d.canvas.w / 4).any(|x| {
            let p = d.canvas.px[row * d.canvas.w + x];
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            g > 150 && g > r + 20 && g > b + 40
        });
        assert!(classy, "barre d'accent aux couleurs de la classe absente");
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
    fn bubbles_live_sixty_seconds_with_fades() {
        // 0.17.0 : apparition en fondu sur 10 s, pleine, effacement sur les 10 dernières.
        assert_eq!(fade_level(Duration::ZERO), 0);
        assert!(fade_level(Duration::from_secs(2)) >= 8, "à peine lisible à 2 s : {}", fade_level(Duration::from_secs(2)));
        assert_eq!(fade_level(Duration::from_secs(10)), 24);
        assert_eq!(fade_level(Duration::from_secs(49)), 24);
        assert_eq!(fade_level(Duration::from_secs(55)), 12);
        assert_eq!(fade_level(LIFE), 0);
        let mut book = FontBook::new();
        for (name, data) in crate::fonts::fallback_fonts() {
            let _ = book.add_fallback(&name, data);
        }
        let mut b = Bubbles::default();
        let t0 = Instant::now();
        let tr = |_: &str| Some("[DE] Bonsoir".to_string());
        let alpha_max = |u: &Update| u.canvas.as_ref().map_or(0, |c| c.px.iter().map(|p| p >> 24).max().unwrap_or(0));
        let at = |b: &mut Bubbles, book: &mut FontBook, secs: u64| {
            let t = t0 + Duration::from_secs(secs);
            b.update(vec![(seen(1, 800, 500), Some("Guten Abend".into()))], String::new(), t);
            b.tick(book, &tr, &[], t)
        };
        b.update(vec![(seen(1, 800, 500), Some("Guten Abend".into()))], "Fonts\\FRIZQT__.TTF".into(), t0);
        let start = b.tick(&mut book, &tr, &[], t0);
        assert_eq!(alpha_max(&start[0]), 0, "invisible à son apparition");
        let rising = alpha_max(&at(&mut b, &mut book, 3)[0]);
        let full = alpha_max(&at(&mut b, &mut book, 12)[0]);
        assert!(rising > 0 && rising < full, "fondu entrant : {rising} puis {full}");
        // L'addon la garde après la bulle du jeu : à 55 s, à moitié effacée.
        let half = alpha_max(&at(&mut b, &mut book, 55)[0]);
        assert!(half < full * 6 / 10 && half > full * 4 / 10, "{full} -> {half}");
        // 60 s : retirée.
        let ups = at(&mut b, &mut book, 60);
        assert!(ups.iter().all(|u| u.canvas.is_none()) && !b.active());
        // Renvoyée encore par l'addon : pas reprise.
        b.update(vec![(seen(1, 800, 500), Some("Guten Abend".into()))], String::new(), t0 + Duration::from_secs(61));
        assert!(!b.active(), "bulle expirée reprise");
    }

    #[test]
    fn overlapping_bubbles_stack_newest_on_top_and_slide_back_down() {
        // Empilement pur : la plus ancienne reste en bas, les suivantes montent au-dessus.
        let lifts = stack_lifts(&[(100, 500, 200, 40), (120, 505, 180, 40), (110, 500, 200, 40), (900, 500, 100, 40)], 3);
        assert_eq!(lifts, vec![0, 38, 86, 0]);
        let mut book = FontBook::new();
        for (name, data) in crate::fonts::fallback_fonts() {
            let _ = book.add_fallback(&name, data);
        }
        let mut b = Bubbles::default();
        let t0 = Instant::now();
        let tr = |body: &str| Some(format!("[DE] {body}"));
        let list = |ids: &[u16]| ids.iter().map(|&id| (seen(id, 800, 500), Some(format!("Message {id}")))).collect::<Vec<_>>();
        // Deux répliques du même joueur, à la même place.
        b.update(list(&[1]), "Fonts\\FRIZQT__.TTF".into(), t0);
        b.tick(&mut book, &tr, &[], t0);
        let t1 = t0 + Duration::from_secs(2);
        b.update(list(&[1, 2]), String::new(), t1);
        let ups = b.tick(&mut book, &tr, &[], t1);
        let pos = |ups: &[Update], b: &Bubbles, id: u16| ups.iter().find(|u| u.frame == FIRST_FRAME + slots(b)[&id] && u.canvas.is_some()).map(|u| (u.bottom, u.canvas.as_ref().unwrap().h as i32));
        let old = b.states.iter().find(|s| s.seen.id == 1).unwrap().posted.clone().unwrap();
        let (new_bottom, _) = pos(&ups, &b, 2).expect("nouvelle bulle posée");
        assert!(new_bottom > old.1 + 20, "la plus récente au-dessus : {new_bottom} / {}", old.1);
        // La plus ancienne s'en va : la récente redescend en glissant (pas d'un coup).
        let t2 = t1 + Duration::from_millis(100);
        b.update(list(&[2]), String::new(), t2);
        let ups = b.tick(&mut book, &tr, &[], t2);
        let (mid, _) = pos(&ups, &b, 2).expect("bulle en train de descendre");
        assert!(mid < new_bottom && mid > old.1, "glissement : {new_bottom} -> {mid} -> {}", old.1);
        let mut t = t2;
        for _ in 0..30 {
            t += Duration::from_millis(50);
            b.update(list(&[2]), String::new(), t);
            b.tick(&mut book, &tr, &[], t);
        }
        let last = b.states[0].posted.clone().unwrap();
        assert_eq!(last.1, old.1, "revenue à sa place naturelle");
        // Aperçu de la bulle pour l'œil (CT_APERCU=dossier).
        if let Ok(dir) = std::env::var("CT_APERCU") {
            for (name, st, text) in [
                ("bulle_classe.bmp", Some(Style { class: [0x33, 0x93, 0xFF], icon: 0 }), "[DE] Bonsoir à tous, qui vient au donjon ce soir ?"),
                ("bulle_simple.bmp", None, "Salut, on se retrouve à l'entrée ?"),
            ] {
                let s = Seen { style: st, ..seen(1, 0, 0) };
                let d = draw(&mut book, "x", &s, text);
                std::fs::write(std::path::Path::new(&dir).join(name), d.canvas.to_bmp([92, 110, 84])).unwrap();
            }
        }
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
        let both = || vec![(seen(1, 800, 500), Some("Guten Abend zusammen!".into())), (seen(2, 300, 400), Some("Salut".into()))];
        b.update(both(), "Fonts\\FRIZQT__.TTF".into(), t0);
        // Fondu entrant terminé (0.17.0) : 10 s plus tard, l'addon les a renvoyées.
        let t0 = t0 + Duration::from_secs(10);
        b.update(both(), String::new(), t0);
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
        // (Sous la pointe : la marge de l'ombre, 0.17.0 ; l'ombre seule est presque transparente.)
        let m = b.states.iter().find(|s| s.seen.id == 1).unwrap().drawn.as_ref().unwrap().margin as usize;
        let row = cv.h - m - 3;
        let painted = |x: usize| cv.px[row * cv.w + x] >> 24 > 120;
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

