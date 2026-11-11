//! Détection du matériel du joueur (S4a) : processeur, mémoire vive, cartes graphiques,
//! moteurs de calcul disponibles, et WoW en cours d'exécution. Sert à choisir, sur chaque
//! PC, où faire tourner la traduction (carte graphique ou processeur) et quelle taille
//! de modèle elle peut porter sans gêner le jeu.
//!
//! Les seuils sont provisoires : le comparatif des modèles (S4b) les fixera.

#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub name: String,
    pub logical: usize,
    pub physical: Option<usize>,
    pub avx2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub avx512: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub name: String,
    pub vendor_id: u32,
    pub dedicated: u64,
    pub software: bool,
    /// Mémoire vidéo que Windows accorde à l'application en ce moment (DXGI).
    pub budget: Option<u64>,
    /// Identifiant de la carte (LUID : partie haute, partie basse), pour les compteurs Windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub luid: (i32, u32),
    /// Mémoire vidéo dédiée utilisée en ce moment par tous les programmes (compteurs Windows).
    pub used: Option<u64>,
    /// Dont WoW.
    pub used_by_wow: Option<u64>,
}

impl Gpu {
    pub fn vendor(&self) -> &'static str {
        match self.vendor_id {
            0x1002 | 0x1022 => "AMD",
            0x10DE => "NVIDIA",
            0x8086 => "Intel",
            0x1414 => "Microsoft",
            0x5143 => "Qualcomm",
            _ => "autre",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Runtimes {
    pub vulkan: bool,
    pub cuda: bool,
    pub directml: bool,
    pub hip: bool,
    pub opencl: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Hw {
    pub cpu: Cpu,
    pub ram_total: u64,
    pub ram_avail: u64,
    pub gpus: Vec<Gpu>,
    pub runtimes: Runtimes,
    /// Exécutables de WoW en cours (Wow.exe, WowClassic.exe...).
    pub wow: Vec<String>,
}

pub const GIB: u64 = 1 << 30;

pub fn gib(b: u64) -> String {
    format!("{:.1} Go", b as f64 / GIB as f64).replace('.', ",")
}

// --- Processeur (commun à Windows et Linux) ----------------------------------------

#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
fn cpu_features() -> Cpu {
    use std::arch::x86_64::__cpuid;
    let mut name = String::new();
    let max_ext = unsafe { __cpuid(0x8000_0000) }.eax;
    if max_ext >= 0x8000_0004 {
        let mut bytes = Vec::with_capacity(48);
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            let r = unsafe { __cpuid(leaf) };
            for v in [r.eax, r.ebx, r.ecx, r.edx] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        name = String::from_utf8_lossy(&bytes)
            .trim_matches(char::from(0))
            .trim()
            .to_string();
    }
    Cpu {
        name,
        logical: std::thread::available_parallelism().map_or(1, |n| n.get()),
        physical: None,
        avx2: is_x86_feature_detected!("avx2"),
        fma: is_x86_feature_detected!("fma"),
        f16c: is_x86_feature_detected!("f16c"),
        avx512: is_x86_feature_detected!("avx512f"),
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn cpu_features() -> Cpu {
    Cpu {
        name: std::env::consts::ARCH.to_string(),
        logical: std::thread::available_parallelism().map_or(1, |n| n.get()),
        ..Default::default()
    }
}

// --- Windows ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{CloseHandle, FreeLibrary, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, GlobalMemoryStatusEx, RelationProcessorCore,
        MEMORYSTATUSEX, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };

    #[repr(C)]
    struct AdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        subsys_id: u32,
        revision: u32,
        dedicated_video: usize,
        dedicated_system: usize,
        shared_system: usize,
        luid_low: u32,
        luid_high: i32,
        flags: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct VideoMemoryInfo {
        budget: u64,
        current_usage: u64,
        available_for_reservation: u64,
        current_reservation: u64,
    }

    const IID_IDXGIFACTORY1: GUID = GUID {
        data1: 0x770a_ae78,
        data2: 0xf26f,
        data3: 0x4dba,
        data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
    };
    const IID_IDXGIADAPTER3: GUID = GUID {
        data1: 0x6459_67a4,
        data2: 0x1392,
        data3: 0x4310,
        data4: [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd],
    };
    const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Entrée `index` de la table de méthodes d'un objet COM.
    unsafe fn vmethod(obj: *mut c_void, index: usize) -> *const c_void {
        let vtbl = *(obj as *const *const *const c_void);
        *vtbl.add(index)
    }

    unsafe fn release(obj: *mut c_void) {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(vmethod(obj, 2));
        f(obj);
    }

    /// Cartes graphiques par DXGI (dxgi.dll chargée à la demande, sans dépendance de lien).
    pub fn gpus() -> Vec<Gpu> {
        let mut out = Vec::new();
        unsafe {
            let lib = LoadLibraryExW(wide("dxgi.dll").as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
            if lib.is_null() {
                return out;
            }
            let Some(create) = GetProcAddress(lib, c"CreateDXGIFactory1".as_ptr() as *const u8) else {
                FreeLibrary(lib);
                return out;
            };
            let create: unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> i32 =
                std::mem::transmute(create);
            let mut factory: *mut c_void = null_mut();
            if create(&IID_IDXGIFACTORY1, &mut factory) < 0 || factory.is_null() {
                FreeLibrary(lib);
                return out;
            }
            // IDXGIFactory1::EnumAdapters1 (12), IDXGIAdapter1::GetDesc1 (10),
            // IUnknown::QueryInterface (0), IDXGIAdapter3::QueryVideoMemoryInfo (14).
            let enum1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
                std::mem::transmute(vmethod(factory, 12));
            for i in 0..16u32 {
                let mut adapter: *mut c_void = null_mut();
                if enum1(factory, i, &mut adapter) < 0 || adapter.is_null() {
                    break;
                }
                let desc1: unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32 =
                    std::mem::transmute(vmethod(adapter, 10));
                let mut d: AdapterDesc1 = std::mem::zeroed();
                if desc1(adapter, &mut d) >= 0 {
                    let n = d.description.iter().position(|c| *c == 0).unwrap_or(128);
                    let mut g = Gpu {
                        name: String::from_utf16_lossy(&d.description[..n]),
                        vendor_id: d.vendor_id,
                        dedicated: d.dedicated_video as u64,
                        software: d.flags & DXGI_ADAPTER_FLAG_SOFTWARE != 0,
                        budget: None,
                        luid: (d.luid_high, d.luid_low),
                        used: None,
                        used_by_wow: None,
                    };
                    let qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32 =
                        std::mem::transmute(vmethod(adapter, 0));
                    let mut a3: *mut c_void = null_mut();
                    if qi(adapter, &IID_IDXGIADAPTER3, &mut a3) >= 0 && !a3.is_null() {
                        let qvmi: unsafe extern "system" fn(*mut c_void, u32, u32, *mut VideoMemoryInfo) -> i32 =
                            std::mem::transmute(vmethod(a3, 14));
                        let mut info = VideoMemoryInfo::default();
                        if qvmi(a3, 0, 0, &mut info) >= 0 && info.budget > 0 {
                            g.budget = Some(info.budget);
                        }
                        release(a3);
                    }
                    out.push(g);
                }
                release(adapter);
            }
            release(factory);
            FreeLibrary(lib);
        }
        out
    }

    /// Valeurs d'un compteur Windows à instances (« \\GPU Process Memory(*)\\Dedicated Usage »...).
    pub fn pdh_values(path: &str) -> Option<Vec<(String, i64)>> {
        use windows_sys::Win32::System::Performance::{
            PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
            PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_MORE_DATA,
        };
        unsafe {
            let mut q = null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                return None;
            }
            let mut c = null_mut();
            let out = (|| {
                if PdhAddEnglishCounterW(q, wide(path).as_ptr(), 0, &mut c) != 0 {
                    return None;
                }
                if PdhCollectQueryData(q) != 0 {
                    return None;
                }
                let (mut size, mut count) = (0u32, 0u32);
                let r = PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, null_mut());
                if r != PDH_MORE_DATA || size == 0 {
                    return Some(Vec::new());
                }
                let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![std::mem::zeroed(); size as usize / item + 1];
                if PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, buf.as_mut_ptr()) != 0 {
                    return None;
                }
                let mut v = Vec::new();
                for it in &buf[..count as usize] {
                    let mut n = 0;
                    while *it.szName.add(n) != 0 {
                        n += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n));
                    v.push((name, it.FmtValue.Anonymous.largeValue));
                }
                Some(v)
            })();
            PdhCloseQuery(q);
            out
        }
    }

