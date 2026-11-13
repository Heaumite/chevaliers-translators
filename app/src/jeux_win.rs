//! Traduction en jeu, une fenêtre de WoW ou plusieurs à la fois (0.15).
//!
//! Chaque fenêtre de WoW ouverte (multi-comptes, ou Retail et Classic côte à côte) a sa propre
//! session : capture de son bandeau, chat miroir, bulles et panneaux posés sur elle, envois
//! traduits. Toutes partagent un seul moteur de traduction (`hub.rs`) : un seul modèle en mémoire
//! vidéo, une seule file de demandes, chaque traduction revenant au jeu qui l'a demandée.
//!
//! Économies (0.15) :
//!   - capture rapide (toutes les `poll_ms`) seulement quand le bandeau change ; immobile depuis
//!     1,5 s : toutes les 40 ms (le bandeau garde chaque trame au moins 50 ms) ;
//!   - zone de capture resserrée sur le bandeau dès qu'il est trouvé (environ 4 fois moins de
//!     pixels lus), élargie s'il est perdu.
//!
//! Bandeau caché (0.15) : une image faite des pixels juste au-dessus du bandeau est posée sur lui,
//! vue par le joueur mais exclue des captures d'écran (Windows 10 2004 et suivants) ; la capture
//! lit toujours le bandeau. Si la capture ne le lit plus avec le cache (Windows qui ne le permet
//! pas), le cache est abandonné pour la session.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;

use crate::bandeau::{clip_cover, cover_canvas, cover_strip, CoverGuard, CoverStrip, CoverVerdict};
use crate::capture_win::{self, Capturer};
use crate::decode::Decoded;
use crate::hub::Hub;
use crate::live::{FontSource, Live, ALIVE_TIMEOUT};
use crate::mirror;
use crate::overlay_win::Overlay;
use crate::proto::DecodeError;

/// Un jeu à traduire.
#[derive(Clone, Debug)]
pub struct GameSpec {
    /// Fenêtre du jeu (0 : la fenêtre de WoW trouvée, suivie ; outils de développement).
    pub hwnd: isize,
    /// Dossier AddOns de sa version (présence de l'application).
    pub addons: Option<PathBuf>,
    pub fonts: FontSource,
    /// Langue du client de ce jeu (cible des traductions reçues).
    pub client: &'static str,
    /// « Retail », « Classic »... pour le journal.
    pub label: String,
}

/// Capture lente quand le bandeau est immobile (ms).
const IDLE_POLL_MS: u64 = 40;
/// Bandeau immobile depuis ce temps : capture lente.
const FAST_FOR: Duration = Duration::from_millis(1500);
/// Image du cache du bandeau renouvelée au plus à cette cadence.
const COVER_EVERY: Duration = Duration::from_millis(100);
/// Plus d'image du cache depuis ce temps (bandeau caché par l'addon, écran de chargement) : le
/// cache s'en va. 0.16.3 : 1,2 s (avant 0,4 s, à peine plus que l'écart entre deux images : une
/// trame illisible suffisait à le retirer un instant).
const COVER_STALE: Duration = Duration::from_millis(1200);
/// Image du cache renvoyée au moins à cette cadence (signe que le bandeau est là).
const COVER_RESEND: Duration = Duration::from_millis(200);

enum Cap {
    Dims(i32, i32, usize, usize),
    Frame(Result<Decoded, DecodeError>),
    Cover(CoverStrip),
    /// Ligne pour le journal (préfixée du numéro du jeu).
    Note(String),
    Fail(String),
}

/// Capture rapide d'une fenêtre cachée par une autre (image demandée à Windows, plus coûteuse) :
/// le bandeau garde chaque trame au moins 50 ms.
const HIDDEN_POLL_MS: u64 = 25;

fn spawn_capture(game: usize, hwnd: isize, width: usize, height: usize, poll_ms: u64, tx: Sender<(usize, Cap)>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        // Une erreur interne du fil de capture ne le laisse plus mourir en silence (0.15.1 : le
        // jeu restait sans traduction) : elle remonte comme un échec, et la capture est relancée.
        let tx2 = tx.clone();
        let run = std::panic::AssertUnwindSafe(move || capture_loop(game, hwnd, width, height, poll_ms, tx, stop));
        if let Err(p) = std::panic::catch_unwind(run) {
            let why = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "erreur inconnue".into());
            let _ = tx2.send((game, Cap::Fail(format!("erreur interne de la capture : {why}"))));
        }
    });
}

