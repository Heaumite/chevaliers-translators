//! Images par seconde du jeu et à-coups, mesurés pendant un test : l'addon les envoie par le
//! bandeau (`/chevaliers perf on`, deux fois par seconde) et cette sonde les lit à l'écran.
//! Sert à vérifier qu'un modèle de traduction ne gêne pas le jeu, et à régler la traduction
//! sur ce que le jeu tolère (plus longue image de chaque demi-seconde).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perf {
    pub fps: f32,
    /// Plus longue image de la demi-seconde, en millisecondes (addon 0.8 et suivants).
    pub worst_ms: Option<f32>,
}

pub struct FpsProbe {
    samples: Arc<Mutex<Vec<(Instant, Perf)>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

/// Trame de performance : images par seconde × 10 sur 2 octets, puis plus longue image en
/// millisecondes sur 2 octets.
pub fn parse_perf(payload: &[u8]) -> Option<Perf> {
    if payload.len() < 2 {
        return None;
    }
    Some(Perf {
        fps: u16::from_be_bytes([payload[0], payload[1]]) as f32 / 10.0,
        worst_ms: (payload.len() >= 4).then(|| u16::from_be_bytes([payload[2], payload[3]]) as f32),
    })
}

impl FpsProbe {
    #[cfg(windows)]
    pub fn start() -> Result<FpsProbe, String> {
        use crate::decode::{decode, Image};
        let samples: Arc<Mutex<Vec<(Instant, Perf)>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, stop2) = (samples.clone(), stop.clone());
        // Le capteur est créé dans le fil de mesure (ressources Windows propres à ce fil).
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let handle = std::thread::spawn(move || {
            let mut cap = match crate::capture_win::Capturer::new(0, 48) {
                Ok(c) => {
                    let _ = tx.send(Ok(()));
                    c
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let mut last: Option<u16> = None;
            while !stop2.load(Ordering::Relaxed) {
                let (w, h) = (cap.w, cap.h);
                if let Ok(buf) = cap.grab() {
                    if let Ok(d) = decode(&Image { w, h, stride: w * 4, bgra: buf }) {
                        if d.frame.ftype == crate::proto::FRAME_PERF && last != Some(d.frame.seq) {
                            last = Some(d.frame.seq);
                            if let Some(f) = parse_perf(&d.frame.payload) {
                                s2.lock().unwrap().push((Instant::now(), f));
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
        });
        rx.recv().map_err(|e| e.to_string())??;
        Ok(FpsProbe {
            samples,
            stop,
            handle: Some(handle),
        })
    }

    #[cfg(not(windows))]
    pub fn start() -> Result<FpsProbe, String> {
        Err("la mesure des images par seconde lit l'écran de Windows".into())
    }

    /// Mesures datées reçues entre deux instants.
    pub fn timed_between(&self, a: Instant, b: Instant) -> Vec<(Instant, Perf)> {
        self.samples.lock().unwrap().iter().filter(|(t, _)| *t >= a && *t <= b).copied().collect()
    }

}

impl Drop for FpsProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_values() {
        assert_eq!(parse_perf(&[0x01, 0xB8]), Some(Perf { fps: 44.0, worst_ms: None }));
        assert_eq!(parse_perf(&[0x01, 0xB8, 0x01, 0x2C]), Some(Perf { fps: 44.0, worst_ms: Some(300.0) }));
        assert_eq!(parse_perf(&[1]), None);
    }
}