    /// Mémoire vidéo dédiée utilisée par un processus (toutes cartes confondues).
    pub fn gpu_memory_of(pid: u32) -> Option<u64> {
        let prefix = format!("pid_{pid}_");
        let v = pdh_values("\\GPU Process Memory(*)\\Dedicated Usage")?;
        Some(v.iter().filter(|(n, _)| n.starts_with(&prefix)).map(|(_, x)| (*x).max(0) as u64).sum())
    }

    /// Complète chaque carte avec la mémoire vidéo utilisée en ce moment, et la part de WoW.
    pub fn gpu_usage(gpus: &mut [Gpu], wow_pids: &[u32]) {
        let adapters = pdh_values("\\GPU Adapter Memory(*)\\Dedicated Usage");
        let procs = pdh_values("\\GPU Process Memory(*)\\Dedicated Usage");
        for g in gpus.iter_mut() {
            let tag = format!("luid_0x{:08x}_0x{:08x}", g.luid.0 as u32, g.luid.1).to_lowercase();
            if let Some(a) = &adapters {
                let used: i64 = a.iter().filter(|(n, _)| n.to_lowercase().starts_with(&tag)).map(|(_, x)| *x).sum();
                if a.iter().any(|(n, _)| n.to_lowercase().starts_with(&tag)) {
                    g.used = Some(used.max(0) as u64);
                }
            }
            if let (Some(p), false) = (&procs, wow_pids.is_empty()) {
                let w: i64 = p
                    .iter()
                    .filter(|(n, _)| {
                        let n = n.to_lowercase();
                        n.contains(&tag) && wow_pids.iter().any(|pid| n.starts_with(&format!("pid_{pid}_")))
                    })
                    .map(|(_, x)| *x)
                    .sum();
                g.used_by_wow = Some(w.max(0) as u64);
            }
        }
    }

