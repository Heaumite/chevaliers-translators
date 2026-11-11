//! Icône de l'application près de l'horloge (Windows) : état, fenêtre des modèles, journal,
//! Quitter ; bulles d'information (addon installé, à recharger).
//!
//! La fenêtre cachée qui reçoit les clics vit sur le fil principal : ses messages passent par la
//! même pompe que les fenêtres du chat miroir (`Overlay::pump`), ou par `Tray::pump` quand la
//! traduction ne tourne pas (WoW fermé).

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetSystemMetrics, MessageBoxW, PeekMessageW, PostMessageW,
    RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, TrackPopupMenu,
    TranslateMessage, HICON, ICONINFO, IDOK, IDYES, MB_DEFBUTTON2, MB_YESNOCANCEL, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION,
    MB_OK, MB_OKCANCEL, MB_SETFOREGROUND, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PM_REMOVE, SM_CXSMICON,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW,
    WS_OVERLAPPED,
};

pub const TITLE: &str = "Chevaliers Translators";

const WM_TRAY: u32 = WM_APP + 1;
/// Numéro du groupe d'icônes de l'exécutable (build.rs).
const ICON_ID: usize = 1;
const CMD_INTERFACE: usize = 1;
const CMD_JOURNAL: usize = 2;
const CMD_QUIT: usize = 3;
const CMD_UNINSTALL: usize = 4;

/// Quitter confirmé dans le menu.
static QUIT: AtomicBool = AtomicBool::new(false);
/// Désinstallation demandée (0.14.3) : 0 non, 1 l'addon seul, 2 l'addon et les données.
static UNINSTALL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// Message envoyé par Windows quand la barre des tâches renaît (explorateur relancé) : l'icône
/// doit être reposée.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Ligne d'état montrée en tête du menu.
static STATUS: Mutex<String> = Mutex::new(String::new());
/// Journal en cours, ouvert par le menu.
static JOURNAL: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    /// Description de l'icône, reprise si la barre des tâches renaît.
    static NID: RefCell<Option<NOTIFYICONDATAW>> = const { RefCell::new(None) };
    /// Fenêtre des modèles ouverte depuis le menu (une seule à la fois).
    static UI: RefCell<Option<std::process::Child>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copie tronquée dans un tableau de taille fixe, terminée par un zéro.
fn fill(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    pub fn new(status: &str) -> Result<Tray, String> {
        let class = wide("ChevaliersTranslatorsIcone");
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(null()),
                hIcon: null_mut(),
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
                hIconSm: null_mut(),
            };
            if RegisterClassExW(&wc) == 0 && GetLastError() != 1410 {
                return Err("RegisterClassExW a échoué (icône)".into());
            }
            let title = wide(TITLE);
            // Fenêtre jamais montrée (pas de fenêtre « message seulement » : le menu de l'icône
            // ne se refermerait pas d'un clic ailleurs).
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW a échoué (icône)".into());
            }
            TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);
            let icon = make_icon();
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = icon;
            NID.with(|n| *n.borrow_mut() = Some(nid));
            let tray = Tray { hwnd, icon };
            tray.set_status(status);
            // Barre des tâches pas encore prête (ouverture de session) : l'icône sera posée à sa
            // naissance (message TaskbarCreated).
            let ok = NID.with(|n| n.borrow().as_ref().map(|nid| Shell_NotifyIconW(NIM_ADD, nid) != 0)).unwrap_or(false);
            if !ok {
                crate::say!("[icône] barre des tâches indisponible : icône posée dès qu'elle sera prête");
            }
            Ok(tray)
        }
    }

    /// État : infobulle de l'icône et première ligne du menu.
    pub fn set_status(&self, status: &str) {
        *STATUS.lock().unwrap() = status.to_string();
        NID.with(|n| {
            if let Some(nid) = n.borrow_mut().as_mut() {
                nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                fill(&mut nid.szTip, &format!("{TITLE}\n{status}"));
                unsafe {
                    Shell_NotifyIconW(NIM_MODIFY, nid);
                }
            }
        });
    }

    /// Bulle d'information (notification de Windows) attachée à l'icône.
    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        crate::say!("[icône] {title} : {text}");
        NID.with(|n| {
            if let Some(nid) = n.borrow_mut().as_mut() {
                let mut b = *nid;
                b.uFlags = NIF_INFO;
                fill(&mut b.szInfoTitle, title);
                fill(&mut b.szInfo, text);
                b.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
                unsafe {
                    Shell_NotifyIconW(NIM_MODIFY, &b);
                }
            }
        });
    }

    /// Quitter demandé (et confirmé) dans le menu.
    pub fn quit_asked() -> bool {
        QUIT.load(Ordering::Relaxed)
    }

    /// Désinstallation confirmée dans le menu : `Some(true)` avec les données, `Some(false)`
    /// l'addon seul.
    pub fn uninstall_asked() -> Option<bool> {
        match UNINSTALL.load(Ordering::Relaxed) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    pub fn set_journal(path: PathBuf) {
        *JOURNAL.lock().unwrap() = Some(path);
    }

    /// Messages de l'icône quand la traduction ne tourne pas (sinon : pompe du chat miroir).
    pub fn pump() {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        NID.with(|n| {
            if let Some(nid) = n.borrow_mut().take() {
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &nid);
                }
            }
        });
        unsafe {
            DestroyWindow(self.hwnd);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

/// Boîte de message de l'application (erreur au démarrage, déjà lancée...).
pub fn message(text: &str, error: bool) {
    let (t, c) = (wide(text), wide(TITLE));
    unsafe {
        MessageBoxW(
            null_mut(),
            t.as_ptr(),
            c.as_ptr(),
            MB_OK | MB_SETFOREGROUND | if error { MB_ICONERROR } else { MB_ICONINFORMATION },
        );
    }
}

/// Logo de l'application à la taille des petites icônes de Windows : icône de l'exécutable
/// (ressource 1, écrite par build.rs), sinon l'écu dessiné par le programme.
fn make_icon() -> HICON {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 64);
        let own = windows_sys::Win32::UI::WindowsAndMessaging::LoadImageW(
            GetModuleHandleW(null()),
            ICON_ID as *const u16, // MAKEINTRESOURCE
            windows_sys::Win32::UI::WindowsAndMessaging::IMAGE_ICON,
            size,
            size,
            0,
        );
        if !own.is_null() {
            return own as HICON;
        }
        let px = crate::brand::icon_bgra(size as usize);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = null_mut();
        let color = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return null_mut();
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
        // Masque tout à zéro : la transparence vient du canal alpha.
        let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const _);
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn open_interface() {
    let busy = UI.with(|u| u.borrow_mut().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))));
    if busy {
        return;
    }
    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("interface").spawn()) {
        Ok(c) => UI.with(|u| *u.borrow_mut() = Some(c)),
        Err(e) => message(&format!("Fenêtre des modèles impossible à ouvrir : {e}"), true),
    }
}

