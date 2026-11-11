//! Fenêtre de l'interface : une vraie fenêtre de l'application (titre, icône, fermeture qui
//! rend la main), la page étant affichée par WebView2, le moteur web livré avec Windows 10 et 11.
//! Fermer la fenêtre arrête l'outil, sauf téléchargement en cours : il continue dans la console.

use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;

/// Ouvre la fenêtre et ne revient pas (la boucle des fenêtres tient le programme) ;
/// renvoie une erreur seulement si la fenêtre ou WebView2 ne peut pas être créée.
pub fn show(url: String, busy: impl Fn() -> bool + 'static) -> Result<(), String> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Chevaliers Translators")
        .with_inner_size(LogicalSize::new(1060.0, 860.0))
        .with_min_inner_size(LogicalSize::new(560.0, 520.0))
        .with_window_icon(Icon::from_rgba(crate::ui::window_icon_rgba(64), 64, 64).ok())
        // Barre de titre sombre, comme la page et la bannière.
        .with_theme(Some(tao::window::Theme::Dark))
        .build(&event_loop)
        .map_err(|e| format!("fenêtre : {e}"))?;
    // Données de WebView2 dans le dossier de l'application, pas à côté de l'exécutable.
    let mut context = wry::WebContext::new(Some(crate::fonts::cache_root().join("fenetre")));
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_background_color((14, 13, 10, 255))
        .with_devtools(false);
    #[cfg(windows)]
    let webview = builder.build(&window).map_err(|e| format!("WebView2 : {e}"))?;
    #[cfg(not(windows))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or("fenêtre sans conteneur")?;
        builder.build_gtk(vbox).map_err(|e| format!("WebKitGTK : {e}"))?
    };
    let (mut closing, mut told) = (false, false);
    event_loop.run(move |event, _, control_flow| {
        let _keep = (&webview, &context);
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            closing = true;
            window.set_visible(false);
        }
        if closing {
            if busy() {
                if !told {
                    told = true;
                    crate::say!("Fenêtre fermée : les téléchargements en cours continuent ici (Ctrl+C pour les arrêter ; ils reprendront au prochain lancement).");
                }
            } else {
                crate::say!("Fenêtre fermée.");
                *control_flow = ControlFlow::Exit;
            }
        }
    })
}
