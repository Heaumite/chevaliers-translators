//! Panneau des tests lancés depuis WoW : l'application le dessine par-dessus le jeu (fenêtre
//! transparente, au-dessus de WoW, qui laisse passer les clics), en haut à droite, loin du
//! bandeau de l'addon (en bas à droite). Le joueur lance le test, le suit et lit le verdict
//! sans quitter WoW.

// Utilisé par la boucle Windows ; ailleurs, seulement par les tests.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;

use crate::bench::Status;
use crate::catalog::Verdict;
use crate::mirror::{Canvas, Rgb};

// Couleurs de la bannière et du logo, comme la fenêtre des modèles : champ noir, filigrane
// d'or, texte parchemin ; sinople et gueules pour les verdicts.
/// Filigrane d'or du cadre.
const GOLD: Rgb = [201, 165, 90];
/// Titres (or clair du filigrane).
pub const TITLE: Rgb = [230, 199, 126];
/// Texte (parchemin).
pub const TEXT: Rgb = [236, 226, 198];
/// Texte secondaire.
pub const DIM: Rgb = [168, 158, 133];
const OK: Rgb = [147, 194, 90];
const BAD: Rgb = [236, 90, 82];
const WARN: Rgb = [230, 160, 70];
/// Champ noir de la bannière.
const FIELD: Rgb = [16, 15, 11];

/// Contenu du panneau.
#[derive(Clone, Debug, Default)]
pub struct Panel {
    pub title: String,
    pub phase: String,
    pub last: Option<String>,
    pub notes: Vec<String>,
    pub result: Vec<String>,
    pub verdict: Option<(Verdict, bool)>,
    pub error: Option<String>,
    /// Lignes déjà colorées (bilan d'une série).
    pub colored: Vec<(String, Rgb)>,
    pub footer: String,
    pub done: bool,
}

/// Couleur d'un verdict : vert si compatible et noté, rouge si plantage ou mémoire,
/// orange sinon (trop lourd, mesure incomplète, sauté).
pub fn verdict_color(v: Option<Verdict>, recorded: bool) -> Rgb {
    match v {
        Some(Verdict::Ok) if recorded => OK,
        Some(Verdict::Crash) | Some(Verdict::Memory) => BAD,
        None => DIM,
        _ => WARN,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: String,
    pub color: Rgb,
    pub title: bool,
}

impl Panel {
    pub fn start(title: &str) -> Panel {
        Panel {
            title: title.into(),
            phase: "Préparation".into(),
            footer: "Ne bouge pas jusqu'au verdict. /chevaliers essai stop pour arrêter.".into(),
            ..Default::default()
        }
    }

    /// Panneau d'accueil : l'application attend les demandes du jeu.
    pub fn ready() -> Panel {
        Panel {
            title: "Tests en jeu prêts".into(),
            phase: "Dans le chat : /chevaliers essai (série complète, environ 15 min)".into(),
            footer: "Personnage immobile dans un endroit calme ; le verdict s'affiche ici.".into(),
            done: true,
            ..Default::default()
        }
    }

    pub fn apply(&mut self, s: Status) {
        match s {
            Status::Phase(p) => self.phase = p,
            Status::Translation(t) => self.last = Some(t),
            Status::Note(n) => {
                self.notes.push(n);
                if self.notes.len() > 3 {
                    self.notes.remove(0);
                }
            }
            Status::Done { lines, verdict, recorded, aborted } => {
                self.done = true;
                self.phase = if aborted { "Test arrêté".into() } else { "Test terminé".into() };
                self.last = None;
                self.result = lines;
                self.verdict = (!aborted).then_some((verdict, recorded));
                self.footer = if recorded {
                    "Noté pour ce PC. /chevaliers essai fermer pour masquer ce panneau.".into()
                } else {
                    "Rien n'est noté pour ce PC. /chevaliers essai fermer pour masquer ce panneau.".into()
                };
            }
        }
    }

    pub fn fail(&mut self, e: &str) {
        self.done = true;
        self.phase = "Test impossible".into();
        self.error = Some(e.to_string());
        self.footer = "/chevaliers essai fermer pour masquer ce panneau.".into();
    }

    /// Lignes à dessiner, avec leur couleur.
    pub fn rows(&self) -> Vec<Row> {
        let row = |t: &str, c: Rgb| Row { text: t.to_string(), color: c, title: false };
        let mut out = vec![Row { text: self.title.clone(), color: TITLE, title: true }, row(&self.phase, TEXT)];
        for (t, c) in &self.colored {
            out.push(row(t, *c));
        }
        if let Some(l) = &self.last {
            out.push(row(l, DIM));
        }
        for n in &self.notes {
            out.push(row(n, WARN));
        }
        if let Some(e) = &self.error {
            out.push(row(e, BAD));
        }
        for l in &self.result {
            let c = if l.starts_with("Verdict") || l.starts_with("Test arrêté") {
                match self.verdict {
                    Some((Verdict::Ok, true)) => OK,
                    Some((Verdict::Ok, false)) | None => WARN,
                    Some((Verdict::Heavy, _)) => WARN,
                    Some(_) => BAD,
                }
            } else if l.starts_with("Pilote graphique : aucune") {
                OK
            } else if l.starts_with("Pilote graphique") {
                BAD
            } else if l.starts_with("Résultat noté") {
                continue;
            } else {
                TEXT
            };
            out.push(row(l, c));
        }
        out.push(row(&self.footer, DIM));
        out
    }
}

// --- Dessin -------------------------------------------------------------------------------

/// Polices : titre dans celle de WoW si elle est en cache, texte en Segoe UI ou Arial
/// (cyrillique et grec compris, pour les traductions affichées).
pub struct Fonts {
    title: Vec<fontdue::Font>,
    body: Vec<fontdue::Font>,
}

impl Fonts {
    pub fn load() -> Fonts {
        let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let mut paths: Vec<PathBuf> = ["segoeui.ttf", "arial.ttf", "seguisym.ttf"]
            .iter()
            .map(|f| PathBuf::from(&win).join("Fonts").join(f))
            .collect();
        paths.extend(
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            ]
            .iter()
            .map(PathBuf::from),
        );
        let load = |d: Vec<u8>| fontdue::Font::from_bytes(d, fontdue::FontSettings::default()).ok();
        let body: Vec<fontdue::Font> = paths.iter().filter_map(|p| std::fs::read(p).ok()).filter_map(load).collect();
        let mut title: Vec<fontdue::Font> = crate::ui::game_font().and_then(load).into_iter().collect();
        title.extend(paths.iter().filter_map(|p| std::fs::read(p).ok()).filter_map(load));
        Fonts { title, body }
    }

    fn chain(&self, title: bool) -> &[fontdue::Font] {
        if title && !self.title.is_empty() {
            &self.title
        } else {
            &self.body
        }
    }
}

