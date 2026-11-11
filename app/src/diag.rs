//! Diagnostic d'un plantage graphique : journaux de Windows et rapports d'erreur de WoW.
//!
//! Le 09/11/2026, les plantages avec un modèle chargé pendant le jeu étaient des
//! réinitialisations du pilote graphique (TDR) : Windows note un « LiveKernelEvent » de code
//! 141 (un moteur de la carte n'a pas répondu à temps) ou 117 (le pilote a cessé de répondre),
//! avec un vidage WATCHDOG-AAAAMMJJ-HHMM.dmp. Ces codes sont les mêmes pour toutes les marques.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// « AAAA-MM-JJ HH:MM:SS », heure locale.
    pub time: String,
    pub provider: String,
    pub id: u32,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Réinitialisation du pilote graphique (LiveKernelEvent 141, 117, ou Display 4101).
    DriverReset { code: String, dump: Option<String> },
    /// Rapport du pilote lui-même (AMD : a2000002...).
    DriverReport { code: String },
    /// Programme arrêté par une erreur (WoW, serveur de traduction).
    AppCrash { app: String },
    Other,
}

/// Valeur d'une ligne « P1 : 141 » (étiquettes traduites selon la langue de Windows).
fn param(msg: &str, name: &str) -> Option<String> {
    msg.lines().find_map(|l| {
        let t = l.trim();
        let rest = t.strip_prefix(name)?;
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return None; // P10 n'est pas P1
        }
        let v = rest.trim_start().strip_prefix(':')?.trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// Nom du vidage WATCHDOG-AAAAMMJJ-HHMM cité dans le message, s'il y en a un.
fn dump_name(msg: &str) -> Option<String> {
    let i = msg.find("WATCHDOG-")?;
    let s: String = msg[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    (s.len() >= "WATCHDOG-20261109-0247".len()).then_some(s)
}

pub fn classify(e: &Event) -> Kind {
    let m = &e.message;
    if e.provider.contains("Display") && e.id == 4101 {
        return Kind::DriverReset { code: "4101".into(), dump: None };
    }
    if m.contains("LiveKernelEvent") {
        let code = param(m, "P1").unwrap_or_default().to_lowercase();
        return match code.as_str() {
            "141" | "117" | "1a8" | "1a1" => Kind::DriverReset { code, dump: dump_name(m) },
            _ => Kind::DriverReport { code },
        };
    }
    if e.provider.contains("Application Error") || e.provider.contains("Application Hang") {
        let low = m.to_lowercase();
        for app in ["llama-server.exe", "wow"] {
            if let Some(i) = low.find(app) {
                let name: String = m[i..].chars().take_while(|c| !c.is_whitespace() && *c != ',').collect();
                return Kind::AppCrash { app: name };
            }
        }
    }
    Kind::Other
}

/// Réinitialisations distinctes (un vidage WATCHDOG par réinitialisation ; un même vidage
/// peut être cité par plusieurs rapports).
pub fn driver_resets(events: &[Event]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for e in events {
        if let Kind::DriverReset { code, dump } = classify(e) {
            let key = dump.unwrap_or_else(|| format!("{} {code}", e.time));
            if !out.iter().any(|(k, _)| *k == key) {
                out.push((key, code));
            }
        }
    }
    out
}

/// Script PowerShell : événements des journaux Système et Application depuis `since`
/// (secondes depuis 1970), en JSON UTF-8.
fn script(since: u64) -> String {
    format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
         $t=[DateTimeOffset]::FromUnixTimeSeconds({since}).LocalDateTime;\
         $s=@(Get-WinEvent -FilterHashtable @{{LogName='System';StartTime=$t}} -ErrorAction SilentlyContinue | Where-Object {{ $_.ProviderName -match 'Display|amdkmdag|amdwddmg|nvlddmkm|igfx|dxgkrnl' }});\
         $a=@(Get-WinEvent -FilterHashtable @{{LogName='Application';StartTime=$t}} -ErrorAction SilentlyContinue | Where-Object {{ ($_.ProviderName -match 'Windows Error Reporting|Application Error|Application Hang') -and ($_.Message -match 'LiveKernelEvent|Wow|llama-server') }});\
         $e=@(@($s+$a) | Sort-Object TimeCreated | ForEach-Object {{ [pscustomobject]@{{ t=$_.TimeCreated.ToString('yyyy-MM-dd HH:mm:ss'); p=$_.ProviderName; id=$_.Id; m=[string]$_.Message }} }});\
         ConvertTo-Json -Depth 4 -Compress -InputObject @{{ debut=$t.ToString('yyyy-MM-dd HH:mm'); ev=$e }}"
    )
}

/// Heure locale d'un vidage d'après son nom (« …-20261109-0247.dmp » : « 2026-11-09 02:47 »).
pub fn dump_time(msg: &str) -> Option<String> {
    let b = msg.as_bytes();
    if b.len() < 18 {
        return None;
    }
    (0..=b.len() - 18).find_map(|i| {
        let w = &b[i..i + 18];
        let ok = w[0] == b'-'
            && w[1..9].iter().all(u8::is_ascii_digit)
            && w[9] == b'-'
            && w[10..14].iter().all(u8::is_ascii_digit)
            && &w[14..18] == b".dmp";
        ok.then(|| {
            let d = std::str::from_utf8(w).unwrap_or_default();
            format!("{}-{}-{} {}:{}", &d[1..5], &d[5..7], &d[7..9], &d[10..12], &d[12..14])
        })
    })
}

/// Lit la sortie du script : { debut, ev } (ou une liste seule). Windows renvoie parfois plus
/// tard des rapports en attente : une réinitialisation dont le vidage date d'avant `debut`
/// est ancienne, elle est écartée (09/11/2026 : le vidage de 02:47 signalé à nouveau à 05:20).
pub fn parse_json(text: &str) -> Vec<Event> {
    let t = text.trim().trim_start_matches('\u{feff}');
    if t.is_empty() {
        return Vec::new();
    }
    let v: serde_json::Value = match serde_json::from_str(t) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let since = v.get("debut").and_then(|d| d.as_str()).map(str::to_string);
    let list = if since.is_some() || v.get("ev").is_some() { v["ev"].clone() } else { v };
    let items = match list {
        serde_json::Value::Array(a) => a,
        o @ serde_json::Value::Object(_) => vec![o],
        _ => Vec::new(),
    };
    items
        .iter()
        .map(|i| Event {
            time: i["t"].as_str().unwrap_or_default().to_string(),
            provider: i["p"].as_str().unwrap_or_default().to_string(),
            id: i["id"].as_u64().unwrap_or(0) as u32,
            message: i["m"].as_str().unwrap_or_default().to_string(),
        })
        .filter(|e| match (&since, dump_time(&e.message)) {
            (Some(s), Some(d)) => d >= *s,
            _ => true,
        })
        .collect()
}

/// Événements graphiques et plantages depuis `since` (Windows ; ailleurs : aucun).
pub fn events_since(since: u64) -> Result<Vec<Event>, String> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    let out = crate::quiet(&mut std::process::Command::new("powershell"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script(since)])
        .output()
        .map_err(|e| format!("PowerShell : {e}"))?;
    Ok(parse_json(&String::from_utf8_lossy(&out.stdout)))
}

/// Rapports d'erreur de WoW (dossier Errors) modifiés depuis `since`, avec les lignes utiles.
pub fn wow_reports(errors_dir: &Path, since: u64) -> Vec<(PathBuf, Vec<String>)> {
    const KEYS: &[&str] = &[
        "out of memory",
        "outofmemory",
        "0x8007000e",
        "device_hung",
        "device_removed",
        "device_reset",
        "0x887a0005",
        "0x887a0006",
        "0x887a0007",
        "error #",
        "exception:",
    ];
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(errors_dir) else { return out };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
        .filter(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| d.as_secs() + 60 >= since)
        })
        .collect();
    files.sort();
    for f in files {
        let text = String::from_utf8_lossy(&std::fs::read(&f).unwrap_or_default()).into_owned();
        let lines: Vec<String> = text
            .lines()
            .filter(|l| {
                let l = l.to_lowercase();
                KEYS.iter().any(|k| l.contains(k))
            })
            .take(6)
            .map(|l| l.trim().chars().take(160).collect())
            .collect();
        out.push((f, lines));
    }
    out
}

