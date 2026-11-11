//! Lancement du serveur de traduction (llama.cpp) et « mode doux » sur la carte graphique.
//!
//! Les plantages du 09/11/2026 (pilote graphique réinitialisé, codes 141 et 117) viennent du
//! partage du temps de calcul de la carte entre WoW et la traduction, pas de la mémoire.
//! Le mode doux rend la traduction plus discrète, sur toutes les marques de carte :
//!   - priorité basse de la carte graphique pour le serveur (réglage de Windows, comme
//!     Process Lasso) : WoW passe devant ;
//!   - travail envoyé à la carte en petits paquets (GGML_VK_MAX_NODES_PER_SUBMIT, 100 par
//!     défaut dans llama.cpp) et lots de calcul réduits (-ub) : la carte revient plus souvent
//!     à WoW ;
//!   - en option : file graphique au lieu de la file de calcul à part
//!     (GGML_VK_ALLOW_GRAPHICS_QUEUE), et une partie seulement du modèle sur la carte (-ngl).

use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::catalog::Place;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPriority {
    /// Priorité normale de Windows (comme WoW).
    Normal,
    /// Sous la normale : WoW passe devant.
    Low,
    /// La plus basse : la traduction n'utilise la carte que lorsque rien d'autre ne la demande.
    Idle,
}

impl GpuPriority {
    pub fn parse(s: &str) -> Option<GpuPriority> {
        Some(match s {
            "normale" | "normal" => GpuPriority::Normal,
            "basse" | "low" => GpuPriority::Low,
            "minimale" | "idle" => GpuPriority::Idle,
            _ => return None,
        })
    }
    pub fn code(self) -> &'static str {
        match self {
            GpuPriority::Normal => "normale",
            GpuPriority::Low => "basse",
            GpuPriority::Idle => "minimale",
        }
    }
    /// Valeur de D3DKMT_SCHEDULINGPRIORITYCLASS (0 IDLE, 1 BELOW_NORMAL, 2 NORMAL).
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn class(self) -> i32 {
        match self {
            GpuPriority::Idle => 0,
            GpuPriority::Low => 1,
            GpuPriority::Normal => 2,
        }
    }
}

/// Réglages du serveur sur la carte graphique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tuning {
    pub priority: GpuPriority,
    /// Opérations par envoi à la carte (llama.cpp : 100 par défaut).
    pub nodes_per_submit: Option<u32>,
    /// Taille des lots de calcul (-ub ; llama.cpp : 512 par défaut).
    pub ubatch: Option<u32>,
    /// File graphique (partagée avec le jeu) au lieu de la file de calcul à part.
    pub graphics_queue: bool,
    /// Couches du modèle sur la carte (None : toutes).
    pub gpu_layers: Option<u32>,
}

impl Tuning {
    /// Mode doux, appliqué par défaut sur la carte graphique.
    pub fn gentle() -> Tuning {
        Tuning {
            priority: GpuPriority::Low,
            nodes_per_submit: Some(10),
            ubatch: Some(64),
            graphics_queue: false,
            gpu_layers: None,
        }
    }

    /// Réglages d'origine de llama.cpp (ceux des plantages du 09/11/2026).
    pub fn raw() -> Tuning {
        Tuning {
            priority: GpuPriority::Normal,
            nodes_per_submit: None,
            ubatch: None,
            graphics_queue: false,
            gpu_layers: None,
        }
    }

    pub fn label(&self) -> String {
        format!(
            "priorité {} sur la carte, {}, {}, {}, {}",
            self.priority.code(),
            self.nodes_per_submit.map_or("paquets d'origine (100)".into(), |n| format!("paquets de {n} opérations")),
            self.ubatch.map_or("lots d'origine (512)".into(), |u| format!("lots de {u}")),
            if self.graphics_queue { "file graphique" } else { "file de calcul à part" },
            self.gpu_layers.map_or("tout le modèle sur la carte".into(), |n| format!("{n} couches sur la carte"))
        )
    }

