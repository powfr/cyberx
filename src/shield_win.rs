//! Accès direct à WebView2 (Windows) :
//! - plein écran des vidéos (YouTube, etc.) ;
//! - interception des requêtes réseau pour le bouclier, iframes comprises.

use std::{cell::RefCell, rc::Rc};

use tao::event_loop::EventLoopProxy;
use webview2_com::{
    take_pwstr, ContainsFullScreenElementChangedEventHandler, Microsoft::Web::WebView2::Win32::*,
    WebResourceRequestedEventHandler,
};
use windows::{
    core::{w, IUnknown, Interface, PWSTR},
    Win32::{Foundation::BOOL, System::Com::IStream},
};
use wry::{WebView, WebViewExtWindows};

use crate::{shield::AdState, UserEvent};

pub fn attach(webview: &WebView, tab: u32, ad: Rc<RefCell<AdState>>, proxy: EventLoopProxy<UserEvent>) {
    if let Err(e) = unsafe { attach_inner(webview, tab, ad, proxy) } {
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
    ad: Rc<RefCell<AdState>>,
    proxy: EventLoopProxy<UserEvent>,
) -> windows::core::Result<()> {
    let core = webview.controller().CoreWebView2()?;

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