/// Résumé lisible pour le joueur.
pub fn report(events: &[Event], wow: &[(PathBuf, Vec<String>)]) -> String {
    let mut s = String::new();
    let resets = driver_resets(events);
    if resets.is_empty() {
        s.push_str("Aucune réinitialisation du pilote graphique dans les journaux de Windows.\n");
    } else {
        s.push_str(&format!(
            "{} réinitialisation(s) du pilote graphique (la carte n'a pas répondu à temps ; Windows l'a relancée) :\n",
            resets.len()
        ));
        for (k, code) in &resets {
            let what = match code.as_str() {
                "141" => "un moteur de la carte bloqué",
                "117" => "le pilote a cessé de répondre",
                "4101" => "pilote d'affichage relancé",
                _ => "réinitialisation",
            };
            s.push_str(&format!("  - {k} : code {code}, {what}\n"));
        }
        s.push_str("  Ce n'est pas un manque de mémoire : la carte était partagée en temps de calcul entre le jeu et la traduction.\n");
    }
    for e in events {
        match classify(e) {
            Kind::DriverReport { code } => {
                s.push_str(&format!("Rapport du pilote {} à {} (code {code}).\n", e.provider, e.time))
            }
            Kind::AppCrash { app } => s.push_str(&format!("{app} arrêté par une erreur à {}.\n", e.time)),
            _ => {}
        }
    }
    for (f, lines) in wow {
        s.push_str(&format!(
            "Rapport de plantage de WoW : {}\n",
            f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        for l in lines {
            s.push_str(&format!("    {l}\n"));
        }
    }
    s
}

/// Commande `diagnostic` : dernières heures.
pub fn run(wow_errors: Option<PathBuf>, hours: u64) -> Result<bool, String> {
    let since = crate::catalog::now_secs().saturating_sub(hours * 3600);
    if !cfg!(windows) {
        crate::say!("Diagnostic : lecture des journaux de Windows uniquement sous Windows.");
        return Ok(true);
    }
    let events = events_since(since)?;
    let wow = wow_errors.map(|d| wow_reports(&d, since)).unwrap_or_default();
    crate::say!("Diagnostic des {hours} dernières heures\n");
    print!("{}", report(&events, &wow));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Messages réels du 09/11/2026 (Windows en français), sérialisés comme le fait le script.
    const SAMPLE: &str = r#"[{"t":"2026-11-09 02:22:15","p":"Windows Error Reporting","id":1001,"m":"Détecteur d'erreurs , type 0\r\nNom d’événement: LiveKernelEvent\r\nRéponse : Non disponible\r\n\r\nSignature du problème :\r\nP1 : 117\r\nP2 : aaaaaaaaaaaaaaaa\r\nP10 : \r\n\r\nFichiers joints :\r\n\\\\?\\C:\\WINDOWS\\LiveKernelReports\\WATCHDOG\\WATCHDOG-20261109-0222.dmp\r\n"},
{"t":"2026-11-09 02:22:18","p":"Windows Error Reporting","id":1001,"m":"Détecteur d'erreurs LKD_0x141_Tdr:6_IMAGE_amdkmdag.sys-TOS-CP, type 0\r\nNom d’événement: LiveKernelEvent\r\nP1 : 141\r\nFichiers joints :\r\n\\\\?\\C:\\WINDOWS\\LiveKernelReports\\WATCHDOG\\WATCHDOG-20261109-0221.dmp\r\n"},
{"t":"2026-11-09 02:22:47","p":"Windows Error Reporting","id":1001,"m":"Nom d’événement: LiveKernelEvent\r\nP1 : 117\r\n\\\\?\\C:\\WINDOWS\\LiveKernelReports\\WATCHDOG\\WATCHDOG-20261109-0222.dmp\r\n"},
{"t":"2026-11-09 02:46:22","p":"Windows Error Reporting","id":1001,"m":"Détecteur d'erreurs LKD_0xA2000002_amdfendr!AmdlogDrv, type 0\r\nNom d’événement: LiveKernelEvent\r\nP1 : a2000002\r\n\\\\?\\C:\\WINDOWS\\LiveKernelReports\\AMD_REPORT_UM\\AMD_REPORT_UM-20261109-0245.dmp\r\n"},
{"t":"2026-11-09 02:47:41","p":"Application Error","id":1000,"m":"Nom de l’application défaillante WowClassic.exe, version : 1.60.1.70862"}]"#;

    #[test]
    fn real_driver_resets_are_recognised() {
        let ev = parse_json(SAMPLE);
        assert_eq!(ev.len(), 5);
        let r = driver_resets(&ev);
        assert_eq!(
            r,
            vec![
                ("WATCHDOG-20261109-0222".to_string(), "117".to_string()),
                ("WATCHDOG-20261109-0221".to_string(), "141".to_string())
            ]
        );
        assert_eq!(classify(&ev[3]), Kind::DriverReport { code: "a2000002".into() });
        assert_eq!(classify(&ev[4]), Kind::AppCrash { app: "WowClassic.exe".into() });
        let text = report(&ev, &[]);
        assert!(text.contains("2 réinitialisation(s)") && text.contains("pas un manque de mémoire"), "{text}");
        assert!(text.contains("WowClassic.exe arrêté"));
        // Un seul événement : PowerShell sort un objet, pas une liste.
        let one = r#"{"t":"2026-11-09 02:22:15","p":"Windows Error Reporting","id":1001,"m":"LiveKernelEvent\nP1 : 141\nP10 : x"}"#;
        assert_eq!(driver_resets(&parse_json(one)).len(), 1);
        assert!(parse_json("").is_empty());
        // Rapport en attente renvoyé à 05:20 pour le vidage de 02:47 : ancien, écarté.
        let late = format!(r#"{{"debut":"2026-11-09 04:26","ev":{SAMPLE}}}"#);
        assert!(driver_resets(&parse_json(&late)).is_empty());
        let fresh = format!(r#"{{"debut":"2026-11-09 02:20","ev":{SAMPLE}}}"#);
        assert_eq!(driver_resets(&parse_json(&fresh)).len(), 2);
        assert_eq!(dump_time(r"\\?\C:\WINDOWS\LiveKernelReports\WATCHDOG\WATCHDOG-20261109-0247.dmp"), Some("2026-11-09 02:47".into()));
    }

    #[test]
    fn wow_crash_reports_are_summarised() {
        let dir = std::env::temp_dir().join(format!("ct-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("2026-11-09_02.47.40_Crash_11108.txt"),
            "==== \nERROR #134 (0x85100086) Fatal condition\nProgram: WowClassic.exe\nDXGI_ERROR_DEVICE_HUNG\nautre ligne\n",
        )
        .unwrap();
        let r = wow_reports(&dir, 0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, vec!["ERROR #134 (0x85100086) Fatal condition", "DXGI_ERROR_DEVICE_HUNG"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
