//! Traduction par rafales : le modèle génère quelques jetons, s'arrête un instant pour laisser
//! la carte graphique à WoW, puis reprend là où il en était (le serveur garde le calcul déjà
//! fait : seule la suite est calculée). Les messages longs sont découpés en phrases. Le joueur
//! ne voit qu'un message traduit ; la carte, elle, n'est jamais occupée longtemps d'affilée.
//!
//! Idée du 09/11/2026, après des réinitialisations du pilote graphique sous charge continue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::bench::http;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bursts {
    /// Jetons générés par rafale.
    pub tokens: u32,
    /// Pause entre deux rafales.
    pub pause_ms: u64,
}

impl Bursts {
    /// Réglage de départ du mode doux sur la carte graphique.
    pub fn gentle() -> Bursts {
        Bursts { tokens: 16, pause_ms: 30 }
    }
    pub fn label(&self) -> String {
        format!("rafales de {} jetons, pauses de {} ms", self.tokens, self.pause_ms)
    }
}

/// Découpe un message long en morceaux d'environ `max` caractères, aux fins de phrase
/// (sinon aux virgules, sinon aux espaces). Un message court reste entier.
pub fn split_sentences(text: &str, max: usize) -> Vec<String> {
    let t = text.trim();
    if t.chars().count() <= max {
        return vec![t.to_string()];
    }
    // Phrases : coupure après . ! ? … suivis d'une espace.
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = t.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        cur.push(*c);
        let end = matches!(c, '.' | '!' | '?' | '…' | '。');
        if end && chars.get(i + 1).is_some_and(|n| n.is_whitespace()) {
            sentences.push(cur.trim().to_string());
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur.trim().to_string());
    }
    // Phrase encore trop longue : aux virgules, puis aux espaces.
    let mut pieces: Vec<String> = Vec::new();
    for s in sentences {
        if s.chars().count() <= max {
            pieces.push(s);
            continue;
        }
        let mut part = String::new();
        for w in s.split_inclusive([',', ';', ' ']) {
            if part.chars().count() + w.chars().count() > max && !part.trim().is_empty() {
                pieces.push(part.trim().to_string());
                part.clear();
            }
            part.push_str(w);
        }
        if !part.trim().is_empty() {
            pieces.push(part.trim().to_string());
        }
    }
    // Regroupe les petits morceaux voisins tant que la limite le permet.
    let mut out: Vec<String> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(last) if last.chars().count() + 1 + p.chars().count() <= max => {
                last.push(' ');
                last.push_str(&p);
            }
            _ => out.push(p),
        }
    }
    out
}

/// Consigne brute pour /completion, à partir de la requête construite pour le modèle :
/// telle quelle pour /completion ; pour une discussion, le serveur applique le gabarit du
/// modèle (/apply-template). Renvoie (consigne, paramètres de génération).
pub fn raw_prompt(port: u16, path: &str, body: &Value) -> Result<(String, Value), String> {
    let mut params = json!({ "temperature": 0.0, "cache_prompt": true });
    if let Some(stop) = body.get("stop") {
        params["stop"] = stop.clone();
    }
    if path == "/completion" {
        let p = body["prompt"].as_str().ok_or("consigne absente")?;
        return Ok((p.to_string(), params));
    }
    let (code, b) = http(
        port,
        "POST",
        "/apply-template",
        Some(&json!({ "messages": body["messages"] })),
        Duration::from_secs(30),
    )?;
    if code != 200 {
        return Err(format!("gabarit indisponible (HTTP {code})"));
    }
    let v: Value = serde_json::from_str(&b).map_err(|e| e.to_string())?;
    let p = v["prompt"].as_str().ok_or("gabarit sans consigne")?;
    Ok((p.to_string(), params))
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub text: String,
    pub tokens: u64,
    pub bursts: u32,
    /// Temps passé à générer (hors pauses), en secondes.
    pub busy: f64,
}

/// La rafale s'est-elle arrêtée sur la limite de jetons (le texte continue) ?
fn hit_limit(v: &Value, asked: u32, got: u64) -> bool {
    if let Some(t) = v["stop_type"].as_str() {
        return t == "limit";
    }
    if let Some(b) = v["stopped_limit"].as_bool() {
        return b;
    }
    got >= asked as u64
}