    /// Forme courte, pour les fichiers de résultats.
    pub fn code(&self) -> String {
        format!(
            "prio={};paquets={};lots={};file={};couches={}",
            self.priority.code(),
            self.nodes_per_submit.map_or("100".into(), |n| n.to_string()),
            self.ubatch.map_or("512".into(), |u| u.to_string()),
            if self.graphics_queue { "graphique" } else { "calcul" },
            self.gpu_layers.map_or("toutes".into(), |n| n.to_string())
        )
    }
}

pub struct Launch<'a> {
    pub exe: &'a Path,
    pub model: &'a Path,
    pub port: u16,
    pub place: Place,
    /// Fils de calcul sur le processeur.
    pub threads: usize,
    pub tuning: &'a Tuning,
}

/// Emplacements de conversation du serveur, et contexte de chacun.
pub const SLOTS: u32 = 4;
pub const CONTEXT_PER_SLOT: u32 = 512;

/// Modèle à attention à fenêtre glissante (famille Gemma).
pub fn sliding_window(model: &Path) -> bool {
    model.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("gemma"))
}

/// Arguments de llama-server.
pub fn args(l: &Launch) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        l.model.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        l.port.to_string(),
        // 4 emplacements de 512 jetons (un message de chat y tient largement) : chacun garde la
        // consigne de sa paire de langues, que le message suivant de la même paire réutilise
        // au lieu de la relire (TranslateGemma : 75 de ses 93 jetons de consigne).
        "-c".into(),
        (CONTEXT_PER_SLOT * SLOTS).to_string(),
        "-np".into(),
        SLOTS.to_string(),
        "-t".into(),
        l.threads.to_string(),
    ];
    // Modèles Gemma (fenêtre glissante) : cache complet, sinon le serveur recalcule ~21 jetons
    // à chaque reprise de rafale (journaux du 09/11/2026 : 0,25 s par reprise pour le 4B).
    if sliding_window(l.model) {
        a.push("--swa-full".into());
    }
    match l.place {
        Place::Cpu => {
            // Aucun calcul sur la carte, pas même les gros lots que llama.cpp y envoie sinon.
            a.extend(["-ngl", "0", "--device", "none"].map(String::from));
        }
        Place::Gpu => {
            a.push("-ngl".into());
            a.push(l.tuning.gpu_layers.map_or("999".into(), |n| n.to_string()));
            if let Some(u) = l.tuning.ubatch {
                a.extend(["-ub".into(), u.to_string(), "-b".into(), u.max(256).to_string()]);
            }
        }
    }
    a
}

/// Variables d'environnement du serveur (moteur Vulkan).
pub fn envs(l: &Launch) -> Vec<(&'static str, String)> {
    let mut e = Vec::new();
    if l.place == Place::Gpu {
        if let Some(n) = l.tuning.nodes_per_submit {
            e.push(("GGML_VK_MAX_NODES_PER_SUBMIT", n.to_string()));
        }
        if l.tuning.graphics_queue {
            e.push(("GGML_VK_ALLOW_GRAPHICS_QUEUE", "1".into()));
        }
    }
    e
}

/// Lance le serveur. La priorité sur la carte se règle ensuite, modèle chargé
/// (`apply_priority`) : avant, Windows ne connaît pas encore le processus côté carte graphique
/// (essai du 09/11/2026 : code 0xc000000d, paramètre invalide, juste après le lancement).
pub fn spawn(l: &Launch, log: std::fs::File) -> Result<Child, String> {
    let mut cmd = Command::new(l.exe);
    crate::quiet(&mut cmd).args(args(l));
    for (k, v) in envs(l) {
        cmd.env(k, v);
    }
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| format!("lancement de {} : {e}", l.exe.display()))?;
    tie_to_app(&child);
    Ok(child)
}

/// Serveur rattaché à un « job » Windows fermé avec l'application : il s'arrête avec elle, même
/// fermée brutalement (gestionnaire des tâches, plantage, Ctrl+C), au lieu de garder la mémoire
/// vidéo du modèle.
#[cfg(windows)]
pub fn tie_to_app(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // Poignée gardée ouverte jusqu'à la fin de l'application (adresse rangée en entier).
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        h as usize
    });
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as windows_sys::Win32::Foundation::HANDLE, child.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
pub fn tie_to_app(_child: &Child) {}