    pub fn memory() -> (u64, u64) {
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut m) == 0 {
                return (0, 0);
            }
            (m.ullTotalPhys, m.ullAvailPhys)
        }
    }

    pub fn physical_cores() -> Option<usize> {
        unsafe {
            let mut len = 0u32;
            GetLogicalProcessorInformation(null_mut(), &mut len);
            let size = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
            if len == 0 {
                return None;
            }
            let mut buf: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> =
                vec![std::mem::zeroed(); len as usize / size + 1];
            if GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) == 0 {
                return None;
            }
            let n = len as usize / size;
            Some(buf[..n].iter().filter(|e| e.Relationship == RelationProcessorCore).count())
        }
    }

    /// Processus de WoW (le jeu lui-même, pas ses outils comme WowVoiceProxy).
    pub fn wow_processes() -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE || snap.is_null() {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e) != 0;
            while ok {
                let n = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..n]);
                let low = name.to_lowercase();
                let tool = ["voice", "proxy", "launcher", "error", "crash"].iter().any(|t| low.contains(t));
                if low.starts_with("wow") && low.ends_with(".exe") && !tool {
                    out.push((name, e.th32ProcessID));
                }
                ok = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
        out
    }
}

/// Bibliothèques des moteurs de calcul présentes dans Windows\System32 (aucune n'est chargée).
fn runtimes() -> Runtimes {
    let sys = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("System32");
    let has = |f: &str| sys.join(f).is_file();
    let in_path = |f: &str| {
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(f).is_file()))
            .unwrap_or(false)
    };
    let windows = cfg!(windows);
    Runtimes {
        vulkan: windows && has("vulkan-1.dll"),
        cuda: windows && has("nvcuda.dll"),
        directml: windows && has("DirectML.dll"),
        hip: windows
            && (has("amdhip64_6.dll") || has("amdhip64.dll") || in_path("amdhip64_6.dll") || in_path("amdhip64.dll")),
        opencl: windows && has("OpenCL.dll"),
    }
}

#[cfg(not(windows))]
fn linux_memory() -> (u64, u64) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let get = |k: &str| {
        text.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |kb| kb * 1024)
    };
    (get("MemTotal:"), get("MemAvailable:"))
}

