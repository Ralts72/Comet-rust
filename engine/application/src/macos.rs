use anyhow::{Context, Result, ensure};
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSApplication;

define_class!(
    // SAFETY: AppKit subclasses and calls this object only on the main thread.
    #[unsafe(super(NSApplication))]
    #[thread_kind = MainThreadOnly]
    struct CometApplication;

    impl CometApplication {
        #[unsafe(method(terminate:))]
        fn request_termination(&self, _sender: Option<&AnyObject>) {
            // Route menu, Cmd-Q and Dock Quit through winit's existing window delegate.
            // Its CloseRequested event lets the editor confirm instead of terminating AppKit.
            for window in self.windows() {
                window.performClose(None);
            }
        }
    }
);

pub(super) fn prepare() -> Result<()> {
    let _main = objc2::MainThreadMarker::new().context("AppKit requires the main thread")?;
    // SAFETY: sharedApplication is inherited from NSApplication; calling it before winit creates
    // the process application preserves winit's delegate and event dispatch on our subclass.
    let app: Retained<NSApplication> =
        unsafe { msg_send![CometApplication::class(), sharedApplication] };
    ensure!(
        app.isKindOfClass(CometApplication::class()),
        "AppKit application was initialized before the Comet host"
    );
    Ok(())
}
