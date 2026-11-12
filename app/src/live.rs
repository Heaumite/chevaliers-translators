//! Décodeur en direct : statistiques du test S1, affichage du chat relayé (S2), envoi traduit
//! (S3) et chat miroir (S5) : descriptions des fenêtres de chat -> images posées sur le jeu.
//!
//! Traduction réelle (0.9) : chaque message reçu, et depuis la 0.11 chaque message du joueur, est
//! rangé dans sa langue (`lang.rs`) ; s'il n'est pas dans la langue du client, il est traduit
//! (`engine.rs`) et le chat miroir l'affiche « [DE] traduction » à sa place.
//!
//! Envoi traduit (0.11) : Maj+Entrée envoie le texte du joueur ; l'addon rouvre la boîte de
//! saisie, vide, sur le même canal, et le signale ; la traduction y est alors collée puis envoyée
//! (Ctrl+V et Entrée simulés par `run`, avec l'accord du joueur, seulement à ce moment-là). Les
//! brouillons (texte en cours de frappe) sont traduits d'avance. Trad:AUTO : langue du
//! destinataire d'un chuchotement, sinon langue majoritaire des derniers messages du canal (si
//! c'est celle du client, rien à traduire), sinon l'anglais.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::decode::{Decoded, Image};
use crate::engine::{Ask, Event, Priority, Translator};
use crate::lang::{self, Detected, Speakers};
use crate::mirror::{self, Mirror, Update};
use crate::proto::{
    self, DecodeError, FRAME_CHAT, FRAME_MADV, FRAME_MLINE, FRAME_MVIEW, FRAME_NOTE, FRAME_OCCLUDE, FRAME_PERF,
    FRAME_PING, FRAME_RETARGET, FRAME_SEND, FRAME_STATE, FRAME_TEST, STATE_CANCEL, STATE_READY, STATE_SENT,
};

/// D'où viennent les polices du chat miroir.
pub enum FontSource {
    /// Archives du jeu (avec cache local) : `--wow` et `--flavor`.
    Game { wow: PathBuf, flavor: String },
    /// Un fichier imposé pour toutes les fenêtres (harnais de test).
    File(PathBuf),
    /// Polices de secours seulement.
    None,
}

#[derive(Default)]
struct TestStats {
    seen: Vec<bool>,
    unique: usize,
    max_seq: u16,
    mismatched: usize,
    payload_bytes: usize,
    first: Option<Instant>,
    last: Option<Instant>,
    geometry: Option<(usize, usize)>,
}

struct Assembly {
    parts: Vec<Option<Vec<u8>>>,
    chan: u8,
    started: Instant,
}

/// Traduction en cours.
enum Pending {
    /// Message reçu : son texte d'origine (repéré dans les lignes du miroir), son expéditeur et
    /// sa langue.
    Received { body: String, sender: String, src: &'static str, late: bool },
    /// Texte du joueur : brouillon pendant la frappe, ou envoi (Maj+Entrée). `sends` : envois qui
    /// attendent cette traduction ; `key` : cible, canal, destinataire et texte.
    Own { key: String, text: String, src: &'static str, tgt: &'static str, sends: Vec<u16> },
}

/// Envoi traduit en cours (Maj+Entrée).
struct SendJob {
    text: String,
    tgt: &'static str,
    /// Texte à coller : la traduction, ou le texte d'origine s'il n'y a rien à traduire.
    paste: Option<String>,
    /// Boîte de saisie rouverte, vide, prête pour le collage (trame de l'addon).
    ready: Option<Instant>,
    created: Instant,
    pasted: bool,
}

/// Envoi traduit collé : quand le message revient dans le chat, il s'affiche dans la langue du
/// joueur (son texte d'origine), précédé de l'acronyme de la langue d'envoi.
struct Sent {
    paste: String,
    source: String,
    tgt: &'static str,
    at: Instant,
}

/// Un envoi dont la traduction n'est pas collée dans ce délai est abandonné (l'addon remet le
/// texte d'origine dans la boîte un peu après).
pub const SEND_DEADLINE: Duration = Duration::from_secs(5);
/// Le joueur tient encore Maj après Maj+Entrée : on lui laisse ce temps pour la relâcher.
const SHIFT_GRACE: Duration = Duration::from_millis(600);
/// Les brouillons d'une même boîte se remplacent dans la file du traducteur.
const DRAFT_GROUP: u64 = 1;

#[derive(Default, Debug)]
pub struct TrStats {
    pub received: usize,
    pub foreign: usize,
    pub neutral: usize,
    pub translated: usize,
    pub cached: usize,
    pub failed: usize,
    pub sent: usize,
}

/// Traduction réelle branchée sur le chat.
pub struct Tr {
    engine: Box<dyn Translator>,
    /// Langue du client de WoW : celle du joueur.
    pub client: &'static str,
    speakers: Speakers,
    /// Langues vues récemment par canal (code de canal du protocole), pour Trad:AUTO.
    /// Langues vues par portée : canal, et zone pour les canaux géographiques et la proximité.
    seen: HashMap<(u8, u16), VecDeque<(&'static str, Instant)>>,
    /// Zone actuelle (carte du jeu) et canaux numérotés géographiques (Général, Commerce...).
    pub zone: u16,
    pub geo: std::collections::HashSet<u8>,
    pending: HashMap<u64, Pending>,
    /// Traductions des brouillons : (clé, traduction, moment).
    drafts: VecDeque<(String, String, Instant)>,
    sent: VecDeque<Sent>,
    pub stats: TrStats,
    /// Traduire aussi les messages du joueur (0.11 : oui par défaut).
    pub own_too: bool,
    /// Durées des dernières traductions de messages reçus (secondes).
    lat: VecDeque<f64>,
}

impl Tr {
    /// Budget d'attente de l'affichage pour une traduction demandée maintenant : le temps de
    /// traduction réellement observé (médiane récente, allongée si la file est chargée). La ligne
    /// patiente ce temps-là dans le miroir, puis s'affiche telle quelle si la traduction tarde.
    /// Ainsi l'attente d'affichage suit la vitesse du modèle (équilibre traduction/affichage,
    /// 0.14.8) : modèle rapide, chat vif ; modèle plus lourd, court délai, toujours sans trou.
    fn budget(&self) -> Duration {
        let mut l: Vec<f64> = self.lat.iter().copied().collect();
        l.sort_by(|a, b| a.total_cmp(b));
        let med = l.get(l.len() / 2).copied().unwrap_or(1.0);
        let waiting = self.pending.values().filter(|p| matches!(p, Pending::Received { .. })).count();
        let eta = ((waiting / crate::engine::PARALLEL) as f64 + 1.0) * med;
        Duration::from_secs_f64(eta.clamp(0.4, mirror::HOLD_MAX.as_secs_f64()))
    }
}

/// Codes de canal (protocole) d'un type de chat de WoW, pour Trad:AUTO ; `target` : numéro du
/// canal pour CHANNEL (chaque canal numéroté a sa langue majoritaire).
fn channel_codes(chat_type: &str, target: &str) -> Vec<u8> {
    match chat_type {
        "SAY" | "EMOTE" => vec![1, 15],
        "YELL" => vec![2, 1],
        "WHISPER" => vec![3, 4],
        "BN_WHISPER" => vec![16, 17],
        "PARTY" | "PARTY_LEADER" => vec![5, 6],
        "RAID" | "RAID_LEADER" | "RAID_WARNING" => vec![7, 8, 9],
        "GUILD" | "GUILD_DISCORD" => vec![10, 18],
        "OFFICER" => vec![11],
        "INSTANCE_CHAT" | "INSTANCE_CHAT_LEADER" => vec![12, 13],
        "COMMUNITIES_CHANNEL" => vec![19],
        "CHANNEL" => match target.trim().parse::<u8>() {
            Ok(n) if (1..=63).contains(&n) => vec![proto::CHANNEL_NUMBERED + n],
            // Canal sans numéro connu : tous les canaux numérotés.
            _ => (14..=14).chain(proto::CHANNEL_NUMBERED + 1..=127).collect(),
        },
        _ => Vec::new(),
    }
}

/// Panneau du bilan d'un combat : traductions des messages reçus pendant le verrouillage du chat
/// (les 12 derniers).
#[cfg_attr(not(windows), allow(dead_code))]
fn digest_rows(items: &[(String, String)]) -> Vec<crate::panel::Row> {
    let mut rows = vec![crate::panel::Row {
        text: format!("Pendant le combat : {} message(s) traduit(s)", items.len()),
        color: crate::panel::TITLE,
        title: true,
    }];
    let skip = items.len().saturating_sub(12);
    if skip > 0 {
        rows.push(crate::panel::Row { text: format!("(… {skip} plus anciens)"), color: crate::panel::DIM, title: false });
    }
    for (who, text) in &items[skip..] {
        let name = who.split('-').next().unwrap_or(who);
        rows.push(crate::panel::Row { text: format!("{name} : {text}"), color: crate::panel::TEXT, title: false });
    }
    rows.push(crate::panel::Row { text: "Texte d'origine dans le chat (lignes reçues pendant le combat).".into(), color: crate::panel::DIM, title: false });
    rows
}

/// Nom de joueur comparable : minuscules, sans le royaume.
fn player_key(name: &str) -> String {
    name.split('-').next().unwrap_or(name).trim().to_lowercase()
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Tr {
    pub fn new(engine: Box<dyn Translator>, client: &'static str) -> Tr {
        Tr {
            engine,
            client,
            speakers: Speakers::default(),
            seen: HashMap::new(),
            zone: 0,
            geo: std::collections::HashSet::new(),
            pending: HashMap::new(),
            drafts: VecDeque::new(),
            sent: VecDeque::new(),
            stats: TrStats::default(),
            lat: VecDeque::new(),
            own_too: true,
        }
    }

    /// Langue cible d'un message du joueur. `dest` : « AUTO » ou un code (Trad:XX). Renvoie la
    /// langue du client quand l'interlocuteur la parle (rien à traduire).
    pub fn target(&self, dest: &str, chat_type: &str, target: &str, now: Instant) -> Option<&'static str> {
        let d = dest.to_lowercase();
        if d != "auto" && !d.is_empty() {
            return lang::LANGS.iter().find(|l| **l == d).copied();
        }
        if chat_type == "WHISPER" || chat_type == "BN_WHISPER" {
            if let Some(l) = self.speakers.language_of(&player_key(target)) {
                return Some(l);
            }
        }
        self.majority(&channel_codes(chat_type, target), now).or((self.client != "en").then_some("en"))
    }

    /// Langue majoritaire des messages vus ces 15 dernières minutes sur ces canaux ; les plus
    /// récents comptent davantage (poids divisé par deux toutes les 5 min).
    fn majority(&self, codes: &[u8], now: Instant) -> Option<&'static str> {
        let mut w: HashMap<&'static str, f64> = HashMap::new();
        for c in codes {
            for (l, t) in self.seen.get(&self.scope(*c)).into_iter().flatten() {
                let age = now.saturating_duration_since(*t).as_secs_f64();
                if age < 900.0 {
                    *w.entry(l).or_default() += 0.5f64.powf(age / 300.0);
                }
            }
        }
        w.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(a.0))).map(|(l, _)| l)
    }

    /// Portée d'un canal (0.13.3) : un canal géographique (Général, Commerce, Défense locale...)
    /// et la proximité (Dire, Crier, émotes) valent pour la zone en cours, les autres partout.
    fn scope(&self, chan: u8) -> (u8, u16) {
        let local = matches!(chan, 1 | 2 | 15)
            || (chan > proto::CHANNEL_NUMBERED && self.geo.contains(&(chan - proto::CHANNEL_NUMBERED)));
        (chan, if local { self.zone } else { 0 })
    }

    fn note_seen(&mut self, chan: u8, l: &'static str, now: Instant) {
        let key = self.scope(chan);
        let q = self.seen.entry(key).or_default();
        q.push_back((l, now));
        while q.len() > 40 {
            q.pop_front();
        }
    }
}