fn open_journal() {
    use std::os::windows::process::CommandExt;
    let path = JOURNAL.lock().unwrap().clone();
    let mut cmd = std::process::Command::new("explorer");
    match path {
        // Le dossier des journaux, celui en cours sélectionné.
        Some(p) => cmd.raw_arg(format!("/select,\"{}\"", p.display())),
        None => cmd.arg(crate::fonts::cache_root().join("journal")),
    };
    let _ = cmd.spawn();
}

fn confirm_quit(hwnd: HWND) {
    let text = wide(
        "Quitter Chevaliers Translators ?\n\n\
         La traduction s'arrête. Les messages des joueurs restent cachés dans le chat de WoW \
         jusqu'à /reload ou /chevaliers off.",
    );
    let caption = wide(TITLE);
    let r = unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OKCANCEL | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND)
    };
    if r == IDOK {
        QUIT.store(true, Ordering::Relaxed);
    }
}

/// Désinstallation (0.14.3) : l'addon est retiré de toutes les versions de WoW ; au choix, les
/// modèles et les données de l'application aussi. L'application s'arrête ensuite.
fn confirm_uninstall(hwnd: HWND) {
    let data = crate::fonts::cache_root();
    let size = crate::catalog::go(crate::install::dir_size(&data));
    let text = wide(&format!(
        "Désinstaller Chevaliers Translators ?\n\n\
         L'addon est retiré de toutes les versions de WoW trouvées, puis l'application s'arrête \
         (WoW ouvert : relance-le ensuite).\n\n\
         Supprimer aussi les modèles de traduction, le moteur, les journaux et les réglages de \
         l'addon ({size} dans {}) ?\n\n\
         Oui : tout est supprimé.\nNon : seuls l'addon et sa présence sont retirés.\nAnnuler : rien n'est fait.",
        data.display()
    ));
    let caption = wide(TITLE);
    let r = unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND)
    };
    let mode = match r {
        IDYES => 2,
        windows_sys::Win32::UI::WindowsAndMessaging::IDNO => 1,
        _ => return,
    };
    UNINSTALL.store(mode, Ordering::Relaxed);
    QUIT.store(true, Ordering::Relaxed);
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let head = wide(&format!("{TITLE} {}", env!("CARGO_PKG_VERSION")));
        let status = wide(&STATUS.lock().unwrap().replace('&', "&&"));
        let items = [
            wide("Modèles de traduction..."),
            wide("Journal de l'application"),
            wide("Quitter"),
            wide("Désinstaller..."),
        ];
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, head.as_ptr());
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, status.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, CMD_INTERFACE, items[0].as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_JOURNAL, items[1].as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, CMD_UNINSTALL, items[3].as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, items[2].as_ptr());
        SetMenuDefaultItem(menu, CMD_INTERFACE as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Sans cela, le menu ne se referme pas quand on clique ailleurs.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match cmd as usize {
            CMD_INTERFACE => open_interface(),
            CMD_JOURNAL => open_journal(),
            CMD_QUIT => confirm_quit(hwnd),
            CMD_UNINSTALL => confirm_uninstall(hwnd),
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        let event = (lp & 0xffff) as u32;
        if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
            show_menu(hwnd);
        }
        return 0;
    }
    let reborn = TASKBAR_CREATED.load(Ordering::Relaxed);
    if reborn != 0 && msg == reborn {
        NID.with(|n| {
            if let Some(nid) = n.borrow().as_ref() {
                Shell_NotifyIconW(NIM_ADD, nid);
            }
        });
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
