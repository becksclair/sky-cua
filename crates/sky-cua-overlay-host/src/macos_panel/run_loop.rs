//! Main run loop integration for the macOS host.
//!
//! The host is single-threaded: the main thread owns AppKit, the backend, and
//! the listener socket. In place of an external event loop it waits in the
//! main `CFRunLoop`, which also services AppKit, and is woken early by a
//! `CFFileDescriptor` source when a client connects.

use std::{ffi::c_void, os::fd::RawFd, time::Duration};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventMask};
use objc2_core_foundation::{
    CFFileDescriptor, CFOptionFlags, CFRetained, CFRunLoop, CFRunLoopRunResult, CFRunLoopSource,
    kCFFileDescriptorReadCallBack, kCFRunLoopCommonModes, kCFRunLoopDefaultMode,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

/// Wakes the main run loop when a file descriptor becomes readable.
pub struct ReadableWake {
    descriptor: CFRetained<CFFileDescriptor>,
    source: CFRetained<CFRunLoopSource>,
}

impl ReadableWake {
    /// Attaches `fd` to the main run loop. `None` off the main thread, or if
    /// CoreFoundation cannot wrap the descriptor. The caller keeps `fd` open
    /// for as long as the returned value lives.
    #[must_use]
    pub fn attach(fd: RawFd) -> Option<Self> {
        MainThreadMarker::new()?;
        // SAFETY: the callback matches the required signature and uses no
        // context, and the descriptor is not closed on invalidation.
        let descriptor = unsafe {
            CFFileDescriptor::new(None, fd, false, Some(readable_callback), std::ptr::null())
        }?;
        let source = CFFileDescriptor::new_run_loop_source(None, Some(&descriptor), 0)?;
        // SAFETY: the common-modes constant is a valid mode name.
        CFRunLoop::main()?.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        Some(Self { descriptor, source })
    }

    /// Runs the main run loop until the descriptor is readable, another
    /// source fires, or `timeout` passes.
    pub fn wait(&self, timeout: Duration) {
        // The callback is one-shot. Arming it while the descriptor is already
        // readable fires it at once, so a connection that arrived while a
        // request was being handled is not missed.
        self.descriptor
            .enable_call_backs(kCFFileDescriptorReadCallBack);
        // SAFETY: the default-mode constant is a valid mode name.
        let result = CFRunLoop::run_in_mode(
            unsafe { kCFRunLoopDefaultMode },
            timeout.as_secs_f64(),
            true,
        );
        if result == CFRunLoopRunResult::Finished {
            // The mode has no sources left, so the run loop returned at once;
            // sleep instead of spinning.
            std::thread::sleep(timeout);
        }
    }
}

impl Drop for ReadableWake {
    fn drop(&mut self) {
        self.source.invalidate();
        self.descriptor.invalidate();
    }
}

/// The wake-up is the whole effect: the run loop returns to its caller, which
/// accepts the pending connections.
unsafe extern "C-unwind" fn readable_callback(
    _descriptor: *mut CFFileDescriptor,
    _call_back_types: CFOptionFlags,
    _info: *mut c_void,
) {
}

/// Runs the main run loop for `duration`, servicing AppKit.
pub(super) fn run_main_loop_for(duration: Duration) {
    // SAFETY: the default-mode constant is a valid mode name.
    let result = CFRunLoop::run_in_mode(
        unsafe { kCFRunLoopDefaultMode },
        duration.as_secs_f64(),
        false,
    );
    if result == CFRunLoopRunResult::Finished {
        std::thread::sleep(duration);
    }
}

/// Delivers every queued application event without waiting.
pub(super) fn dispatch_app_events(app: &NSApplication) {
    let now = NSDate::distantPast();
    // SAFETY: the default run loop mode constant is a valid mode name.
    let mode = unsafe { NSDefaultRunLoopMode };
    while let Some(event) =
        app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&now), mode, true)
    {
        app.sendEvent(&event);
    }
}