/// Trames du bandeau reçues et manquées (d'après leurs numéros de séquence), par périodes de 30 s
/// (0.12.3) : une trame manquée est un message ou une ligne du miroir perdus jusqu'au renvoi.
#[derive(Default)]
struct BandStats {
    seqs: Vec<u16>,
    since: Option<Instant>,
    total: usize,
    total_missed: usize,
}

impl BandStats {
    fn note(&mut self, seq: u16, now: Instant) {
        self.since.get_or_insert(now);
        self.seqs.push(seq);
    }

    /// Bilan de la période (reçues, manquées), remis à zéro.
    fn take(&mut self) -> (usize, usize) {
        self.since = None;
        let Some(&first) = self.seqs.first() else { return (0, 0) };
        // Numéros relatifs au premier ; ceux d'avant (file prioritaire passée devant) sont ignorés.
        let mut rel: Vec<u16> = self.seqs.iter().map(|s| s.wrapping_sub(first)).filter(|r| *r < 30000).collect();
        self.seqs.clear();
        rel.sort_unstable();
        rel.dedup();
        let got = rel.len();
        let span = rel.last().map_or(0, |m| *m as usize + 1);
        let missed = span.saturating_sub(got);
        self.total += span;
        self.total_missed += missed;
        (got, missed)
    }
}

/// Ligne du miroir qui cite des messages relayés (« \x1DXXXX\x1D », numéro en hexadécimal,
/// 0.12.3) : texte rétabli. `None` si un message cité n'est pas arrivé.
fn resolve_cites(text: &str, bodies: &HashMap<u16, (String, Instant)>) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 128);
    let mut rest = text;
    while let Some(a) = rest.find('\u{1D}') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        let b = after.find('\u{1D}')?;
        let id = u16::from_str_radix(&after[..b], 16).ok()?;
        out.push_str(&bodies.get(&id)?.0);
        rest = &after[b + 1..];
    }
    out.push_str(rest);
    Some(out)
}

pub struct Live {
    /// Contexte du joueur (addon 0.13.3) : 1 combat, 2 instance, 4 raid, 8 absent, 16 chat
    /// verrouillé.
    pub ctx: u8,
    /// Numéro des réglages graphiques de WoW donné par l'addon (0.14.2).
    pub gfx: u8,
    /// Traductions des messages reçus pendant le combat, montrées d'un bloc après : (expéditeur,
    /// traduction) et dernier ajout.
    pub digest: Vec<(String, String)>,
    pub digest_at: Option<Instant>,
    /// Modèle supérieur proposé (0.14) et panneau déjà montré ; réponse du joueur reçue.
    pub proposal: Option<crate::conseil::Proposal>,
    pub proposal_shown: bool,
    pub proposal_answered: bool,
    cmd_nonces: VecDeque<String>,
    test: TestStats,
    band: BandStats,
    addons: Option<PathBuf>,
    errors: HashMap<DecodeError, usize>,
    last_key: Option<(u8, u16, usize)>,
    last_err: Option<DecodeError>,
    idle_since: Instant,
    polls: usize,
    frames_seen: usize,
    chats: HashMap<(u8, u16), Assembly>,
    chats_lost: usize,
    last_progress: Instant,
    mirror: Option<Mirror>,
    font_source: FontSource,
    mirror_hint: bool,
    last_perf: Instant,
    tr: Option<Tr>,
    /// Envois traduits en cours (numéro de demande de l'addon).
    sends: HashMap<u16, SendJob>,
    old_addon_hint: bool,
    occluded: bool,
    occluded_said: Option<Instant>,
    /// Dernier signe de vie de l'addon (chat miroir actif) ; `None` : addon sans signe de vie.
    pub last_alive: Option<Instant>,
    /// Dernier signe d'un addon qui attend l'application (lancée après la connexion : /reload).
    pub waiting_at: Option<Instant>,
    /// Messages relayés récents, par numéro : les lignes du miroir les citent (0.12.3).
    bodies: HashMap<u16, (String, Instant)>,
    /// Lignes qui citaient un message jamais arrivé (attendent leur renvoi complet).
    cites_lost: usize,
    icons_seen: usize,
}

impl Default for Live {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Live {
    /// `addons` : dossier AddOns de la version du jeu (fichier de présence de l'application).
    pub fn new(addons: Option<PathBuf>) -> Self {
        let now = Instant::now();
        Live {
            test: TestStats {
                seen: vec![false; 65536],
                ..Default::default()
            },
            band: BandStats::default(),
            addons,
            errors: HashMap::new(),
            last_key: None,
            last_err: None,
            idle_since: now,
            polls: 0,
            frames_seen: 0,
            chats: HashMap::new(),
            chats_lost: 0,
            last_progress: now,
            mirror: None,
            font_source: FontSource::None,
            mirror_hint: false,
            last_perf: now.checked_sub(Duration::from_secs(60)).unwrap_or(now),
            tr: None,
            sends: HashMap::new(),
            old_addon_hint: false,
            occluded: false,
            occluded_said: None,
            last_alive: None,
            ctx: 0,
            gfx: 0,
            digest: Vec::new(),
            digest_at: None,
            proposal: None,
            proposal_shown: false,
            proposal_answered: false,
            cmd_nonces: VecDeque::new(),
            waiting_at: None,
            bodies: HashMap::new(),
            cites_lost: 0,
            icons_seen: 0,
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn addons_dir(&self) -> Option<&PathBuf> {
        self.addons.as_ref()
    }

    /// Branche la traduction réelle (messages reçus, messages du joueur et Maj+Entrée).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn enable_translation(&mut self, engine: Box<dyn Translator>, client: &'static str, own_too: bool) {
        let mut t = Tr::new(engine, client);
        t.own_too = own_too;
        self.tr = Some(t);
    }

    #[allow(dead_code)]
    pub fn translation(&self) -> Option<&Tr> {
        self.tr.as_ref()
    }

    /// Active le chat miroir (polices de secours chargées tout de suite).
    pub fn enable_mirror(&mut self, source: FontSource, demo: bool, opts: mirror::RenderOpts) {
        let mut m = Mirror::new();
        m.demo = demo;
        m.opts = opts;
        for (name, data) in crate::fonts::fallback_fonts() {
            if let Err(e) = m.book.add_fallback(&name, data) {
                crate::say!("[miroir] police de secours {e}");
            }
        }
        crate::say!(
            "Chat miroir : prêt (polices de secours : {}).",
            {
                let n = m.book.fallback_names();
                if n.is_empty() { "aucune".to_string() } else { n.join(", ") }
            }
        );
        self.mirror = Some(m);
        // Icônes (rang de qualité, marques de raid...) lues dans les archives du jeu.
        if let FontSource::Game { wow, flavor } = &source {
            crate::icons::start(wow.clone(), flavor.clone());
        }
        self.font_source = source;
    }

    /// Tout redessiner (retour au premier plan de WoW).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn mirror_redraw(&mut self) {
        if let Some(m) = self.mirror.as_mut() {
            m.redraw_all();
        }
    }

    /// Fenêtre d'une autre application devant WoW (gauche, haut, droite, bas en pixels du bureau ;
    /// `origin` : coin bas-gauche de la zone de jeu) : le miroir ne dessine rien dessous. `None` :
    /// WoW actif.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn window_over(&mut self, r: Option<(i32, i32, i32, i32)>, origin: (i32, i32)) {
        if let Some(m) = self.mirror.as_mut() {
            m.set_window_occluder(r.map(|(l, t, rr, b)| (l - origin.0, origin.1 - b, rr - l, b - t)));
        }
    }

