//! Accès direct à WebView2 (Windows) :
//! - plein écran des vidéos (YouTube, etc.) ;
//! - nouvelles fenêtres : liens en nouvel onglet, vraies popups (connexion Google…) conservées ;
//! - interception des requêtes réseau pour le bouclier, iframes comprises.

use std::{cell::RefCell, rc::Rc};

use tao::event_loop::EventLoopProxy;
use webview2_com::{
    take_pwstr, ContainsFullScreenElementChangedEventHandler, Microsoft::Web::WebView2::Win32::*,
    NewWindowRequestedEventHandler, WebResourceRequestedEventHandler,
};
use windows::{
    core::{w, IUnknown, Interface, PWSTR},
    Win32::{Foundation::BOOL, System::Com::IStream},
};
use wry::{WebView, WebViewExtWindows};

use crate::{shield::AdState, UserEvent};

pub fn attach(
    webview: &WebView,
    tab: u32,
    private: bool,
    ad: Rc<RefCell<AdState>>,
    proxy: EventLoopProxy<UserEvent>,
) {
    if let Err(e) = unsafe { attach_inner(webview, tab, private, ad, proxy) } {
        eprintln!("Bouclier indisponible pour cet onglet : {e}");
    }
}

fn request_type(ctx: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> &'static str {
    if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT {
        "script"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE {
        "image"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET {
        "stylesheet"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA {
        "media"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT {
        "font"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST
        || ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH
    {
        "xmlhttprequest"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET {
        "websocket"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING {
        "ping"
    } else if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MANIFEST {
        "web_manifest"
    } else {
        "other"
    }
}

unsafe fn attach_inner(
    webview: &WebView,
    tab: u32,
    private: bool,
    ad: Rc<RefCell<AdState>>,
    proxy: EventLoopProxy<UserEvent>,
) -> windows::core::Result<()> {
    let core = webview.controller().CoreWebView2()?;

    // --- Nouvelles fenêtres.
    // Une popup demandée avec une taille ou une position (connexion Google, Apple, PayPal…)
    // doit rester une vraie popup : elle renvoie le résultat à la page qui l'a ouverte, puis se ferme.
    // Les autres (liens « nouvel onglet ») s'ouvrent dans un onglet CyberX.
    let p_nw = proxy.clone();
    let nw_handler = NewWindowRequestedEventHandler::create(Box::new(
        move |_sender: Option<ICoreWebView2>, args: Option<ICoreWebView2NewWindowRequestedEventArgs>| unsafe {
            let Some(args) = args else { return Ok(()) };
            let features = args.WindowFeatures()?;
            let (mut has_size, mut has_pos) = (BOOL::default(), BOOL::default());
            features.HasSize(&mut has_size)?;
            features.HasPosition(&mut has_pos)?;
            if has_size.as_bool() || has_pos.as_bool() {
                return Ok(()); // WebView2 ouvre la popup normalement, liée à la page.
            }
            let mut uri = PWSTR::null();
            args.Uri(&mut uri)?;
            let url = take_pwstr(uri);
            args.SetHandled(true)?;
            let _ = p_nw.send_event(UserEvent::NewWindow { url, private });
            Ok(())
        },
    ));
    let mut nw_token = Default::default();
    core.add_NewWindowRequested(&nw_handler, &mut nw_token)?;

    // --- Plein écran : quand une vidéo passe en plein écran, la fenêtre aussi.
    let p_fs = proxy.clone();
    let fs_handler = ContainsFullScreenElementChangedEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>, _args: Option<IUnknown>| unsafe {
            if let Some(sender) = sender {
                let mut full = BOOL::default();
                sender.ContainsFullScreenElement(&mut full)?;
                let _ = p_fs.send_event(UserEvent::Fullscreen(tab, full.as_bool()));
            }
            Ok(())
        },
    ));
    let mut fs_token = Default::default();
    core.add_ContainsFullScreenElementChanged(&fs_handler, &mut fs_token)?;

    // --- Bouclier : intercepter toutes les requêtes, y compris celles des iframes
    // (là où se cachent beaucoup de pubs). Ancienne version de WebView2 : page principale seulement.
    let env = core.cast::<ICoreWebView2_2>()?.Environment()?;
    match core.cast::<ICoreWebView2_22>() {
        Ok(core22) => core22.AddWebResourceRequestedFilterWithRequestSourceKinds(
            w!("*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        )?,
        Err(_) => core.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?,
    }

    let handler = WebResourceRequestedEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>, args: Option<ICoreWebView2WebResourceRequestedEventArgs>| unsafe {
            let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };

            let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
            args.ResourceContext(&mut ctx)?;
            // On ne bloque jamais la page principale elle-même.
            if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT {
                return Ok(());
            }

            let mut uri = PWSTR::null();
            args.Request()?.Uri(&mut uri)?;
            let uri = take_pwstr(uri);

            let mut src = PWSTR::null();
            sender.Source(&mut src)?;
            let page = take_pwstr(src);

            if ad.borrow().should_block(&uri, &page, request_type(ctx)) {
                let response =
                    env.CreateWebResourceResponse(None::<&IStream>, 403, w!("Blocked by CyberX"), w!(""))?;
                args.SetResponse(&response)?;
                let _ = proxy.send_event(UserEvent::Blocked(tab));
            }
            Ok(())
        },
    ));

    let mut token = Default::default();
    core.add_WebResourceRequested(&handler, &mut token)?;
    Ok(())
}

/// Place une webview au premier plan (ex. la liste de suggestions, par-dessus la page).
pub fn bring_to_front(webview: &WebView) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
    unsafe {
        let mut hwnd = HWND::default();
        if webview.controller().ParentWindow(&mut hwnd).is_ok() {
            let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

/// Bordure de la fenêtre : pas de liseré de couleur Windows 11 (notre interface a le sien),
/// et pas de coins arrondis en plein écran.
pub fn style_window_frame(window: &tao::window::Window, fullscreen: bool) {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_DONOTROUND,
    };
    let hwnd = HWND(window.hwnd() as _);
    unsafe {
        // DWMWA_COLOR_NONE : aucune bordure colorée.
        let none: u32 = 0xFFFF_FFFE;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const u32 as *const _, 4);
        let corner = if fullscreen { DWMWCP_DONOTROUND } else { DWMWCP_DEFAULT };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
    }
}
