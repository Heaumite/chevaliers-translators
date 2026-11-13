//! Un seul moteur de traduction pour plusieurs jeux ouverts (0.15) : chaque fenêtre de WoW a sa
//! traduction (chat miroir, bulles, envois), et toutes leurs demandes passent dans la même file du
//! serveur llama.cpp (un seul modèle en mémoire vidéo). Les traductions finies reviennent au jeu
//! qui les a demandées ; les messages du moteur (modèle actif, repli) au premier jeu ; une
//! proposition de modèle à tous.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::engine::{Ask, Event, Translator};

pub struct Hub {
    engine: Box<dyn Translator>,
    owner: HashMap<u64, usize>,
    inbox: HashMap<usize, Vec<Event>>,
    members: Vec<usize>,
}

impl Hub {
    pub fn new(engine: Box<dyn Translator>) -> Rc<RefCell<Hub>> {
        Rc::new(RefCell::new(Hub { engine, owner: HashMap::new(), inbox: HashMap::new(), members: Vec::new() }))
    }

    /// Branche un jeu (numéro `game`) : son traducteur.
    pub fn join(hub: &Rc<RefCell<Hub>>, game: usize) -> Box<dyn Translator> {
        let mut h = hub.borrow_mut();
        if !h.members.contains(&game) {
            h.members.push(game);
            h.members.sort();
        }
        Box::new(Client { hub: hub.clone(), game })
    }

    /// Jeu fermé : ses traductions en cours sont oubliées.
    pub fn leave(&mut self, game: usize) {
        self.members.retain(|g| *g != game);
        self.owner.retain(|_, g| *g != game);
        self.inbox.remove(&game);
    }

    pub fn members(&self) -> usize {
        self.members.len()
    }

    /// Répartit ce que le moteur a rendu.
    fn pump(&mut self) {
        for e in self.engine.poll() {
            match e {
                Event::Done { id, .. } => {
                    if let Some(g) = self.owner.remove(&id) {
                        self.inbox.entry(g).or_default().push(e);
                    }
                }
                Event::Status(_) => {
                    if let Some(&g) = self.members.first() {
                        self.inbox.entry(g).or_default().push(e);
                    }
                }
                Event::Propose(p) => {
                    for g in self.members.clone() {
                        self.inbox.entry(g).or_default().push(Event::Propose(p.clone()));
                    }
                }
            }
        }
    }
}

struct Client {
    hub: Rc<RefCell<Hub>>,
    game: usize,
}

impl Translator for Client {
    fn submit(&mut self, ask: Ask) -> u64 {
        let mut h = self.hub.borrow_mut();
        let id = h.engine.submit(ask);
        h.owner.insert(id, self.game);
        id
    }

    fn poll(&mut self) -> Vec<Event> {
        let mut h = self.hub.borrow_mut();
        h.pump();
        h.inbox.remove(&self.game).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Priority;
    use std::collections::VecDeque;

    struct Fake {
        next: u64,
        ready: VecDeque<Event>,
    }

    impl Translator for Fake {
        fn submit(&mut self, a: Ask) -> u64 {
            self.next += 1;
            self.ready.push_back(Event::Done { id: self.next, result: Ok(format!("<{}> {}", a.tgt, a.text)), secs: 0.3, cached: false });
            self.next
        }
        fn poll(&mut self) -> Vec<Event> {
            let mut v: Vec<Event> = self.ready.drain(..).collect();
            v.push(Event::Status("modèle prêt".into()));
            v
        }
    }

    fn ask(text: &str, tgt: &'static str) -> Ask {
        Ask { text: text.into(), src: "de", tgt, priority: Priority::Received, group: None }
    }

    #[test]
    fn two_games_share_one_engine_and_get_their_own_translations_back() {
        let hub = Hub::new(Box::new(Fake { next: 0, ready: VecDeque::new() }));
        let mut a = Hub::join(&hub, 1);
        let mut b = Hub::join(&hub, 2);
        let ia = a.submit(ask("Hallo", "fr"));
        let ib = b.submit(ask("Guten Tag", "en"));
        assert_ne!(ia, ib, "une seule file : numéros uniques");
        // Le jeu 2 relève le moteur en premier : il ne reçoit que sa traduction.
        let got_b = b.poll();
        let done = |v: &[Event]| -> Vec<(u64, String)> {
            v.iter()
                .filter_map(|e| match e {
                    Event::Done { id, result: Ok(t), .. } => Some((*id, t.clone())),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(done(&got_b), vec![(ib, "<en> Guten Tag".to_string())]);
        assert!(!got_b.iter().any(|e| matches!(e, Event::Status(_))), "les messages du moteur vont au premier jeu");
        let got_a = a.poll();
        assert_eq!(done(&got_a), vec![(ia, "<fr> Hallo".to_string())]);
        assert!(got_a.iter().any(|e| matches!(e, Event::Status(_))));
        // Jeu fermé : ses demandes en cours ne reviennent à personne.
        let ia2 = a.submit(ask("Tschüss", "fr"));
        hub.borrow_mut().leave(1);
        assert_eq!(hub.borrow().members(), 1);
        let got_b = b.poll();
        assert!(done(&got_b).iter().all(|(id, _)| *id != ia2));
        assert!(got_b.iter().any(|e| matches!(e, Event::Status(_))), "le jeu 2 devient le premier");
    }
}