    /// Clic sur une icône de copie : texte à mettre dans le presse-papiers (la ligne affiche une
    /// coche un instant).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn copy_at(&mut self, frame: u8, x: i32, y: i32, now: Instant) -> Option<String> {
        let m = self.mirror.as_mut()?;
        let (line, text) = m.hit(frame, x, y)?;
        m.mark_copied(line, now);
        Some(text)
    }

    /// Images à poser ou cacher (à appeler une trentaine de fois par seconde).
    pub fn mirror_tick(&mut self, now: Instant) -> Vec<Update> {
        let Some(m) = self.mirror.as_mut() else {
            return Vec::new();
        };
        for path in m.fonts_needed() {
            let loaded = match &self.font_source {
                FontSource::Game { wow, flavor } => crate::fonts::load_game_font(wow, flavor, &path),
                FontSource::File(f) => std::fs::read(f)
                    .map(|d| (d, format!("fichier imposé {}", f.display())))
                    .map_err(|e| format!("{} : {e}", f.display())),
                FontSource::None => Err("aucun dossier WoW indiqué (--wow, --flavor)".into()),
            };
            let data = match loaded {
                Ok((d, how)) => {
                    crate::say!("[miroir] police {path} : {how}");
                    Some(d)
                }
                Err(e) => {
                    crate::say!("[miroir] police {path} indisponible ({e}) : police de secours");
                    None
                }
            };
            if let Err(e) = m.book.set_primary(&path, data) {
                crate::say!("[miroir] police {path} illisible ({e}) : police de secours");
                let _ = m.book.set_primary(&path, None);
            }
        }
        m.tick(now)
    }

    /// Traite une capture ; renvoie vrai si une nouvelle trame a été lue.
    pub fn feed(&mut self, img: &Image) -> bool {
        self.feed_result(crate::decode::decode(img), Instant::now())
    }

    /// Rien de neuf à l'écran : traductions terminées et nettoyage seulement.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn idle(&mut self, now: Instant) {
        self.housekeeping(now);
    }

    /// Résultat d'une capture décodée (fil de capture, 0.12.3).
    pub fn feed_result(&mut self, r: Result<Decoded, DecodeError>, now: Instant) -> bool {
        self.polls += 1;
        let fresh = match r {
            Ok(d) => {
                self.last_err = None;
                self.idle_since = now;
                self.on_frame(d, now)
            }
            Err(DecodeError::NoSync) => {
                self.last_err = None;
                false
            }
            Err(e) => {
                // Une même trame illisible reste affichée plusieurs captures : on ne la compte qu'une fois.
                if self.last_err != Some(e) {
                    *self.errors.entry(e).or_default() += 1;
                }
                self.last_err = Some(e);
                self.idle_since = now;
                false
            }
        };
        self.housekeeping(now);
        fresh
    }

    fn on_frame(&mut self, d: Decoded, now: Instant) -> bool {
        let f = &d.frame;
        let key = (f.ftype, f.seq, f.payload.len());
        if self.last_key == Some(key) {
            // Bandeau immobile (addon 0.13.3) : la même trame encore à l'écran est un signe de vie.
            if f.ftype == crate::proto::FRAME_ALIVE && f.payload.first() == Some(&0) {
                self.waiting_at = Some(now);
            } else if f.ftype != FRAME_TEST && self.last_alive.is_some() {
                self.last_alive = Some(now);
            }
            return false;
        }
        self.last_key = Some(key);
        self.frames_seen += 1;
        if f.ftype != FRAME_TEST {
            self.band.note(f.seq, now);
            // Toute trame de l'addon est un signe de vie (0.12.3) : sous un chat très actif, la file
            // du bandeau ne se vide plus et le signe de vie dédié ne passait plus (miroir caché).
            let beacon = f.ftype == crate::proto::FRAME_ALIVE && f.payload.first() == Some(&0);
            if self.last_alive.is_some() && !beacon {
                self.last_alive = Some(now);
            }
        }
        match f.ftype {
            FRAME_TEST => {
                let t = &mut self.test;
                t.first.get_or_insert(now);
                t.last = Some(now);
                t.geometry = Some((d.geometry.block, d.geometry.cols));
                let maxp = proto::capacity(d.geometry.cols).2;
                if f.payload != proto::test_payload(f.seq, maxp) {
                    t.mismatched += 1;
                }
                if !t.seen[f.seq as usize] {
                    t.seen[f.seq as usize] = true;
                    t.unique += 1;
                    t.payload_bytes += f.payload.len();
                }
                t.max_seq = t.max_seq.max(f.seq);
            }
            FRAME_CHAT | FRAME_SEND | FRAME_MLINE | crate::proto::FRAME_LATE => self.on_fragment(f.ftype, &f.payload, now),
            FRAME_MVIEW => self.on_view(&f.payload, now),
            FRAME_MADV => {
                if let Some(m) = self.mirror.as_mut() {
                    match m.on_measures(&f.payload) {
                        Ok(s) => {
                            crate::say!("[miroir] mesure du jeu : {s}");
                            if f.payload.get(3).is_some_and(|k| *k != 0) {
                                if let Some(l) = m.layout_summary(f.payload[0]) {
                                    crate::say!("[miroir] mise en page ChatFrame{} : {l}", f.payload[0]);
                                }
                            }
                        }
                        Err(e) => crate::say!("[miroir] mesures ignorées : {e}"),
                    }
                }
            }
            FRAME_STATE => self.on_state(&f.payload, now),
            FRAME_OCCLUDE => {
                if let Some(m) = self.mirror.as_mut() {
                    let n = f.payload.first().copied().unwrap_or(0) as usize;
                    // Une ligne par passage de « rien » à « quelque chose » sur le chat, pas plus
                    // d'une toutes les 10 s (les bulles d'aide vont et viennent).
                    if m.set_occluders(&f.payload)
                        && (n > 0) != self.occluded
                        && self.occluded_said.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(10))
                    {
                        self.occluded_said = Some(now);
                        crate::say!(
                            "[miroir] {}",
                            if n > 0 { format!("{n} menu(s), bulle(s) ou fenêtre(s) du jeu sur le chat : le miroir s'efface dessous") } else { "plus rien sur le chat".into() }
                        );
                    }
                    self.occluded = n > 0;
                }
            }
            FRAME_NOTE => crate::say!("[addon] {}", String::from_utf8_lossy(&f.payload)),
            crate::proto::FRAME_ICON => match crate::icons::on_frame(&f.payload) {
                Ok(s) => {
                    self.icons_seen += 1;
                    if self.icons_seen <= 20 {
                        crate::say!("[icône] {s}");
                    }
                }
                Err(e) => crate::say!("[icône] trame illisible : {e}"),
            },
            FRAME_RETARGET => {
                if !self.old_addon_hint {
                    self.old_addon_hint = true;
                    crate::say!("[S3] trame d'un addon antérieur à la 0.11 : relance l'application (elle met l'addon à jour), puis redémarre WoW");
                }
            }
            FRAME_PING => crate::say!("[ping] {}", String::from_utf8_lossy(&f.payload)),
            // [1] : chat miroir actif ; [0] : traduction en attente (application absente à la connexion).
            crate::proto::FRAME_ALIVE => match f.payload.first() {
                Some(0) => self.waiting_at = Some(now),
                _ => {
                    self.last_alive = Some(now);
                    // Contexte du joueur (combat, instance, absence) : les bascules de modèle et le
                    // bilan du combat attendent la fin du combat.
                    if let Some(&c) = f.payload.get(1) {
                        if c != self.ctx {
                            crate::say!(
                                "[contexte] {}",
                                [(1, "combat"), (2, "instance"), (4, "raid"), (8, "absent"), (16, "chat verrouillé")]
                                    .iter()
                                    .filter(|(b, _)| c & b != 0)
                                    .map(|(_, n)| *n)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                        self.ctx = c;
                        crate::engine::set_context(c);
                    }
                    // Réglages graphiques de WoW (addon 0.14.2) : numéro changé à chaque réglage
                    // modifié ; le moteur réévalue la place du modèle dans la mémoire vidéo.
                    if let Some(&g) = f.payload.get(2) {
                        if g != self.gfx && g != 0 {
                            crate::say!("[contexte] réglages graphiques de WoW modifiés");
                        }
                        self.gfx = g;
                        crate::engine::set_gfx(g);
                    }
                }
            },
            crate::proto::FRAME_ZONE if f.payload.len() >= 2 => {
                if let Some(tr) = self.tr.as_mut() {
                    tr.zone = u16::from_be_bytes([f.payload[0], f.payload[1]]);
                    tr.geo = f.payload[2..].iter().copied().collect();
                }
            }
            crate::proto::FRAME_CMD => {
                let t = String::from_utf8_lossy(&f.payload).to_string();
                // La demande part trois fois avec un numéro : comptée une fois.
                if !self.cmd_nonces.contains(&t) {
                    self.cmd_nonces.push_back(t.clone());
                    if self.cmd_nonces.len() > 20 {
                        self.cmd_nonces.pop_front();
                    }
                    let what = t.split('\t').next().unwrap_or_default().to_string();
                    crate::say!("[demande] {what}");
                    match what.as_str() {
                        "monter" => {
                            crate::engine::post(crate::engine::Control::Accept);
                            self.proposal_answered = true;
                        }
                        "non" => {
                            crate::engine::post(crate::engine::Control::Refuse);
                            self.proposal_answered = true;
                        }
                        _ => {}
                    }
                }
            }
            FRAME_PERF => {
                // Deux mesures par seconde : une ligne toutes les 10 s suffit.
                if let Some(p) = crate::fps::parse_perf(&f.payload) {
                    if now.duration_since(self.last_perf) >= Duration::from_secs(10) {
                        self.last_perf = now;
                        match p.worst_ms {
                            Some(w) => crate::say!("[perf] {:.1} images/s, plus longue image {w:.0} ms", p.fps),
                            None => crate::say!("[perf] {:.1} images/s", p.fps),
                        }
                    }
                }
            }
            other => crate::say!("[trame de type inconnu {other}]"),
        }
        true
    }

    fn on_fragment(&mut self, ftype: u8, p: &[u8], now: Instant) {
        if p.len() < 5 || p[2] == 0 || p[3] == 0 || p[2] > p[3] {
            return;
        }
        let id = u16::from_be_bytes([p[0], p[1]]);
        let (part, count, chan) = (p[2] as usize, p[3] as usize, p[4]);
        let a = self.chats.entry((ftype, id)).or_insert_with(|| Assembly {
            parts: vec![None; count],
            chan,
            started: now,
        });
        if a.parts.len() != count {
            *a = Assembly {
                parts: vec![None; count],
                chan,
                started: now,
            };
        }
        a.parts[part - 1] = Some(p[5..].to_vec());
        if a.parts.iter().all(Option::is_some) {
            let a = self.chats.remove(&(ftype, id)).unwrap();
            let body: Vec<u8> = a.parts.into_iter().flatten().flatten().collect();
            let text = String::from_utf8_lossy(&body).into_owned();
            if ftype == FRAME_SEND {
                self.on_send_request(id, &text);
            } else if ftype == FRAME_MLINE {
                // Message cité par son numéro : texte rétabli ; sinon la ligne attend son renvoi.
                let text = if text.contains('\u{1D}') {
                    match resolve_cites(&text, &self.bodies) {
                        Some(t) => t,
                        None => {
                            self.cites_lost += 1;
                            return;
                        }
                    }
                } else {
                    text
                };
                if let Some(m) = self.mirror.as_mut() {
                    // Lignes du jeu (bit de poids fort, addon 0.14.5) : redessinées sans traduction,
                    // pas recopiées au journal.
                    if m.on_line(id, &text) && a.chan & 0x80 == 0 {
                        crate::say!("[miroir {}] {}", a.chan, mirror::plain(&mirror::parse_line_body(&text).text));
                    }
                }
            } else {
                let (sender, msg) = text.split_once('\u{1F}').unwrap_or(("?", &text));
                self.bodies.insert(id, (msg.to_string(), now));
                // Bit de poids fort : message du joueur lui-même (addon 0.9).
                let own = a.chan & 0x80 != 0 || a.chan == 4 || a.chan == 17;
                let chan = a.chan & 0x7F;
                // Canal numéroté : son numéro dans le jeu (/2 Débutants, /4 Services...).
                if chan > proto::CHANNEL_NUMBERED {
                    crate::say!("[Canal {}] {}: {}", chan - proto::CHANNEL_NUMBERED, sender, msg);
                } else {
                    crate::say!("[{}] {}: {}", proto::channel_name(chan), sender, msg);
                }
                if let Some(m) = self.mirror.as_mut() {
                    m.note_chat(msg, sender, now);
                }
                if ftype == crate::proto::FRAME_LATE {
                    self.on_received_at(chan, sender, msg, now, false, true);
                } else if own {
                    self.on_own(chan, sender, msg, now);
                } else {
                    self.on_received(chan, sender, msg, now, false);
                }
            }
        }
    }

    fn on_view(&mut self, p: &[u8], now: Instant) {
        let Some(m) = self.mirror.as_mut() else {
            if !self.mirror_hint {
                self.mirror_hint = true;
                crate::say!("[miroir] description de fenêtre reçue, mais le chat miroir est coupé dans ce décodeur");
            }
            return;
        };
        match mirror::parse_view(p, now) {
            Err(e) => crate::say!("[miroir] description illisible : {e}"),
            Ok(v) => {
                if let Some(v) = m.on_view(v) {
                    if v.flags & mirror::FLAG_VISIBLE == 0 {
                        crate::say!("[miroir] ChatFrame{} : caché", v.frame);
                    } else {
                        crate::say!(
                            "[miroir] ChatFrame{} : {}x{} px en ({}, {}), {} {:.1} px, ligne {:.1} px, ombre {:.1},{:.1}, {} lignes{}{}{}{}{}",
                            v.frame,
                            v.width,
                            v.height,
                            v.left,
                            v.bottom,
                            v.font_path,
                            v.font_px,
                            v.line_h,
                            v.shadow.0,
                            v.shadow.1,
                            v.lines.len(),
                            if v.flags & mirror::FLAG_CALAGE != 0 { ", calage" } else { "" },
                            if v.flags & mirror::FLAG_OFFICIAL != 0 { ", texte officiel (verrouillage ou ligne secrète)" } else { "" },
                            if v.flags & mirror::FLAG_TRUNC != 0 { ", liste tronquée" } else { "" },
                            if v.flags & mirror::FLAG_COVER != 0 { ", dispositif télévision (texte du jeu recouvert)" } else { "" },
                            {
                                let taller = v.height_delta.iter().filter(|d| d.is_some_and(|d| d > 1.0)).count();
                                if taller > 0 { format!(", {taller} ligne(s) plus haute(s) chez WoW (icônes)") } else { String::new() }
                            },
                        );
                    }
                }
            }
        }
    }

    /// Demande S3 : « s|b\x1Flangue\x1Fcible\x1Fcanal\x1Fdestinataire\x1Ftexte » ; « s » : envoi
    /// (Maj+Entrée), « b » : brouillon pendant la frappe.
    fn on_send_request(&mut self, id: u16, body: &str) {
        let f: Vec<&str> = body.splitn(6, '\u{1F}').collect();
        if f.len() != 6 {
            crate::say!("[S3] demande {id} mal formée");
            return;
        }
        let draft = match f[0] {
            "s" => false,
            k if k.starts_with('b') && k[1..].chars().all(|c| c.is_ascii_digit()) => true,
            _ => {
                if !self.old_addon_hint {
                    self.old_addon_hint = true;
                    crate::say!("[S3] demande d'un addon antérieur à la 0.11 : relance l'application (elle met l'addon à jour), puis redémarre WoW");
                }
                return;
            }
        };
        let (dest, chan, target, text) = (f[2], f[3], f[4], f[5]);
        let now = Instant::now();
        let to = if target.is_empty() { String::new() } else { format!(" à {target}") };
        let what = if draft { "brouillon" } else { "envoi" };
        let sends = &mut self.sends;
        if !draft {
            sends.insert(
                id,
                SendJob { text: text.to_string(), tgt: "", paste: None, ready: None, created: now, pasted: false },
            );
        }
        let Some(tr) = self.tr.as_mut() else {
            // Sans traduction réelle (démonstration) : « [→XX] texte ».
            if draft {
                crate::say!("[S3] brouillon {id} ({chan}{to}) : {text}");
            }
            if let Some(j) = sends.get_mut(&id) {
                let t = crate::translate::translate(text, dest);
                crate::say!("[S3] envoi {id} ({chan}{to}) : {text}\n     traduction simulée : {t}");
                j.paste = Some(t);
                j.tgt = "en";
            }
            return;
        };
        // Le joueur écrit d'ordinaire dans la langue du client ; s'il écrit nettement dans une
        // autre, c'est celle-là.
        let src = match lang::detect_for(text, tr.client) {
            Detected::Lang(l, c) if c >= 0.3 => l,
            _ => tr.client,
        };
        let how = if dest.eq_ignore_ascii_case("auto") { "Trad:AUTO" } else { "Trad choisi" };
        match tr.target(dest, chan, target, now) {
            Some(tgt) if tgt != src => {
                let key = format!("{dest}\u{1F}{chan}\u{1F}{target}\u{1F}{text}");
                let job = sends.get_mut(&id);
                // Brouillon déjà traduit : rien à refaire.
                if let Some(t) = tr.drafts.iter().find(|(k, _, _)| *k == key).map(|(_, t, _)| t.clone()) {
                    if let Some(j) = job {
                        crate::say!("[S3] envoi {id} ({chan}{to}) : {text} - {} → {} ({how}), brouillon déjà traduit : {t}", src.to_uppercase(), tgt.to_uppercase());
                        j.paste = Some(t);
                        j.tgt = tgt;
                        tr.stats.sent += 1;
                    }
                    return;
                }
                // Brouillon de ce texte en cours de traduction : l'envoi l'attend.
                let running = tr.pending.values_mut().find_map(|p| match p {
                    Pending::Own { key: k, sends: s, .. } if *k == key => Some(s),
                    _ => None,
                });
                if let Some(waiting) = running {
                    if let Some(j) = job {
                        waiting.push(id);
                        j.tgt = tgt;
                        crate::say!("[S3] envoi {id} ({chan}{to}) : {text} - {} → {} ({how}), brouillon en cours de traduction", src.to_uppercase(), tgt.to_uppercase());
                    }
                    return;
                }
                let ask = Ask {
                    text: text.to_string(),
                    src,
                    tgt,
                    priority: if draft { Priority::Draft } else { Priority::Send },
                    group: draft.then_some(DRAFT_GROUP),
                };
                let n = tr.engine.submit(ask);
                tr.pending.insert(
                    n,
                    Pending::Own { key, text: text.to_string(), src, tgt, sends: if draft { Vec::new() } else { vec![id] } },
                );
                if let Some(j) = job {
                    j.tgt = tgt;
                }
                crate::say!("[S3] {what} {id} ({chan}{to}) : {text} - {} → {} ({how})", src.to_uppercase(), tgt.to_uppercase());
            }
            other => {
                crate::say!(
                    "[S3] {what} {id} ({chan}{to}) : {text} - pas de traduction ({})",
                    match other {
                        Some(_) => "déjà dans la langue visée",
                        None => "langue du destinataire inconnue",
                    }
                );
                if let Some(j) = sends.get_mut(&id) {
                    j.paste = Some(text.to_string());
                    j.tgt = src;
                }
            }
        }
    }

    /// Trame d'état de l'envoi traduit.
    fn on_state(&mut self, p: &[u8], now: Instant) {
        if p.len() < 3 {
            return;
        }
        let id = u16::from_be_bytes([p[1], p[2]]);
        match p[0] {
            STATE_READY => {
                if let Some(j) = self.sends.get_mut(&id) {
                    j.ready.get_or_insert(now);
                }
            }
            STATE_CANCEL => {
                if let Some(j) = self.sends.remove(&id) {
                    if !j.pasted {
                        crate::say!("[S3] envoi {id} annulé dans le jeu");
                    }
                }
            }
            STATE_SENT => {
                if self.sends.remove(&id).is_some() {
                    crate::say!("[S3] envoi {id} parti");
                }
            }
            _ => {}
        }
    }

    /// Envois dont la traduction est à coller maintenant : traduction prête, boîte de saisie prête,
    /// Maj relâchée (ou attendue 0,6 s). Chacun n'est rendu qu'une fois.
    pub fn sends_due(&mut self, now: Instant, shift_held: bool) -> Vec<(u16, String)> {
        let mut out = Vec::new();
        let mut late = Vec::new();
        let mut ids: Vec<u16> = self.sends.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let j = self.sends.get_mut(&id).unwrap();
            let age = now.saturating_duration_since(j.created);
            if j.pasted {
                if age > Duration::from_secs(30) {
                    late.push((id, false));
                }
                continue;
            }
            if age > SEND_DEADLINE {
                late.push((id, true));
                continue;
            }
            let (Some(p), Some(r)) = (&j.paste, j.ready) else { continue };
            if shift_held && now.saturating_duration_since(r) < SHIFT_GRACE {
                continue;
            }
            j.pasted = true;
            out.push((id, p.clone()));
        }
        // Envois collés : de quoi reconnaître le message quand il revient dans le chat.
        if let Some(tr) = self.tr.as_mut() {
            for (id, p) in &out {
                if let Some(j) = self.sends.get(id) {
                    if *p != j.text {
                        tr.sent.push_front(Sent { paste: p.clone(), source: j.text.clone(), tgt: j.tgt, at: now });
                        tr.sent.truncate(20);
                    }
                }
            }
        }
        // Liens d'objets : collés sous forme de repères, remis par l'addon avant l'envoi.
        for (id, p) in out.iter_mut() {
            if let Some(j) = self.sends.get(id) {
                *p = crate::translate::paste_form(p, &j.text);
            }
        }
        for (id, report) in late {
            if let Some(j) = self.sends.remove(&id) {
                if report {
                    crate::say!(
                        "[S3] envoi {id} : {} - {}, abandonné : le texte d'origine revient dans la boîte",
                        j.text,
                        if j.paste.is_none() { "traduction pas prête à temps" } else { "boîte de saisie pas prête à temps" }
                    );
                }
            }
        }
        out
    }

    /// Message du joueur : un envoi traduit qui revient s'affiche avec son texte d'origine ; le
    /// reste est traité comme un message reçu (écrit dans une autre langue que celle du client :
    /// traduit, avec l'acronyme de sa langue).
    fn on_own(&mut self, chan: u8, sender: &str, msg: &str, now: Instant) {
        let Some(tr) = self.tr.as_mut() else {
            if let Some(m) = self.mirror.as_mut() {
                m.settle(msg, now);
            }
            return;
        };
        // Gardé 2 min : un chuchotement à soi-même revient deux fois (envoyé, reçu).
        tr.sent.retain(|s| now.saturating_duration_since(s.at) < Duration::from_secs(120));
        if let Some(s) = tr.sent.iter().find(|s| s.paste.trim() == msg.trim()) {
            if let Some(m) = self.mirror.as_mut() {
                if s.tgt != tr.client && !s.tgt.is_empty() {
                    m.set_translation(msg, sender, &format!("[{}] {}", s.tgt.to_uppercase(), s.source), now);
                } else {
                    m.settle(msg, now);
                }
            }
            return;
        }
        if !tr.own_too {
            if let Some(m) = self.mirror.as_mut() {
                m.settle(msg, now);
            }
            return;
        }
        self.on_received(chan, sender, msg, now, true);
    }

    /// Message d'un joueur (ou du joueur lui-même, `own`) : langue, puis traduction s'il n'est pas
    /// dans la langue du client.
    fn on_received(&mut self, chan: u8, sender: &str, msg: &str, now: Instant, own: bool) {
        self.on_received_at(chan, sender, msg, now, own, false)
    }

    /// `late` : message reçu pendant le combat, relayé après ; sa traduction rejoint le bilan du
    /// combat (ses lignes, encore secrètes, restent celles de WoW).
    fn on_received_at(&mut self, chan: u8, sender: &str, msg: &str, now: Instant, own: bool, late: bool) {
        let Some(tr) = self.tr.as_mut() else { return };
        tr.stats.received += 1;
        let client = tr.client;
        let d = tr.speakers.observe(&player_key(sender), msg, client);
        let foreign = match d {
            Detected::Lang(l, _) => {
                // Trad:AUTO : la langue des autres compte toujours ; celle du joueur seulement
                // quand il écrit lui-même dans une autre langue que la sienne.
                if !own || l != client {
                    tr.note_seen(chan, l, now);
                }
                if l != client {
                    tr.stats.foreign += 1;
                    Some(l)
                } else {
                    None
                }
            }
            Detected::Neutral => {
                tr.stats.neutral += 1;
                None
            }
        };
        // Tout message dans une autre langue que celle du client est traduit (règle du 09/11/2026).
        // La ligne attend sa traduction dans le miroir (elle s'affichera d'un coup, traduite) si
        // elle peut être prête à temps ; sinon elle s'affiche tout de suite et la traduction la
        // remplace à son arrivée.
        let until = now + tr.budget();
        match (foreign, self.mirror.as_mut()) {
            (Some(l), Some(m)) if !late => {
                // La ligne patiente le temps d'une traduction, puis s'affiche telle quelle : jamais
                // de trou, et la traduction la remplace à son arrivée (0.14.8).
                m.hold(msg, sender, until);
                let job = tr.engine.submit(Ask { text: msg.to_string(), src: l, tgt: client, priority: Priority::Received, group: None });
                tr.pending.insert(job, Pending::Received { body: msg.to_string(), sender: sender.to_string(), src: l, late });
            }
            (Some(l), _) => {
                let job = tr.engine.submit(Ask { text: msg.to_string(), src: l, tgt: client, priority: Priority::Received, group: None });
                tr.pending.insert(job, Pending::Received { body: msg.to_string(), sender: sender.to_string(), src: l, late });
            }
            (None, Some(m)) => m.settle(msg, now),
            (None, None) => {}
        }
    }

    /// Traductions terminées, et état du moteur.
    fn poll_translations(&mut self) {
        let Some(tr) = self.tr.as_mut() else { return };
        let now = Instant::now();
        let events = tr.engine.poll();
        let mut ready: Vec<(u16, String)> = Vec::new();
        let mut failed: Vec<u16> = Vec::new();
        for e in events {
            match e {
                Event::Status(s) => crate::say!("[traduction] {s}"),
                Event::Propose(p) => {
                    crate::say!("[proposition] {} {} : panneau sur le jeu hors combat", p.model.name, p.place.label());
                    self.proposal = Some(p);
                    self.proposal_shown = false;
                }
                Event::Done { id, result, secs, cached } => match (tr.pending.remove(&id), result) {
                    (Some(Pending::Received { body, sender, src, late }), Ok(t)) => {
                        tr.stats.translated += 1;
                        if cached {
                            tr.stats.cached += 1;
                        } else {
                            tr.lat.push_back(secs);
                            if tr.lat.len() > 20 {
                                tr.lat.pop_front();
                            }
                        }
                        let shown = format!("[{}] {t}", src.to_uppercase());
                        crate::say!(
                            "[trad {} → {}{}] {t}",
                            src.to_uppercase(),
                            tr.client.to_uppercase(),
                            if cached { ", déjà traduit".to_string() } else { format!(", {secs:.1} s").replace('.', ",") }
                        );
                        if let Some(m) = self.mirror.as_mut() {
                            m.set_translation(&body, &sender, &shown, now);
                        }
                        if late {
                            self.digest.push((sender.clone(), shown.clone()));
                            self.digest_at = Some(now);
                        }
                    }
                    (Some(Pending::Received { body, .. }), Err(e)) => {
                        tr.stats.failed += 1;
                        if e != "trop ancien" {
                            crate::say!("[trad] échec : {e} (message laissé tel quel)");
                        }
                        if let Some(m) = self.mirror.as_mut() {
                            m.settle(&body, now);
                        }
                    }
                    (Some(Pending::Own { key, src, tgt, sends, .. }), Ok(t)) => {
                        let t = crate::translate::truncate_utf8(&t, crate::translate::MAX_CHAT_BYTES);
                        crate::say!(
                            "     {} : {} → {} prête en {secs:.1} s : {t}",
                            if sends.is_empty() { "brouillon".to_string() } else { format!("envoi {}", sends.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ")) },
                            src.to_uppercase(),
                            tgt.to_uppercase()
                        );
                        tr.drafts.retain(|(k, _, _)| *k != key);
                        tr.drafts.push_front((key, t.clone(), now));
                        tr.drafts.truncate(30);
                        tr.stats.sent += sends.len();
                        ready.extend(sends.into_iter().map(|s| (s, t.clone())));
                    }
                    (Some(Pending::Own { key, text, src, tgt, sends }), Err(e)) => {
                        if sends.is_empty() {
                            continue;
                        }
                        if e == "remplacé" {
                            // Brouillon écarté par un plus récent alors qu'un envoi l'attendait.
                            let n = tr.engine.submit(Ask { text: text.clone(), src, tgt, priority: Priority::Send, group: None });
                            tr.pending.insert(n, Pending::Own { key, text, src, tgt, sends });
                        } else {
                            crate::say!("     traduction impossible ({e}) : le texte d'origine revient dans la boîte");
                            failed.extend(sends);
                        }
                    }
                    (None, _) => {}
                },
            }
        }
        for (id, t) in ready {
            if let Some(j) = self.sends.get_mut(&id) {
                j.paste = Some(t);
            }
        }
        for id in failed {
            self.sends.remove(&id);
        }
    }

    fn housekeeping(&mut self, now: Instant) {
        self.poll_translations();
        if let Some(tr) = self.tr.as_mut() {
            tr.drafts.retain(|(_, _, t)| now.saturating_duration_since(*t) < Duration::from_secs(600));
        }
        let before = self.chats.len();
        self.chats
            .retain(|_, a| now.duration_since(a.started) < Duration::from_secs(10));
        self.chats_lost += before - self.chats.len();
        self.bodies.retain(|_, (_, t)| now.saturating_duration_since(*t) < Duration::from_secs(60));
        if self.band.since.is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(30)) {
            let (got, missed) = self.band.take();
            if missed > 0 {
                crate::say!(
                    "[bandeau] {missed} trame(s) manquée(s) sur {} en 30 s ({}) : messages ou lignes perdus jusqu'au prochain renvoi",
                    got + missed,
                    format!("{:.1} %", 100.0 * missed as f64 / (got + missed) as f64).replace('.', ",")
                );
            }
        }

        if self.test.unique > 0 {
            if now.duration_since(self.idle_since) > Duration::from_secs(3) {
                self.print_summary();
                self.test = TestStats {
                    seen: vec![false; 65536],
                    ..Default::default()
                };
                self.errors.clear();
                self.frames_seen = 0;
                self.polls = 0;
            } else if now.duration_since(self.last_progress) > Duration::from_secs(2) {
                self.last_progress = now;
                crate::say!(
                    "  ... {} trames de test reçues (dernière n°{})",
                    self.test.unique, self.test.max_seq
                );
            }
        }
    }

    pub fn print_summary(&self) {
        let t = &self.test;
        let expected = t.max_seq as usize + 1;
        let secs = match (t.first, t.last) {
            (Some(a), Some(b)) => b.duration_since(a).as_secs_f64().max(0.001),
            _ => 0.001,
        };
        crate::say!("==== Résultat du test S1 ====");
        if let Some((b, c)) = t.geometry {
            crate::say!(
                "  Bandeau : blocs de {b} px, {c} colonnes, {} octets utiles par trame",
                proto::capacity(c).2
            );
        }
        crate::say!(
            "  Trames reçues : {} sur {} attendues (perdues : {})",
            t.unique,
            expected,
            expected - t.unique
        );
        crate::say!(
            "  Contenus faux acceptés : {}  (doit rester à 0)",
            t.mismatched
        );
        let mut errs: Vec<_> = self
            .errors
            .iter()
            .map(|(e, n)| format!("{} x{n}", e.label()))
            .collect();
        errs.sort();
        crate::say!(
            "  Trames illisibles vues : {}",
            if errs.is_empty() {
                "aucune".into()
            } else {
                errs.join(", ")
            }
        );
        let fps = t.unique as f64 / secs;
        let avg = t.payload_bytes as f64 / t.unique.max(1) as f64;
        crate::say!(
            "  Durée {:.1} s : {:.1} trames/s, débit utile {:.0} octets/s (charge moyenne {:.0} octets/trame)",
            secs,
            fps,
            t.payload_bytes as f64 / secs,
            avg
        );
        if let Some((_, c)) = t.geometry {
            crate::say!(
                "  Capacité à pleine trame : {:.0} octets/s (objectif ≥ 2000)",
                proto::capacity(c).2 as f64 * fps
            );
        }
        if self.frames_seen > 0 {
            crate::say!(
                "  Captures par trame : {:.1}",
                self.polls as f64 / self.frames_seen as f64
            );
        }
        if self.chats_lost > 0 {
            crate::say!(
                "  Messages de chat incomplets abandonnés : {}",
                self.chats_lost
            );
        }
        if self.cites_lost > 0 {
            crate::say!("  Lignes du miroir citant un message manquant : {}", self.cites_lost);
        }
        if self.band.total_missed > 0 {
            crate::say!("  Trames du bandeau manquées : {} sur {}", self.band.total_missed, self.band.total);
        }
    }
}

