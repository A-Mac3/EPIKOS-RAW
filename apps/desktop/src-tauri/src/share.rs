//! macOS AirDrop: hand exported files to the system's AirDrop sheet
//! (NSSharingService), which shows nearby devices and sends them.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
pub fn airdrop(app: &tauri::AppHandle, files: Vec<PathBuf>) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSSharingService, NSSharingServiceNameSendViaAirDrop};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let (tx, rx) = std::sync::mpsc::channel();
    // AppKit UI: on the main thread.
    app.run_on_main_thread(move || {
        let result = (|| {
            // SAFETY: a static NSString exported by AppKit.
            let name = unsafe { NSSharingServiceNameSendViaAirDrop };
            let service = NSSharingService::sharingServiceNamed(name).ok_or("AirDrop isn't available on this Mac")?;
            let items: Vec<Retained<AnyObject>> = files
                .iter()
                .map(|f| Retained::into_super(Retained::into_super(NSURL::fileURLWithPath(&NSString::from_str(&f.to_string_lossy())))))
                .collect();
            let items = NSArray::from_retained_slice(&items);
            // SAFETY: file URLs are valid AirDrop items (NSPasteboardWriting).
            unsafe {
                if !service.canPerformWithItems(Some(&items)) {
                    return Err("AirDrop can't send these files (is Wi-Fi or Bluetooth off?)");
                }
                service.performWithItems(&items);
            }
            Ok(())
        })();
        let _ = tx.send(result.map_err(str::to_string));
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

#[cfg(not(target_os = "macos"))]
pub fn airdrop(_: &tauri::AppHandle, _: Vec<PathBuf>) -> Result<(), String> {
    Err("AirDrop is only available on macOS".into())
}