fn capture_loop(game: usize, hwnd: isize, width: usize, height: usize, poll_ms: u64, tx: Sender<(usize, Cap)>, stop: Arc<AtomicBool>) {
    {
        let target = hwnd as HWND;
        let mut cap = match Capturer::new_for(width, height, target) {
            Ok(c) => {
                let _ = tx.send((game, Cap::Dims(c.screen_w, c.screen_h, c.w, c.h)));
                c
            }
            Err(e) => {
                let _ = tx.send((game, Cap::Fail(e)));
                return;
            }
        };
        // Seuls les changements partent : une même trame reste plusieurs captures à l'écran.
        let mut last: Option<Result<(u8, u16, usize), DecodeError>> = None;
        let mut resent = Instant::now();
        let mut changed = Instant::now();
        let mut ok_at: Option<Instant> = None;
        let mut cover_at = Instant::now() - Duration::from_secs(1);
        let mut last_cover: Vec<[u8; 3]> = Vec::new();
        let mut cover_sent = Instant::now() - Duration::from_secs(1);
        // Fenêtre cachée par une autre (plusieurs WoW, 0.15.1) : bandeau lu dans la fenêtre même.
        let mut hidden_since: Option<Instant> = None;
        let (mut said_ok, mut said_ko) = (false, false);
        while !stop.load(Ordering::Relaxed) {
            let fast = changed.elapsed() < FAST_FOR;
            let wait = match (fast, cap.via_window) {
                (true, true) => HIDDEN_POLL_MS.max(poll_ms),
                (true, false) => poll_ms.max(1),
                (false, _) => IDLE_POLL_MS.max(poll_ms),
            };
            std::thread::sleep(Duration::from_millis(wait));
            if hwnd != 0 && !cap.target_alive() {
                let _ = tx.send((game, Cap::Fail("fenêtre de WoW fermée".into())));
                return;
            }
            let now = Instant::now();
            let want_cover = now.saturating_duration_since(cover_at) >= COVER_EVERY;
            let (r, strip) = match cap.grab() {
                Ok(img) => {
                    let r = crate::decode::decode(&img);
                    let strip = match (&r, want_cover) {
                        (Ok(d), true) => cover_strip(&img, &d.geometry),
                        _ => None,
                    };
                    (r, strip)
                }
                Err(e) => {
                    let _ = tx.send((game, Cap::Fail(e)));
                    return;
                }
            };
            if cap.via_window {
                let since = *hidden_since.get_or_insert(now);
                if r.is_ok() && !said_ok {
                    said_ok = true;
                    let _ = tx.send((game, Cap::Note("fenêtre cachée par une autre : son bandeau est lu dans la fenêtre elle-même, la traduction continue".into())));
                } else if r.is_err() && !said_ok && !said_ko && now.saturating_duration_since(since) > Duration::from_secs(3) {
                    said_ko = true;
                    let _ = tx.send((game, Cap::Note("fenêtre cachée par une autre : Windows ne donne pas l'image de son bandeau ; sa traduction reprend dès qu'elle est visible".into())));
                }
            } else {
                hidden_since = None;
            }
            match &r {
                Ok(d) => {
                    ok_at = Some(now);
                    cap.narrow_to(&d.geometry);
                }
                Err(_) => {
                    // Bandeau perdu plus d'une seconde : zone large (il a pu changer de taille).
                    if cap.narrowed() && ok_at.is_none_or(|t| now.saturating_duration_since(t) > Duration::from_secs(1)) {
                        cap.widen();
                    }
                }
            }
            if let Some(s) = strip {
                cover_at = now;
                // Renvoyée si elle change, et toutes les 200 ms (signe que le bandeau est là).
                if s.row != last_cover || now.saturating_duration_since(cover_sent) >= COVER_RESEND {
                    last_cover = s.row.clone();
                    cover_sent = now;
                    if tx.send((game, Cap::Cover(s))).is_err() {
                        return;
                    }
                }
            }
            let sig = r.as_ref().map(|d| (d.frame.ftype, d.frame.seq, d.frame.payload.len())).map_err(|e| *e);
            // Bandeau immobile (addon 0.13.3) : la même trame lisible est renvoyée toutes les
            // 300 ms, signe que le jeu est toujours là.
            let still = sig.is_ok() && last.as_ref() == Some(&sig) && resent.elapsed() >= Duration::from_millis(300);
            if last.as_ref() != Some(&sig) || still {
                if last.as_ref() != Some(&sig) {
                    changed = now;
                }
                last = Some(sig);
                resent = now;
                if tx.send((game, Cap::Frame(r))).is_err() {
                    return;
                }
            }
        }
    }
}