/// Sans signe de vie de l'addon pendant ce temps, le miroir est caché.
#[cfg_attr(not(windows), allow(dead_code))]
pub const ALIVE_TIMEOUT: Duration = Duration::from_millis(1200);

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub fn run(
    width: usize,
    height: usize,
    poll_ms: u64,
    addons: Option<PathBuf>,
    fonts: FontSource,
    demo: bool,
    opts: mirror::RenderOpts,
    tr: Option<(crate::engine::Config, &'static str, bool)>,
    keep: &mut dyn FnMut() -> bool,
) -> Result<(), String> {
    use crate::overlay_win::Overlay;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    // Fin de la boucle (`keep` renvoie faux : Quitter, WoW fermé ou lancé dans une autre version,
    // ou erreur) : le fil de capture s'arrête aussi.
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let stop = Stop(Arc::new(AtomicBool::new(false)));
    let stopped = stop.0.clone();
    // Capture et décodage du bandeau dans leur propre fil (0.12.3) : le dessin du miroir, long sous
    // un chat très actif, ne fait plus manquer de trames (chacune ne reste que 50 ms à l'écran).
    let (cap_tx, cap_rx) = std::sync::mpsc::channel::<Result<Result<Decoded, DecodeError>, String>>();
    let (dims_tx, dims_rx) = std::sync::mpsc::sync_channel::<Result<(i32, i32, usize, usize), String>>(1);
    std::thread::spawn(move || {
        let mut cap = match crate::capture_win::Capturer::new(width, height) {
            Ok(c) => {
                let _ = dims_tx.send(Ok((c.screen_w, c.screen_h, c.w, c.h)));
                c
            }
            Err(e) => {
                let _ = dims_tx.send(Err(e));
                return;
            }
        };
        // Seuls les changements partent : une même trame reste plusieurs captures à l'écran.
        let mut last: Option<Result<(u8, u16, usize), DecodeError>> = None;
        let mut resent = Instant::now();
        while !stopped.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(poll_ms));
            let (w, h) = (cap.w, cap.h);
            let r = match cap.grab() {
                Ok(buf) => crate::decode::decode(&Image { w, h, stride: w * 4, bgra: buf }),
                Err(e) => {
                    let _ = cap_tx.send(Err(e));
                    return;
                }
            };
            let sig = r.as_ref().map(|d| (d.frame.ftype, d.frame.seq, d.frame.payload.len())).map_err(|e| *e);
            // Bandeau immobile (addon 0.13.3) : la même trame lisible est renvoyée toutes les
            // 300 ms, signe que le jeu est toujours là.
            let still = sig.is_ok() && last.as_ref() == Some(&sig) && resent.elapsed() >= Duration::from_millis(300);
            if last.as_ref() != Some(&sig) || still {
                last = Some(sig);
                resent = Instant::now();
                if cap_tx.send(Ok(r)).is_err() {
                    return;
                }
            }
        }
    });
    // Jamais d'attente sans fin : une capture qui ne démarre pas devient une erreur (nouvel essai
    // dans 5 s, bulle), au lieu d'une application figée sans présence ni chat traduit.
    let (screen_w, screen_h, cap_w, cap_h) = match dims_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(r) => r?,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err("capture du bandeau bloquée au démarrage".into()),
        Err(_) => return Err("fil de capture arrêté".into()),
    };
    crate::say!(
        "Décodeur en écoute : écran {}x{}, zone {}x{} px en bas à droite, capture toutes les {} ms.",
        screen_w, screen_h, cap_w, cap_h, poll_ms
    );
    match &addons {
        Some(d) => crate::say!("Présence de l'application écrite dans {} (ChevaliersTranslators_Live) : la traduction démarre seule à la connexion.", d.display()),
        None => crate::say!("Sans --wow et --flavor : la traduction ne démarre pas seule (en jeu : /chevaliers on)."),
    }
    let mut live = Live::new(addons);
    live.enable_mirror(fonts, demo, opts);
    match tr {
        Some((cfg, client, own_too)) => {
            crate::say!(
                "Traduction : vers {} (langue du client) ; modèle choisi par l'estimation, chargement en cours. Journal du serveur : {}",
                client.to_uppercase(),
                cfg.log.display()
            );
            if !own_too {
                crate::say!("Tes propres messages ne sont pas traduits (--sans-mes-messages).");
            }
            live.enable_translation(Box::new(crate::engine::Engine::start(cfg)), client, own_too);
        }
        None => crate::say!("Traduction : simulée ([→XX] devant le texte, --sans-traduction ou --demo-trad)"),
    }
    let mut overlay = Overlay::new()?;
    if let (Some(open), Some(dir)) = (crate::capture_win::wow_flavor(), live.addons_dir()) {
        let asked = dir.parent().and_then(|p| p.parent()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        if asked.as_deref().is_some_and(|a| a != open) {
            crate::say!(
                "ATTENTION : WoW ouvert en {open}, application lancée pour {} : polices et choix du modèle de {} (relancer avec --flavor {open}).",
                asked.as_deref().unwrap_or("?"),
                asked.as_deref().unwrap_or("?")
            );
        }
    }
    crate::say!("En jeu : la traduction démarre seule à la connexion (WoW déjà ouvert : /reload). Maj+Entrée : envoi traduit.");
    // Panneau « /reload » posé sur le jeu quand l'addon attend l'application.
    const HINT: u8 = 251;
    const DIGEST: u8 = 250;
    const PROPOSAL: u8 = 249;
    const DOWNLOADS: u8 = 248;
    let mut proposal_until: Option<Instant> = None;
    let mut downloads_until: Option<Instant> = None;
    let mut downloads_said = false;
    let mut game_since: Option<Instant> = None;
    let mut screen_info: Option<String> = None;
    let mut screen_checked = Instant::now() - Duration::from_secs(10);
    let mut digest_until: Option<Instant> = None;
    let mut hint_fonts: Option<crate::panel::Fonts> = None;
    let mut hint_until: Option<Instant> = None;
    let mut hint_last: Option<Instant> = None;
    let mut last_raise = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_presence: Option<Instant> = None;
    let mut overlay_error = false;
    // Presse-papiers du joueur remis après un collage : (moment, contenu d'avant, texte collé).
    let mut restore: Option<(Instant, Option<String>, String)> = None;
    // Le miroir ne s'affiche que par-dessus WoW : caché dès qu'une autre fenêtre est active.
    let mut wow_front = true;
    let mut combat_seen = false;
    loop {
        // Trames décodées par le fil de capture (attente courte : fenêtres et collages restent vifs).
        let mut got: Vec<Result<Decoded, DecodeError>> = Vec::new();
        match cap_rx.recv_timeout(Duration::from_millis(poll_ms.max(4))) {
            Ok(r) => got.push(r?),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("fil de capture arrêté".into()),
        }
        for r in cap_rx.try_iter() {
            got.push(r?);
        }
        let now = Instant::now();
        if got.is_empty() {
            live.idle(now);
        }
        for r in got {
            live.feed_result(r, now);
        }
        overlay.pump();
        if !keep() {
            drop(stop);
            return Ok(());
        }
        // Présence de l'application, relue par l'addon à chaque connexion et /reload.
        if last_presence.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(2)) {
            last_presence = Some(now);
            if let Some(dir) = live.addons_dir() {
                // Dans toutes les versions du jeu installées : celle qu'on lance n'importe pas.
                if let Err(e) = crate::install::write_live_everywhere(dir) {
                    if !overlay_error {
                        crate::say!("[présence] {e}");
                        overlay_error = true;
                    }
                }
            }
        }
        // Envoi traduit : collage puis Entrée, seulement si WoW est devant (boîte de saisie prête).
        for (id, text) in live.sends_due(now, crate::keys_win::shift_held()) {
            if !Overlay::wow_in_front() {
                crate::say!("[S3] envoi {id} : WoW n'est plus au premier plan, rien n'est collé");
                continue;
            }
            let saved = crate::clipboard::get();
            match crate::clipboard::set(&text).and_then(|_| crate::keys_win::paste_and_enter()) {
                Ok(()) => {
                    crate::say!("[S3] envoi {id} : collé et envoyé : {text}");
                    restore = Some((Instant::now() + Duration::from_millis(700), saved, text));
                }
                Err(e) => crate::say!("[S3] envoi {id} : collage impossible ({e}) ; la traduction est dans le presse-papiers (Ctrl+V)"),
            }
        }
        if let Some((at, saved, pasted)) = &restore {
            if now >= *at {
                // Remis seulement si le presse-papiers contient encore notre texte.
                if crate::clipboard::get().as_deref() == Some(pasted.as_str()) {
                    if let Some(s) = saved {
                        let _ = crate::clipboard::set(s);
                    }
                }
                restore = None;
            }
        }
        // Clic sur une icône de copie.
        for (frame, x, y) in overlay.take_clicks() {
            if let Some(text) = live.copy_at(frame, x, y, now) {
                match crate::clipboard::set(&text) {
                    Ok(()) => crate::say!("[copie] {text}"),
                    Err(e) => crate::say!("[copie] impossible : {e}"),
                }
            }
        }
        if now.duration_since(last_tick) >= Duration::from_millis(33) {
            last_tick = now;
            // Caché dès que l'addon ne donne plus signe de vie (déconnexion, écran de chargement,
            // choix du personnage) ou que WoW est fermé ou réduit ; revient avec lui. Une autre
            // application active ne le cache plus (0.12) : le chat reste sur WoW, sauf sous sa
            // fenêtre. WoW derrière elle : le bandeau peut être masqué, l'état d'avant est gardé.
            let in_front = Overlay::wow_in_front();
            let alive = live.last_alive.is_none_or(|t| now.saturating_duration_since(t) < ALIVE_TIMEOUT);
            let front = if in_front {
                alive
            } else if crate::capture_win::wow_client_rect().is_none() {
                false
            } else {
                wow_front
            };
            // Combat (0.14) : rien n'est dessiné sur le jeu (chat du jeu, panneaux cachés) ; la
            // traduction continue, et hors combat le chat traduit revient avec ce qui a été traduit.
            let combat = crate::engine::in_combat();
            if front != wow_front {
                wow_front = front;
                if front && !combat {
                    live.mirror_redraw();
                } else if !front {
                    overlay.hide_all();
                }
            }
            if combat != combat_seen {
                combat_seen = combat;
                if combat {
                    overlay.hide_all();
                    hint_until = None;
                    downloads_until = None;
                } else if wow_front {
                    live.mirror_redraw();
                }
            }
            // Un clic dans WoW peut le passer devant le miroir : on remet le miroir devant. Une autre
            // application devant WoW : rien n'est dessiné sous sa fenêtre.
            if now.saturating_duration_since(last_raise) >= Duration::from_millis(100) {
                last_raise = now;
                overlay.set_wow_rect(crate::capture_win::wow_client_rect());
                // Écran de WoW (et ses changements) au journal : télévision, autre écran, échelle.
                if now.saturating_duration_since(screen_checked) >= Duration::from_secs(5) {
                    screen_checked = now;
                    if let Some((info, tv)) = crate::capture_win::wow_screen_info() {
                        if screen_info.as_deref() != Some(info.as_str()) {
                            crate::say!("[écran] {info}");
                            screen_info = Some(info);
                        }
                        // Télévision : dispositif télévision à la prochaine connexion ou /reload
                        // (l'addon lit la présence à ce moment-là).
                        if tv != crate::install::screen_tv() {
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
                    overlay.raise_all();
                    live.window_over(None, overlay.origin());
                } else {
                    live.window_over(Overlay::window_over_wow(), overlay.origin());
                }
            }
            // L'addon attend l'application (lancée après la connexion) : « /reload » sur le jeu,
            // 20 s, au plus toutes les 3 min.
            let waiting = live.waiting_at.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(25))
                && live.last_alive.is_none_or(|t| live.waiting_at.is_some_and(|w| w > t));
            if waiting && in_front && !combat && hint_until.is_none() && hint_last.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(180)) {
                if let Some((l, top, r, _)) = crate::capture_win::wow_client_rect() {
                    let fonts = hint_fonts.get_or_insert_with(crate::panel::Fonts::load);
                    let px = ((r - l) as f32 / 120.0).clamp(14.0, 32.0);
                    let rows = vec![
                        crate::panel::Row { text: "Chevaliers Translators".into(), color: crate::panel::TITLE, title: true },
                        crate::panel::Row { text: "L'application est lancée : tape /reload pour traduire le chat.".into(), color: crate::panel::TEXT, title: false },
                        crate::panel::Row { text: "La prochaine fois, lance Chevaliers Translators avant WoW.".into(), color: crate::panel::DIM, title: false },
                    ];
                    let cv = crate::panel::render(fonts, &rows, ((r - l) as f32 * 0.32) as usize, px);
                    let left = l + ((r - l) - cv.w as i32) / 2;
                    if overlay.show_screen(HINT, left, top + (px * 4.0) as i32, &cv).is_ok() {
                        crate::say!("[présence] l'addon attend l'application : « /reload » affiché sur le jeu");
                        hint_until = Some(now + Duration::from_secs(20));
                        hint_last = Some(now);
                    }
                }
            }
            if hint_until.is_some_and(|t| now >= t || !waiting) {
                overlay.hide(HINT);
                hint_until = None;
            }
            // Bilan du combat (0.13.3) : les messages reçus pendant le verrouillage du chat,
            // traduits après, d'un bloc, une fois le combat fini (2 s sans nouvel ajout).
            let calm = !crate::engine::in_combat();
            if digest_until.is_none()
                && in_front
                && calm
                && !live.digest.is_empty()
                && live.digest_at.is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(2))
            {
                if let Some((l, top, r, b)) = crate::capture_win::wow_client_rect() {
                    let fonts = hint_fonts.get_or_insert_with(crate::panel::Fonts::load);
                    let px = ((r - l) as f32 / 140.0).clamp(13.0, 28.0);
                    let items: Vec<(String, String)> = live.digest.drain(..).collect();
                    let cv = crate::panel::render(fonts, &digest_rows(&items), ((r - l) as f32 * 0.34) as usize, px);
                    // En haut à gauche de l'écran du jeu, sous le cadre du personnage.
                    let left = l + (px * 2.0) as i32;
                    let y = (top + (b - top) / 5).min(b - cv.h as i32);
                    if overlay.show_screen(DIGEST, left, y, &cv).is_ok() {
                        crate::say!("[combat] bilan affiché : {} message(s) traduit(s) après le combat", items.len());
                        digest_until = Some(now + Duration::from_secs((12 + 3 * items.len() as u64).min(40)));
                    }
                }
            }
            if digest_until.is_some_and(|t| now >= t || !calm) {
                overlay.hide(DIGEST);
                digest_until = None;
            }
            // Proposition d'un modèle supérieur (0.14) : hors combat, une minute après l'arrivée
            // en jeu, 40 s, une fois par session ; cachée dès la réponse du joueur.
            let settled = alive && live.last_alive.is_some() && game_since.is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(60));
            if alive && live.last_alive.is_some() {
                game_since.get_or_insert(now);
            } else {
                game_since = None;
            }
            if proposal_until.is_none() && in_front && calm && settled && !live.proposal_shown && !live.proposal_answered {
                if let (Some(p), Some((l, top, r, _))) = (live.proposal.clone(), crate::capture_win::wow_client_rect()) {
                    let fonts = hint_fonts.get_or_insert_with(crate::panel::Fonts::load);
                    let px = ((r - l) as f32 / 130.0).clamp(13.0, 28.0);
                    let cv = crate::panel::render(fonts, &crate::conseil::panel_rows(&p), ((r - l) as f32 * 0.36) as usize, px);
                    let left = l + ((r - l) - cv.w as i32) / 2;
                    if overlay.show_screen(PROPOSAL, left, top + (px * 4.0) as i32, &cv).is_ok() {
                        crate::say!("[proposition] panneau affiché : {} {}", p.model.name, p.place.label());
                        live.proposal_shown = true;
                        proposal_until = Some(now + Duration::from_secs(40));
                    }
                }
            }
            if proposal_until.is_some_and(|t| now >= t || !calm || live.proposal_answered) {
                overlay.hide(PROPOSAL);
                proposal_until = None;
            }
            // Téléchargement des modèles en cours (0.14) : dit une fois par session, en arrivant en
            // jeu, 15 s.
            if downloads_until.is_none() && !downloads_said && in_front && calm && settled {
                let st = crate::telechargements::status();
                downloads_said = true;
                if st.active {
                    if let Some((l, top, r, _)) = crate::capture_win::wow_client_rect() {
                        let fonts = hint_fonts.get_or_insert_with(crate::panel::Fonts::load);
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
                        let cv = crate::panel::render(fonts, &rows, ((r - l) as f32 * 0.34) as usize, px);
                        let left = l + ((r - l) - cv.w as i32) / 2;
                        if overlay.show_screen(DOWNLOADS, left, top + (px * 4.0) as i32, &cv).is_ok() {
                            downloads_until = Some(now + Duration::from_secs(15));
                        }
                    }
                }
            }
            if downloads_until.is_some_and(|t| now >= t || combat) {
                overlay.hide(DOWNLOADS);
                downloads_until = None;
            }
            let updates = live.mirror_tick(now);
            if !wow_front || combat {
                continue;
            }
            for u in updates {
                let r = match &u.canvas {
                    Some(cv) => overlay.show(u.frame, u.left, u.bottom, cv, &u.hits),
                    None => {
                        overlay.hide(u.frame);
                        Ok(())
                    }
                };
                if let Err(e) = r {
                    if !overlay_error {
                        crate::say!("[miroir] affichage impossible : {e}");
                        overlay_error = true;
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn run(
    _width: usize,
    _height: usize,
    _poll_ms: u64,
    _addons: Option<PathBuf>,
    _fonts: FontSource,
    _demo: bool,
    _opts: mirror::RenderOpts,
    _tr: Option<(crate::engine::Config, &'static str, bool)>,
    _keep: &mut dyn FnMut() -> bool,
) -> Result<(), String> {
    Err("le décodeur en direct capture l'écran de Windows : lance-le depuis PowerShell, pas depuis WSL.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<(String, &'static str, &'static str, bool)>>>;

    /// Faux traducteur : « <cible> texte », tout de suite.
    struct Fake {
        next: u64,
        log: Log,
        ready: VecDeque<Event>,
    }

    impl Translator for Fake {
        fn submit(&mut self, a: Ask) -> u64 {
            self.next += 1;
            self.log.borrow_mut().push((a.text.clone(), a.src, a.tgt, a.priority == Priority::Send));
            self.ready.push_back(Event::Done { id: self.next, result: Ok(format!("<{}> {}", a.tgt, a.text)), secs: 0.4, cached: false });
            self.next
        }
        fn poll(&mut self) -> Vec<Event> {
            self.ready.drain(..).collect()
        }
    }

    fn live_opt(client: &'static str, own_too: bool) -> (Live, Log) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut l = Live::new(None);
        l.enable_mirror(FontSource::None, false, mirror::RenderOpts::default());
        l.enable_translation(Box::new(Fake { next: 0, log: log.clone(), ready: VecDeque::new() }), client, own_too);
        (l, log)
    }

    fn live(client: &'static str) -> (Live, Log) {
        live_opt(client, true)
    }

    fn frag(id: u16, chan: u8, body: &str) -> Vec<u8> {
        let mut p = vec![(id >> 8) as u8, id as u8, 1, 1, chan];
        p.extend_from_slice(body.as_bytes());
        p
    }

    fn chat(l: &mut Live, id: u16, chan: u8, sender: &str, text: &str) {
        l.on_fragment(FRAME_CHAT, &frag(id, chan, &format!("{sender}\u{1F}{text}")), Instant::now());
    }

    #[test]
    fn foreign_messages_are_translated_into_the_mirror() {
        // --sans-mes-messages : les messages du joueur restent tels quels.
        let (mut l, log) = live_opt("fr", false);
        let line = "[2. Commerce] [Hans] : Suche Gruppe für die Quest im Schlingendorntal, bin Stufe 38.";
        l.on_fragment(FRAME_MLINE, &frag(7, 0, &format!("FFC0C0\u{1F}{line}")), Instant::now());
        chat(&mut l, 1, 14, "Hans-Ysondre", "Suche Gruppe für die Quest im Schlingendorntal, bin Stufe 38.");
        // Message en français, jargon seul, et message du joueur lui-même : pas traduits.
        chat(&mut l, 2, 14, "Pierre", "Salut tout le monde, quelqu'un pour faire le donjon ?");
        chat(&mut l, 3, 14, "Bob", "LF2M tank heal");
        chat(&mut l, 4, 0x80 | 1, "Moi", "Hello everyone, anyone up for a dungeon run?");
        l.poll_translations();
        assert_eq!(log.borrow().len(), 1);
        assert_eq!(log.borrow()[0].1, "de");
        assert_eq!((log.borrow()[0].2, log.borrow()[0].3), ("fr", false));
        let shown = l.mirror.as_ref().unwrap().shown(7).unwrap();
        assert_eq!(shown, "[2. Commerce] [Hans] : [DE] <fr> Suche Gruppe für die Quest im Schlingendorntal, bin Stufe 38.");
        let st = &l.translation().unwrap().stats;
        assert_eq!((st.received, st.foreign, st.neutral, st.translated), (3, 1, 1, 1));
    }

    #[test]
    fn shift_enter_targets_the_right_language() {
        let (mut l, log) = live("fr");
        // Hans écrit en allemand en chuchotement, Olav en norvégien sur le canal de guilde.
        chat(&mut l, 1, 3, "Hans-Ysondre", "Hallo zusammen, hat jemand Lust auf einen Dungeon?");
        chat(&mut l, 2, 10, "Olav", "takk for gruppen, ha en fin kveld");
        let send = |l: &mut Live, req: u16, dest: &str, chan: &str, target: &str, text: &str| {
            l.on_send_request(req, &format!("s\u{1F}0\u{1F}{dest}\u{1F}{chan}\u{1F}{target}\u{1F}{text}"));
        };
        // Trad:AUTO : chuchotement à Hans -> allemand ; guilde -> norvégien ; dire, inconnu -> anglais.
        send(&mut l, 10, "AUTO", "WHISPER", "Hans", "merci pour le groupe, bonne soirée");
        send(&mut l, 11, "AUTO", "GUILD", "", "merci pour le groupe, bonne soirée");
        send(&mut l, 12, "AUTO", "SAY", "", "merci pour le groupe, bonne soirée");
        // Trad:IT choisi à la main : l'emporte.
        send(&mut l, 13, "IT", "WHISPER", "Hans", "merci pour le groupe, bonne soirée");
        let got: Vec<(&str, &str, bool)> = log.borrow().iter().skip(2).map(|(_, s, t, u)| (*s, *t, *u)).collect();
        assert_eq!(got, [("fr", "de", true), ("fr", "no", true), ("fr", "en", true), ("fr", "it", true)]);
        l.poll_translations();
        assert_eq!(l.translation().unwrap().stats.sent, 4);
        let t0 = Instant::now();
        // Traduction prête, mais boîte de saisie pas encore rouverte : rien à coller.
        assert!(l.sends_due(t0, false).is_empty());
        let ready = |l: &mut Live, id: u16| l.on_state(&[STATE_READY, (id >> 8) as u8, id as u8], Instant::now());
        ready(&mut l, 10);
        // Maj encore tenue : on attend qu'elle soit relâchée (0,6 s au plus).
        assert!(l.sends_due(Instant::now(), true).is_empty());
        assert_eq!(l.sends_due(Instant::now(), false), vec![(10, "<de> merci pour le groupe, bonne soirée".to_string())]);
        // Rendu une seule fois.
        assert!(l.sends_due(Instant::now(), false).is_empty());
        ready(&mut l, 11);
        assert_eq!(l.sends_due(Instant::now() + Duration::from_millis(700), true).len(), 1, "Maj tenue trop longtemps : collé quand même");
        // Annulé dans le jeu (le joueur reprend la frappe) : jamais collé.
        l.on_state(&[STATE_CANCEL, 0, 12], Instant::now());
        ready(&mut l, 12);
        assert!(l.sends_due(Instant::now(), false).is_empty());
        // Trop tard : abandonné.
        ready(&mut l, 13);
        assert!(l.sends_due(Instant::now() + SEND_DEADLINE + Duration::from_millis(10), false).is_empty());
        assert!(l.sends.get(&13).is_none());
        // Déjà dans la langue visée : le texte d'origine est recollé tel quel, sans traduction.
        send(&mut l, 14, "FR", "SAY", "", "merci pour le groupe");
        ready(&mut l, 14);
        assert_eq!(l.sends_due(Instant::now(), false), vec![(14, "merci pour le groupe".to_string())]);
        assert_eq!(log.borrow().len(), 6);
    }

    #[test]
    fn drafts_make_the_single_press_instant() {
        let (mut l, log) = live("fr");
        let draft = |l: &mut Live, req: u16, text: &str| {
            l.on_send_request(req, &format!("b\u{1F}0\u{1F}DE\u{1F}SAY\u{1F}\u{1F}{text}"));
        };
        // Brouillons traduits pendant la frappe : le dernier sert à l'envoi, sans recalcul.
        draft(&mut l, 20, "merci pour");
        draft(&mut l, 21, "merci pour le groupe");
        l.poll_translations();
        assert_eq!(log.borrow().iter().filter(|e| !e.3).count(), 2, "un brouillon ne passe pas pour un envoi");
        l.on_send_request(22, "s\u{1F}0\u{1F}DE\u{1F}SAY\u{1F}\u{1F}merci pour le groupe");
        assert_eq!(log.borrow().len(), 2, "brouillon déjà traduit : rien à refaire");
        l.on_state(&[STATE_READY, 0, 22], Instant::now());
        let due = l.sends_due(Instant::now(), false);
        assert_eq!(due, vec![(22, "<de> merci pour le groupe".to_string())]);
        // Le message revient dans le chat : affiché avec le texte d'origine du joueur.
        l.on_fragment(FRAME_MLINE, &frag(30, 0, "FFFFFF\u{1F}16:25:53 [Hazè] dit : <de> merci pour le groupe"), Instant::now());
        chat(&mut l, 3, 0x80 | 1, "Hazè-Hyjal", "<de> merci pour le groupe");
        assert_eq!(l.mirror.as_ref().unwrap().shown(30).unwrap(), "16:25:53 [Hazè] dit : [DE] merci pour le groupe");
        assert_eq!(log.borrow().len(), 2, "un envoi traduit qui revient n'est pas retraduit");
        // Envoi pendant que le brouillon du même texte est encore en traduction : il l'attend.
        draft(&mut l, 40, "à demain");
        l.on_send_request(41, "s\u{1F}0\u{1F}DE\u{1F}SAY\u{1F}\u{1F}à demain");
        assert_eq!(log.borrow().len(), 3);
        l.on_state(&[STATE_READY, 0, 41], Instant::now());
        l.poll_translations();
        assert_eq!(l.sends_due(Instant::now(), false), vec![(41, "<de> à demain".to_string())]);
    }

    #[test]
    fn mirror_lines_quote_relayed_messages_by_number() {
        let now = Instant::now();
        let mut b = HashMap::new();
        b.insert(0x2A, ("WTS boost ce soir".to_string(), now));
        assert_eq!(resolve_cites("[4. Services] [Ana]: \u{1D}002A\u{1D} !", &b).as_deref(), Some("[4. Services] [Ana]: WTS boost ce soir !"));
        assert_eq!(resolve_cites("sans citation", &b).as_deref(), Some("sans citation"));
        // Message cité jamais arrivé : la ligne attend son renvoi complet.
        assert_eq!(resolve_cites("x \u{1D}0001\u{1D}", &b), None);
        assert_eq!(resolve_cites("x \u{1D}00", &b), None);
        // Dans le flux : le message relayé, puis la ligne qui le cite.
        let (mut l, _) = live_opt("fr", false);
        l.on_fragment(FRAME_CHAT, &frag(0x2A, 66, "Ana-Hyjal\u{1F}WTS boost ce soir"), now);
        l.on_fragment(FRAME_MLINE, &frag(9, 1, "FFFFFF\u{1F}[2. Commerce] [Ana]: \u{1D}002A\u{1D}"), now);
        assert_eq!(l.mirror.as_ref().unwrap().raw_line(9).as_deref(), Some("[2. Commerce] [Ana]: WTS boost ce soir"));
        l.on_fragment(FRAME_MLINE, &frag(10, 1, "FFFFFF\u{1F}[Bob]: \u{1D}0B0B\u{1D}"), now);
        assert!(l.mirror.as_ref().unwrap().raw_line(10).is_none());
        assert_eq!(l.cites_lost, 1);
    }

    #[test]
    fn own_messages_and_auto_follow_the_channel() {
        let (mut l, log) = live("fr");
        // Le joueur écrit lui-même en anglais : traduit vers le français, avec l'acronyme.
        l.on_fragment(FRAME_MLINE, &frag(5, 0, "FFFFFF\u{1F}[Hazè] dit : Hello everyone, anyone up for a dungeon run?"), Instant::now());
        chat(&mut l, 1, 0x80 | 1, "Hazè-Hyjal", "Hello everyone, anyone up for a dungeon run?");
        l.poll_translations();
        assert_eq!(log.borrow()[0].1, "en");
        assert_eq!(l.mirror.as_ref().unwrap().shown(5).unwrap(), "[Hazè] dit : [EN] <fr> Hello everyone, anyone up for a dungeon run?");
        // « salut » : français pour un client français, jamais traduit.
        chat(&mut l, 2, 0x80 | 1, "Hazè-Hyjal", "salut");
        assert_eq!(log.borrow().len(), 1);
        // Canal surtout français : Trad:AUTO n'y traduit pas ; une phrase anglaise du joueur compte.
        let now = Instant::now();
        for (i, m) in ["bonjour à tous", "quelqu'un pour le donjon ?", "on part dans 5 minutes"].iter().enumerate() {
            chat(&mut l, 10 + i as u16, 14, &format!("Joueur{i}"), m);
        }
        let tr = l.translation().unwrap();
        assert_eq!(tr.target("AUTO", "CHANNEL", "", now), Some("fr"));
        // Sans rien vu sur un canal : l'anglais.
        assert_eq!(tr.target("AUTO", "PARTY", "", now), Some("en"));
        // Chaque canal numéroté a sa langue (addon 0.12 : 64 + numéro) : 1. Général en français,
        // 3. Commerce en anglais.
        chat(&mut l, 20, 64 + 1, "Pierre", "quelqu'un pour aider avec la quête du marais ?");
        for (i, m) in ["Hello need craft, anyone around?", "selling flasks, whisper me for the price"].iter().enumerate() {
            chat(&mut l, 21 + i as u16, 64 + 3, &format!("Trader{i}"), m);
        }
        let tr = l.translation().unwrap();
        assert_eq!(tr.target("AUTO", "CHANNEL", "3", now), Some("en"));
        assert_eq!(tr.target("AUTO", "CHANNEL", "1", now), Some("fr"));
    }
}
