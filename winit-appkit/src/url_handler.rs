//! Delivery of the custom URL schemes an application declares.
//!
//! When the user opens a `myapp://` URL, the system sends the application the `kAEGetURL` Apple
//! Event. The familiar way to read that is `application:openURLs:` on the `NSApplicationDelegate`,
//! but winit deliberately never registers an application delegate so that the slot stays free for
//! the application itself (see the crate documentation). Registering a handler on
//! `NSAppleEventManager` reads the very same event without taking anything the application may
//! want, which is why the URL arrives through
//! [`ApplicationHandlerExtMacOS::received_url`][winit_core::application::macos::ApplicationHandlerExtMacOS::received_url]
//! instead.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSObject, NSObjectProtocol, NSString,
};

use super::app_state::AppState;

/// `'GURL'`, the event class the system uses for the Internet suite.
///
/// `kInternetEventClass` and `kAEGetURL` are declared in `<CoreServices/InternetConfig.h>`, which
/// `objc2-core-services` does not bind, so the four-char codes are spelled out here. They are
/// `AEEventClass` / `AEEventID`, both aliases of `FourCharCode`, itself a `u32`.
const K_INTERNET_EVENT_CLASS: u32 = u32::from_be_bytes(*b"GURL");
/// `'GURL'`, the event id for "open this URL". Same code as the class, which is not a typo.
const K_AE_GET_URL: u32 = u32::from_be_bytes(*b"GURL");
/// `'----'`, the keyword under which an Apple Event carries its direct parameter.
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    /// Receives `kAEGetURL` from `NSAppleEventManager` and forwards it to the application handler.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinitUrlHandler"]
    #[derive(Debug)]
    pub(crate) struct UrlHandler;

    unsafe impl NSObjectProtocol for UrlHandler {}

    impl UrlHandler {
        #[unsafe(method(handleUrl:withReplyEvent:))]
        fn handle_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let Some(url) = parse_url(event) else { return };

            let mtm = MainThreadMarker::from(self);
            AppState::get(mtm).maybe_queue_with_handler(move |app, event_loop| {
                if let Some(handler) = app.macos_handler() {
                    handler.received_url(event_loop, url);
                }
            });
        }
    }
);

impl UrlHandler {
    /// Registers a handler for `kAEGetURL` and returns it.
    ///
    /// `NSAppleEventManager` does NOT retain its handlers, so the caller has to keep the returned
    /// value alive for as long as URLs should be delivered. The Apple Event that launched the
    /// application is delivered shortly after `NSApplication` starts running, so registering any
    /// time before that is early enough.
    pub(crate) fn register(mtm: MainThreadMarker) -> Retained<Self> {
        let this: Retained<Self> =
            unsafe { msg_send![super(Self::alloc(mtm).set_ivars(())), init] };

        let manager = NSAppleEventManager::sharedAppleEventManager();
        // SAFETY: `this` responds to `handleUrl:withReplyEvent:` with the two descriptor arguments
        // the Apple Event manager passes, and the selector is spelled by `sel!`.
        unsafe {
            manager.setEventHandler_andSelector_forEventClass_andEventID(
                &this,
                sel!(handleUrl:withReplyEvent:),
                K_INTERNET_EVENT_CLASS,
                K_AE_GET_URL,
            );
        }

        this
    }
}

/// Reads the URL out of a `kAEGetURL` event, ignoring anything else that reaches the selector.
fn parse_url(event: &NSAppleEventDescriptor) -> Option<String> {
    if event.eventClass() != K_INTERNET_EVENT_CLASS || event.eventID() != K_AE_GET_URL {
        return None;
    }

    let parameter: Retained<NSAppleEventDescriptor> =
        event.paramDescriptorForKeyword(KEY_DIRECT_OBJECT)?;
    let value: Retained<NSString> = parameter.stringValue()?;

    Some(value.to_string())
}