/// Génère la suite de `prompt` par rafales. `on_burst(début, fin)` est appelé après chaque
/// rafale ; `busy` est vrai pendant chaque rafale ; `bursts` peut changer entre deux rafales
/// (régulateur), il est relu à chaque fois.
pub fn generate(
    port: u16,
    prompt: &str,
    params: &Value,
    max_tokens: u32,
    bursts: &dyn Fn() -> Bursts,
    busy: Option<&AtomicBool>,
    on_burst: &mut dyn FnMut(Instant, Instant),
) -> Result<Outcome, String> {
    let mut out = Outcome::default();
    // Reprise exacte : la consigne en texte (le serveur la découpe comme la première fois),
    // suivie des numéros des jetons déjà générés. Renvoyer le texte généré le faisait
    // redécouper autrement à la jonction : 16 à 21 jetons recalculés à chaque reprise pour
    // EuroLLM et TranslateGemma (09/11/2026), 1 seul pour SalamandraTA.
    let mut ids: Vec<Value> = Vec::new();
    let mut by_ids = true;
    loop {
        let b = bursts();
        let want = b.tokens.max(1).min(max_tokens.saturating_sub(out.tokens as u32).max(1));
        let mut body = params.clone();
        body["prompt"] = if by_ids && !ids.is_empty() {
            let mut a = vec![Value::String(prompt.to_string())];
            a.extend(ids.iter().cloned());
            Value::Array(a)
        } else {
            Value::String(format!("{prompt}{}", out.text))
        };
        body["n_predict"] = json!(want);
        body["cache_prompt"] = json!(true);
        body["return_tokens"] = json!(true);
        let t = Instant::now();
        if let Some(f) = busy {
            f.store(true, Ordering::Relaxed);
        }
        let r = http(port, "POST", "/completion", Some(&body), Duration::from_secs(300));
        if let Some(f) = busy {
            f.store(false, Ordering::Relaxed);
        }
        let end = Instant::now();
        on_burst(t, end);
        out.busy += end.duration_since(t).as_secs_f64();
        out.bursts += 1;
        let (code, b2) = r?;
        if code != 200 {
            return Err(format!("HTTP {code} : {}", b2.chars().take(160).collect::<String>()));
        }
        let v: Value = serde_json::from_str(&b2).map_err(|e| format!("réponse illisible : {e}"))?;
        let piece = v["content"].as_str().unwrap_or("");
        let got = v["tokens_predicted"]
            .as_u64()
            .or_else(|| v["timings"]["predicted_n"].as_u64())
            .unwrap_or(0);
        out.text.push_str(piece);
        out.tokens += got;
        match v["tokens"].as_array() {
            Some(t) if !t.is_empty() => ids.extend(t.iter().cloned()),
            // Serveur qui ne renvoie pas les numéros : reprise par le texte.
            _ if got > 0 => by_ids = false,
            _ => {}
        }
        if !hit_limit(&v, want, got) || got == 0 || out.tokens >= max_tokens as u64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(bursts().pause_ms));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn long_messages_are_split_at_sentences() {
        assert_eq!(split_sentences("LFM MC, need heals", 120), vec!["LFM MC, need heals"]);
        let long = "Bonsoir à tous ! La guilde Les Chevaliers recrute pour le raid de jeudi. \
                    Nous cherchons deux soigneurs et un tank, stuff correct exigé. Écrivez-moi en privé.";
        let p = split_sentences(long, 80);
        assert_eq!(
            p,
            vec![
                "Bonsoir à tous ! La guilde Les Chevaliers recrute pour le raid de jeudi.",
                "Nous cherchons deux soigneurs et un tank, stuff correct exigé.",
                "Écrivez-moi en privé."
            ]
        );
        // Phrase sans ponctuation finale trop longue : coupée aux virgules ou aux espaces.
        let run = "wts lot of herbs, mageroyal briarthorn bruiseweed, stranglekelp swiftthistle, cheap prices pst";
        let p = split_sentences(run, 40);
        assert!(p.iter().all(|x| x.chars().count() <= 40), "{p:?}");
        assert_eq!(p.join(" ").replace("  ", " "), run);
    }

    /// Faux serveur : renvoie les mots d'une phrase par paquets de `n_predict` (numéro de jeton
    /// = rang du mot), en vérifiant que chaque reprise donne exactement les jetons déjà générés.
    #[test]
    fn bursts_resume_where_they_stopped() {
        let words: Vec<&str> = "Hallo zusammen , wer kommt mit in die Todesminen ?".split(' ').collect();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = words.join(" ");
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                let mut buf = vec![0u8; 65536];
                let mut n = 0;
                loop {
                    let k = s.read(&mut buf[n..]).unwrap();
                    n += k;
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len: usize = text
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if n >= i + 4 + len || k == 0 {
                            let body: Value = serde_json::from_str(&text[i + 4..]).unwrap();
                            // Première rafale : texte ; reprises : texte puis numéros de jetons.
                            let have = match &body["prompt"] {
                                Value::String(p) => {
                                    assert_eq!(p, "PROMPT>", "la reprise doit passer par les numéros de jetons");
                                    0
                                }
                                Value::Array(a) => {
                                    assert_eq!(a[0], "PROMPT>");
                                    let got: Vec<u64> = a[1..].iter().map(|x| x.as_u64().unwrap()).collect();
                                    assert_eq!(got, (0..got.len() as u64).collect::<Vec<_>>());
                                    got.len()
                                }
                                _ => panic!("consigne inattendue"),
                            };
                            assert_eq!(body["return_tokens"], true);
                            let k = body["n_predict"].as_u64().unwrap() as usize;
                            let next: Vec<&str> = words.iter().skip(have).take(k).copied().collect();
                            let piece = if have == 0 { next.join(" ") } else { format!(" {}", next.join(" ")) };
                            let limit = have + next.len() < words.len();
                            let ids: Vec<usize> = (have..have + next.len()).collect();
                            let v = json!({ "content": piece, "tokens": ids, "tokens_predicted": next.len(), "stop_type": if limit { "limit" } else { "eos" } });
                            let b = v.to_string();
                            write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).unwrap();
                            break;
                        }
                    }
                }
            }
        });
        let mut windows = 0;
        let flag = AtomicBool::new(false);
        let out = generate(
            port,
            "PROMPT>",
            &json!({ "temperature": 0.0 }),
            200,
            &|| Bursts { tokens: 4, pause_ms: 1 },
            Some(&flag),
            &mut |_, _| windows += 1,
        )
        .unwrap();
        assert_eq!(out.text, expected);
        assert_eq!((out.tokens, out.bursts, windows), (10, 3, 3));
        assert!(!flag.load(Ordering::Relaxed));
    }
}
