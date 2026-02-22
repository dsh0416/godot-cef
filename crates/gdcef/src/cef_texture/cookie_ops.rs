use crate::browser::{App, EventQueuesHandle};
use godot::prelude::*;

fn with_cookie_manager(
    app: &App,
    f: impl FnOnce(&EventQueuesHandle, cef::CookieManager) -> bool,
) -> bool {
    let Some(state) = app.state.as_ref() else {
        return false;
    };
    use cef::ImplBrowserHost;
    let Some(host) = app.host() else {
        return false;
    };
    let Some(ctx) = host.request_context() else {
        return false;
    };
    use cef::ImplRequestContext;
    let Some(manager) = ctx.cookie_manager(None) else {
        return false;
    };
    f(&state.event_queues, manager)
}

pub(crate) fn get_all_cookies(app: &App) -> bool {
    with_cookie_manager(app, |eq, manager| {
        use cef::ImplCookieManager;
        let mut visitor = crate::cookie::CookieVisitorImpl::build(eq.clone());
        manager.visit_all_cookies(Some(&mut visitor)) != 0
    })
}

pub(crate) fn get_cookies(app: &App, url: GString, include_http_only: bool) -> bool {
    with_cookie_manager(app, |eq, manager| {
        use cef::ImplCookieManager;
        let url_cef = cef::CefStringUtf16::from(url.to_string().as_str());
        let mut visitor = crate::cookie::CookieVisitorImpl::build(eq.clone());
        manager.visit_url_cookies(Some(&url_cef), include_http_only as _, Some(&mut visitor)) != 0
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn set_cookie(
    app: &App,
    url: GString,
    name: GString,
    value: GString,
    domain: GString,
    path: GString,
    secure: bool,
    httponly: bool,
) -> bool {
    with_cookie_manager(app, |eq, manager| {
        use cef::ImplCookieManager;
        let url_cef = cef::CefStringUtf16::from(url.to_string().as_str());
        let cookie = cef::Cookie {
            size: std::mem::size_of::<cef::Cookie>(),
            name: cef::CefStringUtf16::from(name.to_string().as_str()),
            value: cef::CefStringUtf16::from(value.to_string().as_str()),
            domain: cef::CefStringUtf16::from(domain.to_string().as_str()),
            path: cef::CefStringUtf16::from(path.to_string().as_str()),
            secure: secure as _,
            httponly: httponly as _,
            ..Default::default()
        };
        let mut callback = crate::cookie::SetCookieCallbackImpl::build(eq.clone());
        manager.set_cookie(Some(&url_cef), Some(&cookie), Some(&mut callback)) != 0
    })
}

pub(crate) fn delete_cookies(app: &App, url: GString, cookie_name: GString) -> bool {
    with_cookie_manager(app, |eq, manager| {
        use cef::ImplCookieManager;
        let url_str = url.to_string();
        let name_str = cookie_name.to_string();
        let url_opt = if url_str.is_empty() {
            None
        } else {
            Some(cef::CefStringUtf16::from(url_str.as_str()))
        };
        let name_opt = if name_str.is_empty() {
            None
        } else {
            Some(cef::CefStringUtf16::from(name_str.as_str()))
        };
        let mut callback = crate::cookie::DeleteCookiesCallbackImpl::build(eq.clone());
        manager.delete_cookies(url_opt.as_ref(), name_opt.as_ref(), Some(&mut callback)) != 0
    })
}

pub(crate) fn flush_cookies(app: &App) -> bool {
    with_cookie_manager(app, |eq, manager| {
        use cef::ImplCookieManager;
        let mut callback = crate::cookie::FlushCookieStoreCallbackImpl::build(eq.clone());
        manager.flush_store(Some(&mut callback)) != 0
    })
}