/// Règle la priorité du serveur sur la carte (modèle chargé) ; texte pour le joueur.
pub fn apply_priority(pid: u32, place: Place, t: &Tuning) -> String {
    if place != Place::Gpu || t.priority == GpuPriority::Normal {
        return "normale".into();
    }
    match set_gpu_priority(pid, t.priority) {
        Ok(()) => format!("{} (appliquée)", t.priority.code()),
        Err(e) => format!("{} non appliquée ({e})", t.priority.code()),
    }
}

/// Priorité d'un processus sur la carte graphique (Windows, toutes marques de carte).
pub fn set_gpu_priority(pid: u32, p: GpuPriority) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION,
        };
        type SetFn = unsafe extern "system" fn(*mut std::ffi::c_void, i32) -> i32;
        type GetFn = unsafe extern "system" fn(*mut std::ffi::c_void, *mut i32) -> i32;
        unsafe {
            let name: Vec<u16> = "gdi32.dll".encode_utf16().chain(std::iter::once(0)).collect();
            let lib = LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
            if lib.is_null() {
                return Err("gdi32.dll introuvable".into());
            }
            let set = GetProcAddress(lib, c"D3DKMTSetProcessSchedulingPriorityClass".as_ptr() as *const u8)
                .ok_or("fonction de priorité absente")?;
            let get = GetProcAddress(lib, c"D3DKMTGetProcessSchedulingPriorityClass".as_ptr() as *const u8);
            let set: SetFn = std::mem::transmute(set);
            let h = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION, 0, pid);
            if h.is_null() {
                return Err("accès au processus refusé".into());
            }
            let st = set(h, p.class());
            let mut now = -1;
            if let Some(g) = get {
                let g: GetFn = std::mem::transmute(g);
                g(h, &mut now);
            }
            CloseHandle(h);
            if st != 0 {
                return Err(format!("code 0x{:08x}", st as u32));
            }
            if now >= 0 && now != p.class() {
                return Err(format!("Windows a gardé la classe {now}"));
            }
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, p);
        Err("réglage propre à Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gentle_mode_arguments() {
        let t = Tuning::gentle();
        let (exe, model) = (Path::new("llama-server"), Path::new("m.gguf"));
        let l = Launch { exe, model, port: 8089, place: Place::Gpu, threads: 8, tuning: &t };
        let a = args(&l).join(" ");
        assert_eq!(a, "-m m.gguf --host 127.0.0.1 --port 8089 -c 2048 -np 4 -t 8 -ngl 999 -ub 64 -b 256");
        assert_eq!(envs(&l), vec![("GGML_VK_MAX_NODES_PER_SUBMIT", "10".to_string())]);
        assert_eq!(t.code(), "prio=basse;paquets=10;lots=64;file=calcul;couches=toutes");
        // Réglages d'origine : rien d'ajouté.
        let raw = Tuning::raw();
        let l = Launch { tuning: &raw, ..l };
        assert!(args(&l).join(" ").ends_with("-ngl 999") && envs(&l).is_empty());
        // File graphique et 20 couches.
        let t = Tuning { graphics_queue: true, gpu_layers: Some(20), ..Tuning::gentle() };
        let l = Launch { tuning: &t, ..l };
        assert!(args(&l).join(" ").contains("-ngl 20 -ub 64"));
        assert!(envs(&l).contains(&("GGML_VK_ALLOW_GRAPHICS_QUEUE", "1".to_string())));
        // Processeur : aucun réglage de carte, aucun calcul envoyé à la carte.
        let l = Launch { place: Place::Cpu, threads: 4, ..l };
        assert!(args(&l).join(" ").ends_with("-t 4 -ngl 0 --device none"));
        assert!(envs(&l).is_empty());
        assert_eq!(GpuPriority::parse("minimale").unwrap().class(), 0);
        // TranslateGemma : cache complet de la fenêtre glissante.
        let gemma = Path::new("C:\\m\\translategemma-4b-it.Q4_K_M.gguf");
        let l = Launch { model: gemma, place: Place::Gpu, ..l };
        assert!(args(&l).join(" ").contains("-t 4 --swa-full -ngl "));
    }
}