fn pick(chain: &[fontdue::Font], c: char) -> Option<&fontdue::Font> {
    chain.iter().find(|f| f.lookup_glyph_index(c) != 0).or(chain.first())
}

fn advance(chain: &[fontdue::Font], c: char, px: f32) -> f32 {
    pick(chain, c).map_or(px * 0.5, |f| f.metrics(c, px).advance_width)
}

/// Coupe un texte en lignes de `width` pixels au plus (aux espaces).
fn wrap(chain: &[fontdue::Font], text: &str, px: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut w = 0.0;
    for word in text.split(' ') {
        let ww: f32 = word.chars().map(|c| advance(chain, c, px)).sum();
        let sp = advance(chain, ' ', px);
        if !cur.is_empty() && w + sp + ww > width {
            lines.push(std::mem::take(&mut cur));
            w = 0.0;
        }
        if !cur.is_empty() {
            cur.push(' ');
            w += sp;
        }
        cur.push_str(word);
        w += ww;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn draw_text(cv: &mut Canvas, chain: &[fontdue::Font], text: &str, x: f32, baseline: f32, px: f32, c: Rgb) {
    let mut pen = x;
    for ch in text.chars() {
        let Some(f) = pick(chain, ch) else { continue };
        let (m, bmp) = f.rasterize(ch, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for yy in 0..m.height {
            for xx in 0..m.width {
                let a = bmp[yy * m.width + xx] as f32 / 255.0;
                cv.blend(gx + xx as i32, gy + yy as i32, c, a);
            }
        }
        pen += m.advance_width;
    }
}

/// Dessine le panneau, cadre de la bannière : champ noir presque opaque, filet d'or doublé d'un
/// trait noir, médaillon du logo devant le titre, texte coupé à la largeur.
pub fn render(fonts: &Fonts, rows: &[Row], width: usize, px: f32) -> Canvas {
    let pad = (px * 0.9).round();
    let medal = if rows.first().is_some_and(|r| r.title) { ((px * 2.1).round() as usize).clamp(16, 64) } else { 0 };
    let indent = if medal > 0 { medal as f32 + px * 0.6 } else { 0.0 };
    let inner = width as f32 - 2.0 * pad;
    let mut laid: Vec<(String, Rgb, bool, f32)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let size = if r.title { px * 1.3 } else if i == rows.len() - 1 { px * 0.85 } else { px };
        let room = if r.title && i == 0 { inner - indent } else { inner };
        for l in wrap(fonts.chain(r.title), &r.text, size, room) {
            laid.push((l, r.color, r.title, size));
        }
    }
    let line_h = |size: f32| size * 1.35;
    // Bloc du titre : au moins la hauteur du médaillon.
    let title_lines = if rows.first().is_some_and(|r| r.title) { laid.iter().take_while(|l| l.2).count() } else { 0 };
    let title_h: f32 = laid[..title_lines].iter().map(|l| line_h(l.3)).sum();
    let title_extra = (medal as f32 - title_h).max(0.0);
    let height = (2.0 * pad + laid.iter().map(|(_, _, _, s)| line_h(*s)).sum::<f32>() + title_extra + px * 0.4).ceil() as usize;
    let mut cv = Canvas::new(width, height);
    // Fond aux coins à peine arrondis, filet d'or puis trait noir à l'intérieur.
    let rad = (px * 0.25).max(3.0);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (rad - fx).max(fx - (width as f32 - rad)).max(0.0);
            let dy = (rad - fy).max(fy - (height as f32 - rad)).max(0.0);
            let d = (dx * dx + dy * dy).sqrt();
            if d > rad {
                continue;
            }
            let edge = fx.min(fy).min(width as f32 - fx).min(height as f32 - fy).min(rad - d + 0.5);
            if edge < 1.6 {
                cv.blend(x as i32, y as i32, GOLD, 0.9);
            } else if edge < 2.6 {
                cv.blend(x as i32, y as i32, [0, 0, 0], 0.85);
            } else {
                cv.blend(x as i32, y as i32, FIELD, 0.93);
            }
        }
    }
    let mut y = pad;
    if medal > 0 {
        let logo = crate::brand::logo(medal);
        let top = (pad + (title_h.max(medal as f32) - medal as f32) / 2.0).round() as i32;
        for my in 0..medal {
            for mx in 0..medal {
                let p = &logo.px[(my * medal + mx) * 4..][..4];
                cv.blend(pad as i32 + mx as i32, top + my as i32, [p[0], p[1], p[2]], p[3] as f32 / 255.0);
            }
        }
        // Titre centré verticalement sur le médaillon.
        y += title_extra / 2.0;
    }
    for (i, (text, color, title, size)) in laid.iter().enumerate() {
        let baseline = y + size * 1.0;
        let x = if i < title_lines { pad + indent } else { pad };
        draw_text(&mut cv, fonts.chain(*title), text, x, baseline, *size, *color);
        y += line_h(*size);
        if i + 1 == title_lines {
            y += title_extra / 2.0 + px * 0.25;
        }
    }
    cv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_panel() -> Panel {
        let mut p = Panel::start("Test en jeu : TranslateGemma 4B");
        p.apply(Status::Phase("Traductions : 12/48".into()));
        p.apply(Status::Translation("fr→ru 0,75 s : Танк и лекарь для «Сердца магмы»".into()));
        p.apply(Status::Note("à-coup de 140 ms (seuil 100 ms) : priorité minimale".into()));
        p
    }

    #[test]
    fn statuses_fill_the_panel() {
        let mut p = done_panel();
        let rows = p.rows();
        assert_eq!(rows[0].text, "Test en jeu : TranslateGemma 4B");
        assert!(rows.iter().any(|r| r.text.starts_with("fr→ru") && r.color == DIM));
        assert!(rows.iter().any(|r| r.color == WARN && r.text.contains("à-coup")));
        p.apply(Status::Done {
            lines: vec![
                "Résultat translategemma : 48 traductions".into(),
                "Pilote graphique : aucune réinitialisation pendant le test".into(),
                "Verdict en jeu : compatible".into(),
                "Résultat noté pour ce PC (essais.tsv)".into(),
            ],
            verdict: Verdict::Ok,
            recorded: true,
            aborted: false,
        });
        let rows = p.rows();
        assert!(rows.iter().any(|r| r.text == "Verdict en jeu : compatible" && r.color == OK));
        assert!(rows.iter().any(|r| r.text.starts_with("Pilote graphique") && r.color == OK));
        assert!(!rows.iter().any(|r| r.text.starts_with("Résultat noté")));
        assert!(rows.last().unwrap().text.starts_with("Noté pour ce PC."));
        let mut q = Panel::start("Test en jeu : SalamandraTA 7B v3");
        q.fail("modèle introuvable");
        assert!(q.rows().iter().any(|r| r.color == BAD && r.text == "modèle introuvable"));
    }

    #[test]
    fn panel_is_drawn() {
        let fonts = Fonts::load();
        if fonts.body.is_empty() {
            return; // pas de police système ici
        }
        let cv = render(&fonts, &done_panel().rows(), 640, 20.0);
        assert_eq!(cv.w, 640);
        assert!(cv.h > 150 && cv.h < 400, "{}", cv.h);
        // Coin transparent, fond opaque au milieu, du texte clair quelque part.
        assert_eq!(cv.px[0] >> 24, 0);
        assert!(cv.px[cv.w * (cv.h / 2) + cv.w / 2] >> 24 > 200);
        let light = cv.px.iter().filter(|p| ((**p >> 16) & 0xFF) > 200 && ((**p >> 8) & 0xFF) > 200).count();
        assert!(light > 200, "{light}");
        if let Ok(dir) = std::env::var("CT_PANNEAU_COPIE") {
            std::fs::write(PathBuf::from(dir).join("panneau.bmp"), cv.to_bmp([60, 70, 60])).unwrap();
        }
    }
}