/// Panneaux posés sur le jeu.
const HINT: u8 = 251;
const DIGEST: u8 = 250;
const PROPOSAL: u8 = 249;
const DOWNLOADS: u8 = 248;
const COVER: u8 = 252;

struct Session {
    /// Numéro du jeu (1, 2, 3... : le plus petit libre ; le journal et la file de traduction).
    game: usize,
    /// Numéro de sa capture (jamais réutilisé : une capture arrêtée ne parle pas pour la suivante).
    token: usize,
    spec: GameSpec,
    stop: Arc<AtomicBool>,
    live: Live,
    overlay: Overlay,
    started: Instant,
    ready: bool,
    proposal_until: Option<Instant>,
    downloads_until: Option<Instant>,
    downloads_said: bool,
    game_since: Option<Instant>,
    screen_info: Option<String>,
    screen_checked: Instant,
    digest_until: Option<Instant>,
    hint_until: Option<Instant>,
    hint_last: Option<Instant>,
    last_raise: Instant,
    last_tick: Instant,
    overlay_error: bool,
    restore: Option<(Instant, Option<String>, String)>,
    wow_front: bool,
    combat_seen: bool,
    /// Autre fenêtre devant ce jeu (pixels du bureau), relevée toutes les 100 ms.
    over: Option<(i32, i32, i32, i32)>,
    cover: CoverState,
}

#[derive(Default)]
struct CoverState {
    /// Dernière image reçue, et moment.
    strip: Option<(CoverStrip, Instant)>,
    shown_at: Option<Instant>,
    /// Dernière pose (gauche, haut, image reçue à, fenêtre devant) : rien n'est renvoyé à Windows
    /// si rien n'a changé.
    posted: Option<(i32, i32, Instant, Option<(i32, i32, i32, i32)>)>,
    /// Retraits juste après la pose, pauses (0.16.3 : plus d'abandon sur un changement de fenêtre).
    guard: CoverGuard,
    off: bool,
    said: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        crate::engine::forget_game(self.game);
    }
}

impl Session {
    fn window(&self) -> HWND {
        if self.spec.hwnd == 0 {
            capture_win::wow_window()
        } else {
            self.spec.hwnd as HWND
        }
    }

    fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        capture_win::client_rect(self.window())
    }

    fn in_front(&self) -> bool {
        if self.spec.hwnd == 0 {
            Overlay::wow_in_front()
        } else {
            Overlay::in_front_of(self.spec.hwnd as HWND)
        }
    }

    fn combat(&self) -> bool {
        self.live.ctx & (crate::engine::CTX_COMBAT | crate::engine::CTX_LOCKED) != 0
    }

    /// Cache du bandeau : posé tant que le bandeau est lu, retiré sinon. Une autre fenêtre en
    /// partie devant lui (changement de fenêtre, 0.16.3) : le cache s'arrête à son bord au lieu de
    /// disparaître entièrement.
    fn update_cover(&mut self, now: Instant) {
        if self.cover.off {
            return;
        }
        if !self.cover.guard.allowed(now) {
            if self.cover.shown_at.take().is_some() {
                self.overlay.hide(COVER);
            }
            self.cover.posted = None;
            return;
        }
        let fresh = self.cover.strip.as_ref().is_some_and(|(_, t)| now.saturating_duration_since(*t) < COVER_STALE);
        let rect = self.rect();
        let place = match (&self.cover.strip, rect) {
            (Some((s, at)), Some((_, _, r, b))) if fresh => Some((r - s.from_right, b - s.from_bottom, s.clone(), *at)),
            _ => None,
        };
        // Une autre fenêtre passée devant le bandeau : rien n'est posé sur elle (cache découpé à
        // son bord, ou retiré s'il est entièrement dessous).
        let canvas = place.as_ref().map(|(l, t, s, _)| clip_cover(&cover_canvas(s), *l, *t, self.over));
        match (place, canvas) {
            (Some((l, t, _, at)), Some(Some(cv))) => {
                let key = (l, t, at, self.over);
                if self.cover.shown_at.is_some() && self.cover.posted == Some(key) {
                    // Même image, même place, déjà posée : rien à refaire.
                } else {
                    self.cover.posted = Some(key);
                    match self.overlay.show_cover(COVER, l, t, &cv) {
                        Ok(()) => {
                            self.cover.shown_at.get_or_insert(now);
                        }
                        Err(e) => {
                            crate::say!("[bandeau] cache impossible : {e} ; le bandeau reste visible");
                            self.cover.off = true;
                            self.overlay.hide(COVER);
                        }
                    }
                }
            }
            _ => {
                self.cover.posted = None;
                if let Some(at) = self.cover.shown_at.take() {
                    self.overlay.hide(COVER);
                    // Retiré faute de bandeau lu juste après sa pose, jeu devant et rien sur lui :
                    // la capture ne le traverse peut-être pas. La garde décide (pause, abandon).
                    if !fresh {
                        let steady = self.in_front() && self.over.is_none();
                        match self.cover.guard.on_lost(now, at, steady) {
                            CoverVerdict::Keep => {}
                            CoverVerdict::Pause(d) => crate::say!(
                                "[bandeau] bandeau illisible plusieurs fois juste après la pose du cache : cache retiré {} s, puis nouvel essai",
                                d.as_secs()
                            ),
                            CoverVerdict::Off => {
                                self.cover.off = true;
                                crate::say!("[bandeau] la capture ne lit jamais le bandeau sous son cache : bandeau laissé visible pour cette session");
                            }
                        }
                    }
                }
            }
        }
        if !self.cover.said && self.cover.shown_at.is_some() {
            self.cover.said = true;
            crate::say!("[bandeau] caché aux yeux du joueur (exclu de l'affichage capturé, toujours lu par l'application)");
        }
    }

    /// Une passe de la boucle pour ce jeu (frames décodées déjà données à `live`).
    fn step(&mut self, now: Instant, fonts: &mut Option<crate::panel::Fonts>, primary: bool) {
        // Envoi traduit : collage puis Entrée, seulement si ce WoW est devant (boîte de saisie prête).
        for (id, text) in self.live.sends_due(now, crate::keys_win::shift_held()) {
            if !self.in_front() {
                crate::say!("[S3] envoi {id} : WoW n'est plus au premier plan, rien n'est collé");
                continue;
            }
            let saved = crate::clipboard::get();
            match crate::clipboard::set(&text).and_then(|_| crate::keys_win::paste_and_enter()) {
                Ok(()) => {
                    crate::say!("[S3] envoi {id} : collé et envoyé : {text}");
                    self.restore = Some((Instant::now() + Duration::from_millis(700), saved, text));
                }
                Err(e) => crate::say!("[S3] envoi {id} : collage impossible ({e}) ; la traduction est dans le presse-papiers (Ctrl+V)"),
            }
        }
        if let Some((at, saved, pasted)) = &self.restore {
            if now >= *at {
                // Remis seulement si le presse-papiers contient encore notre texte.
                if crate::clipboard::get().as_deref() == Some(pasted.as_str()) {
                    if let Some(s) = saved {
                        let _ = crate::clipboard::set(s);
                    }
                }
                self.restore = None;
            }
        }
        // Clic sur une icône de copie.
        for (frame, x, y) in self.overlay.take_clicks() {
            if let Some(text) = self.live.copy_at(frame, x, y, now) {
                match crate::clipboard::set(&text) {
                    Ok(()) => crate::say!("[copie] {text}"),
                    Err(e) => crate::say!("[copie] impossible : {e}"),
                }
            }
        }
        if now.duration_since(self.last_tick) < Duration::from_millis(33) {
            return;
        }
        self.last_tick = now;
        // Caché dès que l'addon ne donne plus signe de vie (déconnexion, écran de chargement,
        // choix du personnage) ou que WoW est fermé ou réduit ; revient avec lui. Une autre
        // application active ne le cache pas : le chat reste sur WoW, sauf sous sa fenêtre.
        let in_front = self.in_front();
        let alive = self.live.last_alive.is_none_or(|t| now.saturating_duration_since(t) < ALIVE_TIMEOUT);
        let front = if in_front {
            alive
        } else if self.rect().is_none() {
            false
        } else {
            self.wow_front
        };
        // Combat (0.14) : rien n'est dessiné sur le jeu ; la traduction continue.
        let combat = self.combat();
        if front != self.wow_front {
            self.wow_front = front;
            if front && !combat {
                self.live.mirror_redraw();
            } else if !front {
                self.overlay.hide_all();
                self.cover.posted = None;
            }
        }
        if combat != self.combat_seen {
            self.combat_seen = combat;
            if combat {
                self.overlay.hide_all();
                self.cover.posted = None;
                self.hint_until = None;
                self.downloads_until = None;
            } else if self.wow_front {
                self.live.mirror_redraw();
            }
        }
        // Un clic dans WoW peut le passer devant le miroir : on remet le miroir devant. Une autre
        // application (ou un autre WoW) devant celui-ci : rien n'est dessiné sous sa fenêtre.
        if now.saturating_duration_since(self.last_raise) >= Duration::from_millis(100) {
            self.last_raise = now;
            self.overlay.set_wow_rect(self.rect());
            // Écran du jeu (et ses changements) au journal : télévision, autre écran, échelle.
            if now.saturating_duration_since(self.screen_checked) >= Duration::from_secs(5) {
                self.screen_checked = now;
                if let Some((info, tv)) = capture_win::screen_info_of(self.window()) {
                    if self.screen_info.as_deref() != Some(info.as_str()) {
                        crate::say!("[écran] {info}");
                        self.screen_info = Some(info);
                    }
                    // Télévision : dispositif télévision à la prochaine connexion ou /reload. Une
                    // seule présence pour tous les jeux : celle du premier.
                    if primary && tv != crate::install::screen_tv() {
                        crate::install::set_screen_tv(tv);
                        crate::say!(
                            "[écran] {} : dispositif télévision {} à la prochaine connexion ou /reload",
                            if tv { "WoW passé sur une télévision" } else { "WoW passé sur un écran de PC" },
                            if tv { "actif" } else { "inactif" }
                        );
                    }
                }
            }
            if in_front {
                self.over = None;
                self.overlay.raise_all();
                self.live.window_over(None, self.overlay.origin());
            } else {
                self.over = Overlay::window_over(self.window());
                self.live.window_over(self.over, self.overlay.origin());
            }
        }
        self.update_cover(now);
        // L'addon attend l'application (lancée après la connexion) : « /reload » sur le jeu,
        // 20 s, au plus toutes les 3 min.
        let waiting = self.live.waiting_at.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(25))
            && self.live.last_alive.is_none_or(|t| self.live.waiting_at.is_some_and(|w| w > t));
        if waiting && in_front && !combat && self.hint_until.is_none() && self.hint_last.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(180)) {
            if let Some((l, top, r, _)) = self.rect() {
                let f = fonts.get_or_insert_with(crate::panel::Fonts::load);
                let px = ((r - l) as f32 / 120.0).clamp(14.0, 32.0);
                let rows = vec![
                    crate::panel::Row { text: "Chevaliers Translators".into(), color: crate::panel::TITLE, title: true },
                    crate::panel::Row { text: "L'application est lancée : tape /reload pour traduire le chat.".into(), color: crate::panel::TEXT, title: false },
                    crate::panel::Row { text: "La prochaine fois, lance Chevaliers Translators avant WoW.".into(), color: crate::panel::DIM, title: false },
                ];
                let cv = crate::panel::render(f, &rows, ((r - l) as f32 * 0.32) as usize, px);
                let left = l + ((r - l) - cv.w as i32) / 2;
                if self.overlay.show_screen(HINT, left, top + (px * 4.0) as i32, &cv).is_ok() {
                    crate::say!("[présence] l'addon attend l'application : « /reload » affiché sur le jeu");
                    self.hint_until = Some(now + Duration::from_secs(20));
                    self.hint_last = Some(now);
                }
            }
        }
        if self.hint_until.is_some_and(|t| now >= t || !waiting) {
            self.overlay.hide(HINT);
            self.hint_until = None;
        }
        // Bilan du combat (0.13.3) : les messages reçus pendant le verrouillage du chat, traduits
        // après, d'un bloc, une fois le combat fini (2 s sans nouvel ajout).
        let calm = !combat;
        if self.digest_until.is_none()
            && in_front
            && calm
            && !self.live.digest.is_empty()
            && self.live.digest_at.is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(2))
        {
            if let Some((l, top, r, b)) = self.rect() {
                let f = fonts.get_or_insert_with(crate::panel::Fonts::load);
                let px = ((r - l) as f32 / 140.0).clamp(13.0, 28.0);
                let items: Vec<(String, String)> = self.live.digest.drain(..).collect();
                let cv = crate::panel::render(f, &crate::live::digest_rows(&items), ((r - l) as f32 * 0.34) as usize, px);
                // En haut à gauche de l'écran du jeu, sous le cadre du personnage.
                let left = l + (px * 2.0) as i32;
                let y = (top + (b - top) / 5).min(b - cv.h as i32);
                if self.overlay.show_screen(DIGEST, left, y, &cv).is_ok() {
                    crate::say!("[combat] bilan affiché : {} message(s) traduit(s) après le combat", items.len());
                    self.digest_until = Some(now + Duration::from_secs((12 + 3 * items.len() as u64).min(40)));
                }
            }
        }
        if self.digest_until.is_some_and(|t| now >= t || !calm) {
            self.overlay.hide(DIGEST);
            self.digest_until = None;
        }
        // Proposition d'un modèle supérieur (0.14) : hors combat, une minute après l'arrivée en
        // jeu, 40 s, une fois par session ; cachée dès la réponse du joueur.
        let settled = alive && self.live.last_alive.is_some() && self.game_since.is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(60));
        if alive && self.live.last_alive.is_some() {
            self.game_since.get_or_insert(now);
        } else {
            self.game_since = None;
        }
        if self.proposal_until.is_none() && in_front && calm && settled && !self.live.proposal_shown && !self.live.proposal_answered {
            if let (Some(p), Some((l, top, r, _))) = (self.live.proposal.clone(), self.rect()) {
                let f = fonts.get_or_insert_with(crate::panel::Fonts::load);
                let px = ((r - l) as f32 / 130.0).clamp(13.0, 28.0);
                let cv = crate::panel::render(f, &crate::conseil::panel_rows(&p), ((r - l) as f32 * 0.36) as usize, px);
                let left = l + ((r - l) - cv.w as i32) / 2;
                if self.overlay.show_screen(PROPOSAL, left, top + (px * 4.0) as i32, &cv).is_ok() {
                    crate::say!("[proposition] panneau affiché : {} {}", p.model.name, p.place.label());
                    self.live.proposal_shown = true;
                    self.proposal_until = Some(now + Duration::from_secs(40));
                }
            }
        }
        if self.proposal_until.is_some_and(|t| now >= t || !calm || self.live.proposal_answered) {
            self.overlay.hide(PROPOSAL);
            self.proposal_until = None;
        }
        // Téléchargement des modèles en cours (0.14) : dit une fois par session, en arrivant en
        // jeu, 15 s.
        if self.downloads_until.is_none() && !self.downloads_said && in_front && calm && settled {
            let st = crate::telechargements::status();
            self.downloads_said = true;
            if st.active {
                if let Some((l, top, r, _)) = self.rect() {
                    let f = fonts.get_or_insert_with(crate::panel::Fonts::load);
                    let px = ((r - l) as f32 / 130.0).clamp(13.0, 28.0);
                    let rows = vec![
                        crate::panel::Row { text: "Chevaliers Translators".into(), color: crate::panel::TITLE, title: true },
                        crate::panel::Row { text: format!("Modèles de traduction en cours de téléchargement : {}.", st.line()), color: crate::panel::TEXT, title: false },
                        crate::panel::Row {
                            text: "La traduction utilise le modèle déjà là ; le meilleur pour ton PC sera activé à la fin, hors combat.".into(),
                            color: crate::panel::DIM,
                            title: false,
                        },
                    ];
                    let cv = crate::panel::render(f, &rows, ((r - l) as f32 * 0.34) as usize, px);
                    let left = l + ((r - l) - cv.w as i32) / 2;
                    if self.overlay.show_screen(DOWNLOADS, left, top + (px * 4.0) as i32, &cv).is_ok() {
                        self.downloads_until = Some(now + Duration::from_secs(15));
                    }
                }
            }
        }
        if self.downloads_until.is_some_and(|t| now >= t || combat) {
            self.overlay.hide(DOWNLOADS);
            self.downloads_until = None;
        }
        let updates = self.live.mirror_tick(now);
        if !self.wow_front || combat {
            return;
        }
        for u in updates {
            let r = match &u.canvas {
                Some(cv) => self.overlay.show(u.frame, u.left, u.bottom, cv, &u.hits),
                None => {
                    self.overlay.hide(u.frame);
                    Ok(())
                }
            };
            if let Err(e) = r {
                if !self.overlay_error {
                    crate::say!("[miroir] affichage impossible : {e}");
                    self.overlay_error = true;
                }
            }
        }
    }
}