pub fn detect() -> Hw {
    let mut cpu = cpu_features();
    #[cfg(windows)]
    {
        cpu.physical = win::physical_cores();
        let (t, a) = win::memory();
        let procs = win::wow_processes();
        let wow_pids: Vec<u32> = procs.iter().map(|(_, p)| *p).collect();
        let mut wow: Vec<String> = procs.into_iter().map(|(n, _)| n).collect();
        wow.dedup();
        let mut gpus = win::gpus();
        win::gpu_usage(&mut gpus, &wow_pids);
        Hw {
            cpu,
            ram_total: t,
            ram_avail: a,
            gpus,
            runtimes: runtimes(),
            wow,
        }
    }
    #[cfg(not(windows))]
    {
        cpu.physical = None;
        let (t, a) = linux_memory();
        Hw {
            cpu,
            ram_total: t,
            ram_avail: a,
            gpus: Vec::new(),
            runtimes: runtimes(),
            wow: Vec::new(),
        }
    }
}

/// Processus de WoW en cours (identifiants).
pub fn wow_pids() -> Vec<u32> {
    #[cfg(windows)]
    {
        win::wow_processes().into_iter().map(|(_, p)| p).collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Relevé de mémoire vidéo d'une seconde, pendant un test.
#[derive(Clone, Debug, Default)]
pub struct VramSample {
    /// Utilisée sur la carte, tous programmes compris.
    pub card_used: Option<u64>,
    pub server: Option<u64>,
    pub wow: Option<u64>,
}

/// Relevé léger (deux lectures de compteurs Windows) pour la carte `luid`.
pub fn vram_sample(luid: (i32, u32), server: Option<u32>, wow: &[u32]) -> VramSample {
    #[cfg(windows)]
    {
        let tag = format!("luid_0x{:08x}_0x{:08x}", luid.0 as u32, luid.1).to_lowercase();
        let mut s = VramSample::default();
        if let Some(a) = win::pdh_values("\\GPU Adapter Memory(*)\\Dedicated Usage") {
            let v: Vec<i64> = a.iter().filter(|(n, _)| n.to_lowercase().starts_with(&tag)).map(|(_, x)| *x).collect();
            if !v.is_empty() {
                s.card_used = Some(v.iter().sum::<i64>().max(0) as u64);
            }
        }
        if let Some(p) = win::pdh_values("\\GPU Process Memory(*)\\Dedicated Usage") {
            let of = |pid: u32| -> u64 {
                let prefix = format!("pid_{pid}_");
                p.iter().filter(|(n, _)| n.starts_with(&prefix)).map(|(_, x)| (*x).max(0) as u64).sum()
            };
            s.server = server.map(of);
            if !wow.is_empty() {
                s.wow = Some(wow.iter().map(|p| of(*p)).sum());
            }
        }
        s
    }
    #[cfg(not(windows))]
    {
        let _ = (luid, server, wow);
        VramSample::default()
    }
}

/// Mémoire vidéo dédiée utilisée par un processus (Windows ; ailleurs : inconnue).
pub fn gpu_memory_of(pid: u32) -> Option<u64> {
    #[cfg(windows)]
    {
        win::gpu_memory_of(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

// --- Choix du niveau -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Insuffisant,
    Leger,
    Moyen,
    Eleve,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Insuffisant => "insuffisant",
            Level::Leger => "léger",
            Level::Moyen => "moyen",
            Level::Eleve => "élevé",
        }
    }
    /// Classe de modèle visée (taille du fichier du modèle, quantifié).
    pub fn model_class(self) -> &'static str {
        match self {
            Level::Insuffisant => "aucun : traduction locale déconseillée",
            Level::Leger => "version low : 6 langues (fr, en, es, it, de, ru), modèle de 1,7 à 2 milliards de paramètres (≈ 1 à 1,5 Go)",
            Level::Moyen => "modèle de ≈ 2 milliards de paramètres (≈ 1,5 à 2,5 Go)",
            Level::Eleve => "modèle de 4 à 9 milliards de paramètres (≈ 3 à 5,5 Go)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Path {
    pub level: Level,
    pub on: String,
    pub engine: String,
    pub reason: String,
}

/// Marge laissée au jeu et au système.
const VRAM_MARGIN: u64 = GIB;
const RAM_MARGIN: u64 = 2 * GIB;

/// Voies possibles (carte graphique, processeur), de la meilleure à la moins bonne.
pub fn assess(hw: &Hw) -> Vec<Path> {
    let mut paths = Vec::new();
    for (i, g) in hw.gpus.iter().enumerate() {
        if g.software || !matches!(g.vendor_id, 0x1002 | 0x10DE | 0x8086) {
            continue;
        }
        let engine = if g.vendor_id == 0x10DE && hw.runtimes.cuda {
            "CUDA"
        } else if hw.runtimes.vulkan {
            "Vulkan"
        } else if hw.runtimes.directml {
            "DirectML"
        } else {
            continue;
        };
        // Carte intégrée (peu de mémoire dédiée) : elle partage la mémoire vive, pas d'intérêt.
        if g.dedicated < 2 * GIB {
            continue;
        }
        // Mémoire libre : mesurée (dédiée moins utilisée par tous les programmes) si possible,
        // sinon la part accordée par Windows (qui ne tient pas compte des autres programmes).
        let free = match g.used {
            Some(u) => g.dedicated.saturating_sub(u),
            None => g.budget.unwrap_or(g.dedicated).min(g.dedicated),
        }
        .saturating_sub(VRAM_MARGIN);
        let level = if free >= 6 * GIB {
            Level::Eleve
        } else if free >= 3 * GIB {
            Level::Moyen
        } else if free >= GIB {
            Level::Leger
        } else {
            Level::Insuffisant
        };
        paths.push(Path {
            level,
            on: format!("carte graphique {} ({})", i + 1, g.name),
            engine: engine.into(),
            reason: format!(
                "{} de mémoire vidéo libres en ce moment{} (marge de 1 Go)",
                gib(free),
                if g.used.is_some() { "" } else { " (estimation)" }
            ),
        });
    }
    let cores = hw.cpu.physical.unwrap_or(hw.cpu.logical / 2).max(1);
    let free = hw.ram_avail.saturating_sub(RAM_MARGIN);
    let level = if !hw.cpu.avx2 {
        Level::Insuffisant
    } else if cores >= 8 && free >= 4 * GIB {
        Level::Moyen
    } else if cores >= 4 && free >= 2 * GIB {
        Level::Leger
    } else {
        Level::Insuffisant
    };
    paths.push(Path {
        level,
        on: "processeur".into(),
        engine: if hw.cpu.avx2 { "processeur AVX2".into() } else { "processeur sans AVX2".into() },
        reason: format!(
            "{cores} cœurs, AVX2 {}, {} de mémoire vive utilisables (marge de 2 Go)",
            if hw.cpu.avx2 { "oui" } else { "non" },
            gib(free)
        ),
    });
    // Tri stable : à niveau égal, la carte graphique passe avant le processeur.
    paths.sort_by(|a, b| b.level.cmp(&a.level));
    paths
}

pub fn run() -> bool {
    let hw = detect();
    let yes = |b: bool| if b { "oui" } else { "non" };
    crate::say!("Matériel détecté");
    crate::say!(
        "  Processeur : {} - {} cœurs / {} fils - AVX2 {}, FMA {}, F16C {}, AVX-512 {}",
        if hw.cpu.name.is_empty() { "inconnu" } else { &hw.cpu.name },
        hw.cpu.physical.map_or("?".to_string(), |p| p.to_string()),
        hw.cpu.logical,
        yes(hw.cpu.avx2),
        yes(hw.cpu.fma),
        yes(hw.cpu.f16c),
        yes(hw.cpu.avx512)
    );
    crate::say!("  Mémoire vive : {}, dont {} libres", gib(hw.ram_total), gib(hw.ram_avail));
    if hw.gpus.is_empty() {
        crate::say!("  Cartes graphiques : aucune détectée{}", if cfg!(windows) { "" } else { " (détection sous Windows seulement)" });
    }
    for (i, g) in hw.gpus.iter().enumerate() {
        if g.software {
            crate::say!("  Carte graphique {} : {} - rendu logiciel, ignorée", i + 1, g.name);
            continue;
        }
        crate::say!(
            "  Carte graphique {} : {} ({}) - {} dédiés{}{}",
            i + 1,
            g.name,
            g.vendor(),
            gib(g.dedicated),
            g.used.map_or(String::new(), |u| format!(", {} utilisés en ce moment", gib(u))),
            g.used_by_wow.map_or(String::new(), |w| format!(" dont WoW {}", gib(w)))
        );
    }
    let r = &hw.runtimes;
    crate::say!(
        "  Moteurs de calcul : Vulkan {}, CUDA {}, DirectML {}, HIP (ROCm) {}, OpenCL {}",
        yes(r.vulkan),
        yes(r.cuda),
        yes(r.directml),
        yes(r.hip),
        yes(r.opencl)
    );
    crate::say!(
        "  WoW en cours : {}",
        if hw.wow.is_empty() { "non".to_string() } else { hw.wow.join(", ") }
    );
    crate::say!("");
    let paths = assess(&hw);
    let best = &paths[0];
    crate::say!(
        "Niveau proposé : {} - sur {} ({}), {}",
        best.level.label(),
        best.on,
        best.engine,
        best.level.model_class()
    );
    crate::say!("  Raison : {}", best.reason);
    for p in &paths[1..] {
        crate::say!(
            "  Autre voie : {} - niveau {} ({}) : {}",
            p.on,
            p.level.label(),
            p.engine,
            p.reason
        );
    }
    crate::say!("");
    crate::say!("Seuils provisoires : le comparatif des modèles (S4b) mesurera vitesse, qualité et effet sur les images par seconde.");
    if hw.wow.is_empty() {
        crate::say!("Refaire la mesure WoW lancé, personnage en jeu : la mémoire libre change.");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(cores: usize, avx2: bool, ram_avail: u64, gpu: Option<(u32, u64, Option<u64>)>, vulkan: bool) -> Hw {
        Hw {
            cpu: Cpu {
                name: "test".into(),
                logical: cores * 2,
                physical: Some(cores),
                avx2,
                ..Default::default()
            },
            ram_total: ram_avail * 2,
            ram_avail,
            gpus: gpu
                .map(|(v, d, b)| Gpu {
                    name: "carte".into(),
                    vendor_id: v,
                    dedicated: d,
                    budget: b,
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            runtimes: Runtimes {
                vulkan,
                ..Default::default()
            },
            wow: vec![],
        }
    }

    #[test]
    fn levels() {
        // Grosse carte AMD avec Vulkan : niveau élevé sur la carte.
        let p = assess(&pc(8, true, 20 * GIB, Some((0x1002, 20 * GIB, Some(16 * GIB))), true));
        assert_eq!((p[0].level, p[0].engine.as_str()), (Level::Eleve, "Vulkan"));
        assert!(p[0].on.starts_with("carte graphique 1"));
        // Même carte, mais le jeu occupe presque tout : la carte descend, le processeur reste.
        let p = assess(&pc(8, true, 20 * GIB, Some((0x1002, 8 * GIB, Some(3 * GIB))), true));
        assert_eq!(p[0].level, Level::Moyen);
        // Mémoire utilisée mesurée : elle prime sur la part accordée par Windows.
        let mut h = pc(8, true, 20 * GIB, Some((0x1002, 16 * GIB, Some(15 * GIB))), true);
        h.gpus[0].used = Some(12 * GIB);
        let p = assess(&h);
        // 16 - 12 - 1 = 3 Go sur la carte : niveau moyen, à égalité avec le processeur (carte d'abord).
        assert_eq!((p[0].level, p[1].on.as_str(), p[1].level), (Level::Moyen, "processeur", Level::Moyen));
        assert!(p[0].reason.starts_with("3,0 Go"));
        // Pas de moteur pour la carte : processeur seulement.
        let p = assess(&pc(6, true, 10 * GIB, Some((0x10DE, 12 * GIB, None)), false));
        assert_eq!((p.len(), p[0].on.as_str(), p[0].level), (1, "processeur", Level::Leger));
        // Vieux processeur sans AVX2, pas de carte : insuffisant.
        let p = assess(&pc(4, false, 8 * GIB, None, false));
        assert_eq!(p[0].level, Level::Insuffisant);
        // Carte logicielle et carte intégrée ignorées.
        let mut h = pc(8, true, 4 * GIB, Some((0x8086, GIB / 2, None)), true);
        h.gpus.push(Gpu { name: "Microsoft Basic Render Driver".into(), vendor_id: 0x1414, software: true, ..Default::default() });
        assert_eq!(assess(&h).len(), 1);
    }

    #[test]
    fn detection_runs_here() {
        let hw = detect();
        assert!(hw.cpu.logical >= 1);
        assert!(hw.ram_total > 0);
        assert!(!assess(&hw).is_empty());
        assert_eq!(gib(3 * GIB / 2), "1,5 Go");
    }
}