/// Préfixe du journal pendant le travail d'un jeu, quand il y en a plusieurs.
fn tag(s: &Session, many: bool) -> String {
    if many {
        format!("[WoW {}{}] ", s.game, if s.spec.label.is_empty() { String::new() } else { format!(" {}", s.spec.label) })
    } else {
        String::new()
    }
}

/// Boucle de traduction : jeux donnés par `games` (relu chaque seconde : un WoW ouvert ou fermé
/// en cours de route est pris ou laissé), jusqu'à ce que `keep` renvoie faux. `strict` : une
/// capture qui échoue arrête tout (un seul jeu, outils) ; sinon le jeu est repris 5 s plus tard.
#[allow(clippy::too_many_arguments)]
pub fn run_games(
    width: usize,
    height: usize,
    poll_ms: u64,
    demo: bool,
    opts: mirror::RenderOpts,
    engine: Option<(crate::engine::Config, bool)>,
    games: &mut dyn FnMut() -> Vec<GameSpec>,
    keep: &mut dyn FnMut() -> bool,
    strict: bool,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Cap)>();
    let own_too = engine.as_ref().is_none_or(|e| e.1);
    let hub = engine.map(|(cfg, _)| {
        crate::say!(
            "Traduction : un seul modèle pour tous les jeux ouverts ; chargement en cours. Journal du serveur : {}",
            cfg.log.display()
        );
        if !own_too {
            crate::say!("Tes propres messages ne sont pas traduits (--sans-mes-messages).");
        }
        Hub::new(Box::new(crate::engine::Engine::start(cfg)))
    });
    if hub.is_none() {
        crate::say!("Traduction : simulée ([→XX] devant le texte, --sans-traduction ou --demo-trad)");
    }
    let hub: Option<Rc<RefCell<Hub>>> = hub;
    let mut sessions: Vec<Session> = Vec::new();
    let mut next_token = 1usize;
    let mut focus_seen = 0usize;
    let mut retry: HashMap<isize, Instant> = HashMap::new();
    let mut looked = Instant::now() - Duration::from_secs(10);
    let mut fonts: Option<crate::panel::Fonts> = None;
    let mut last_presence: Option<Instant> = None;
    let mut presence_error = false;
    crate::say!("En jeu : la traduction démarre seule à la connexion (WoW déjà ouvert : /reload). Maj+Entrée : envoi traduit.");
    loop {
        let now = Instant::now();
        // Jeux ouverts ou fermés (chaque seconde).
        if now.saturating_duration_since(looked) >= Duration::from_secs(1) {
            looked = now;
            let wanted = games();
            // Fermés : session finie (fenêtres du miroir retirées, capture arrêtée, traductions en
            // cours oubliées). Une fenêtre de WoW = une session de traduction liée à elle.
            let mut closed: Vec<usize> = Vec::new();
            sessions.retain(|s| {
                let keep = wanted.iter().any(|w| w.hwnd == s.spec.hwnd);
                if !keep {
                    closed.push(s.game);
                }
                keep
            });
            for g in &closed {
                crate::say!("[WoW {g}] fenêtre fermée : sa traduction s'arrête");
                if let Some(h) = &hub {
                    h.borrow_mut().leave(*g);
                }
            }
            if !closed.is_empty() {
                crate::say!("{} jeu(x) traduit(s)", sessions.len());
            }
            for spec in wanted {
                if sessions.iter().any(|s| s.spec.hwnd == spec.hwnd) || retry.get(&spec.hwnd).is_some_and(|t| now < *t) {
                    continue;
                }
                // Numéro du jeu : le plus petit libre (trois WoW ouverts : 1, 2 et 3).
                let game = (1..).find(|g| !sessions.iter().any(|s| s.game == *g)).unwrap_or(1);
                let token = next_token;
                next_token += 1;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_capture(token, spec.hwnd, width, height, poll_ms, tx.clone(), stop.clone());
                let mut live = Live::new(spec.addons.clone());
                live.game = game;
                live.enable_mirror(spec.fonts.clone(), demo, opts);
                if let Some(h) = &hub {
                    live.enable_translation(Hub::join(h, game), spec.client, own_too);
                }
                let overlay = match Overlay::new() {
                    Ok(o) => o,
                    Err(e) => {
                        stop.store(true, Ordering::Relaxed);
                        if strict {
                            return Err(e);
                        }
                        crate::say!("[WoW {game}] fenêtres du miroir impossibles : {e}");
                        retry.insert(spec.hwnd, now + Duration::from_secs(5));
                        continue;
                    }
                };
                crate::say!(
                    "[WoW {game}] {} : traduction vers {}",
                    if spec.label.is_empty() { "jeu" } else { spec.label.as_str() },
                    spec.client.to_uppercase()
                );
                sessions.push(Session {
                    game,
                    token,
                    spec,
                    stop,
                    live,
                    overlay,
                    started: now,
                    ready: false,
                    proposal_until: None,
                    downloads_until: None,
                    downloads_said: false,
                    game_since: None,
                    screen_info: None,
                    screen_checked: now - Duration::from_secs(10),
                    digest_until: None,
                    hint_until: None,
                    hint_last: None,
                    last_raise: now,
                    last_tick: now,
                    overlay_error: false,
                    restore: None,
                    wow_front: true,
                    combat_seen: false,
                    over: None,
                    cover: CoverState::default(),
                });
            }
        }
        // Trames des captures (attente courte : fenêtres et collages restent vifs).
        let mut got: Vec<(usize, Cap)> = Vec::new();
        let wait = if sessions.is_empty() { 100 } else { poll_ms.max(4) };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(m) => got.push(m),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("fils de capture arrêtés".into()),
        }
        got.extend(rx.try_iter());
        let now = Instant::now();
        let many = sessions.len() > 1;
        let mut failed: Vec<(usize, String)> = Vec::new();
        let mut fresh: Vec<usize> = Vec::new();
        for (token, msg) in got {
            let Some(s) = sessions.iter_mut().find(|s| s.token == token) else { continue };
            let game = s.game;
            crate::set_journal_prefix(&tag(s, many));
            match msg {
                Cap::Dims(sw, sh, cw, ch) => {
                    s.ready = true;
                    crate::say!("Décodeur en écoute : écran {sw}x{sh}, zone {cw}x{ch} px en bas à droite, capture toutes les {poll_ms} ms (40 ms au repos).");
                    match s.live.addons_dir() {
                        Some(d) => crate::say!("Présence de l'application écrite dans {} (ChevaliersTranslators_Live) : la traduction démarre seule à la connexion.", d.display()),
                        None => crate::say!("Sans --wow et --flavor : la traduction ne démarre pas seule (en jeu : /chevaliers on)."),
                    }
                }
                Cap::Frame(r) => {
                    s.live.feed_result(r, now);
                    fresh.push(game);
                }
                Cap::Cover(strip) => {
                    let proven = s.cover.guard.proven();
                    s.cover.guard.on_strip(now, s.cover.shown_at);
                    if !proven && s.cover.guard.proven() {
                        crate::say!("[bandeau] bandeau lu à travers son cache : le cache tient (changements de fenêtre compris)");
                    }
                    s.cover.strip = Some((strip, now));
                }
                Cap::Note(t) => crate::say!("{t}"),
                Cap::Fail(e) => failed.push((token, e)),
            }
        }
        crate::set_journal_prefix("");
        for (token, e) in failed {
            if let Some(i) = sessions.iter().position(|s| s.token == token) {
                if strict {
                    return Err(e);
                }
                let s = sessions.remove(i);
                let game = s.game;
                crate::say!("[WoW {game}] capture arrêtée : {e} ; nouvel essai dans 5 s");
                retry.insert(s.spec.hwnd, now + Duration::from_secs(5));
                if let Some(h) = &hub {
                    h.borrow_mut().leave(game);
                }
            }
        }
        // Une capture qui ne démarre pas devient une erreur (nouvel essai), jamais une attente
        // sans fin.
        if let Some(i) = sessions.iter().position(|s| !s.ready && now.saturating_duration_since(s.started) > Duration::from_secs(10)) {
            if strict {
                return Err("capture du bandeau bloquée au démarrage".into());
            }
            let s = sessions.remove(i);
            crate::say!("[WoW {}] capture du bandeau bloquée au démarrage ; nouvel essai dans 5 s", s.game);
            retry.insert(s.spec.hwnd, now + Duration::from_secs(5));
            if let Some(h) = &hub {
                h.borrow_mut().leave(s.game);
            }
        }
        for s in sessions.iter_mut() {
            if !fresh.contains(&s.game) {
                s.live.idle(now);
            }
        }
        // Messages des fenêtres de ce fil (miroirs, icône près de l'horloge), même sans jeu.
        Overlay::pump_thread();
        if !keep() {
            return Ok(());
        }
        // Présence de l'application, relue par l'addon à chaque connexion et /reload (une fois
        // par dossier, plusieurs jeux pouvant partager une version).
        if last_presence.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(2)) {
            last_presence = Some(now);
            let mut dirs: Vec<&PathBuf> = sessions.iter().filter_map(|s| s.live.addons_dir()).collect();
            dirs.sort();
            dirs.dedup();
            for d in dirs {
                if let Err(e) = crate::install::write_live_everywhere(d) {
                    if !presence_error {
                        crate::say!("[présence] {e}");
                        presence_error = true;
                    }
                }
            }
        }
        // Fenêtre de WoW active : ses traductions passent devant celles des autres jeux.
        let focus = sessions.iter().find(|s| s.in_front()).map_or(0, |s| s.game);
        crate::engine::set_focus(focus);
        if focus != focus_seen {
            focus_seen = focus;
            if focus != 0 && sessions.len() > 1 {
                crate::say!("[WoW {focus}] fenêtre active : ses traductions passent en premier");
            }
        }
        let many = sessions.len() > 1;
        for (k, s) in sessions.iter_mut().enumerate() {
            crate::set_journal_prefix(&tag(s, many));
            s.step(now, &mut fonts, k == 0);
        }
        crate::set_journal_prefix("");
    }
}
