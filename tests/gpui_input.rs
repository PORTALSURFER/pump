//! Native macOS input coverage for the live embedded Pump GPUI editor.

#[cfg(target_os = "macos")]
mod macos_clipboard {
    use cocoa::base::id;
    use objc::runtime::BOOL;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::{CStr, CString};

    struct PasteboardSnapshot {
        items: Vec<PasteboardItemSnapshot>,
    }

    struct PasteboardItemSnapshot {
        types: Vec<(String, Vec<u8>)>,
    }

    pub struct PasteboardRestore(PasteboardSnapshot);

    impl PasteboardRestore {
        pub unsafe fn capture() -> Self {
            Self(PasteboardSnapshot::capture())
        }
    }

    impl Drop for PasteboardRestore {
        fn drop(&mut self) {
            unsafe {
                self.0.restore();
            }
        }
    }

    impl PasteboardSnapshot {
        unsafe fn capture() -> Self {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            if pasteboard.is_null() {
                return Self { items: Vec::new() };
            }
            let pasteboard_items: id = msg_send![pasteboard, pasteboardItems];
            if pasteboard_items.is_null() {
                return Self { items: Vec::new() };
            }
            let item_count: usize = msg_send![pasteboard_items, count];
            let mut items = Vec::with_capacity(item_count);
            for item_index in 0..item_count {
                let item: id = msg_send![pasteboard_items, objectAtIndex: item_index];
                if item.is_null() {
                    continue;
                }
                let item_types: id = msg_send![item, types];
                if item_types.is_null() {
                    items.push(PasteboardItemSnapshot { types: Vec::new() });
                    continue;
                }
                let type_count: usize = msg_send![item_types, count];
                let mut types = Vec::with_capacity(type_count);
                for type_index in 0..type_count {
                    let type_object: id = msg_send![item_types, objectAtIndex: type_index];
                    let type_name = pasteboard_string(type_object)
                        .expect("pasteboard type should expose UTF-8 text");
                    let data: id = msg_send![item, dataForType: type_object];
                    let data_length = if data.is_null() {
                        0
                    } else {
                        msg_send![data, length]
                    };
                    let bytes = if data_length == 0 {
                        Vec::new()
                    } else {
                        let pointer: *const u8 = msg_send![data, bytes];
                        assert!(!pointer.is_null(), "pasteboard data should have bytes");
                        std::slice::from_raw_parts(pointer, data_length).to_vec()
                    };
                    types.push((type_name, bytes));
                }
                items.push(PasteboardItemSnapshot { types });
            }
            Self { items }
        }

        unsafe fn restore(&self) {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            if pasteboard.is_null() {
                return;
            }
            let _: isize = msg_send![pasteboard, clearContents];
            if self.items.is_empty() {
                return;
            }
            let objects: id =
                msg_send![class!(NSMutableArray), arrayWithCapacity: self.items.len()];
            if objects.is_null() {
                return;
            }
            for snapshot in &self.items {
                let allocated: id = msg_send![class!(NSPasteboardItem), alloc];
                let item: id = msg_send![allocated, init];
                if item.is_null() {
                    continue;
                }
                for (type_name, bytes) in &snapshot.types {
                    let type_c_string = CString::new(type_name.as_bytes())
                        .expect("pasteboard type should not contain nul");
                    let type_object: id = msg_send![
                        class!(NSString),
                        stringWithUTF8String: type_c_string.as_ptr()
                    ];
                    if type_object.is_null() {
                        continue;
                    }
                    let data: id = msg_send![
                        class!(NSData),
                        dataWithBytes: bytes.as_ptr()
                        length: bytes.len()
                    ];
                    let _: BOOL = msg_send![item, setData: data forType: type_object];
                }
                let _: () = msg_send![objects, addObject: item];
                let _: () = msg_send![item, release];
            }
            let _: BOOL = msg_send![pasteboard, writeObjects: objects];
        }
    }

    unsafe fn pasteboard_string(value: id) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let pointer: *const i8 = msg_send![value, UTF8String];
        if pointer.is_null() {
            return None;
        }
        CStr::from_ptr(pointer).to_str().ok().map(str::to_owned)
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_gpui_input_regression() {
    if std::env::var_os("TOYBOX_UI_SCREENSHOT").is_none() {
        eprintln!("native input skipped: set TOYBOX_UI_SCREENSHOT=1 to run AppKit fixture");
        return;
    }

    let runner = std::env::var_os("CARGO_BIN_EXE_pump_gpui_input")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let executable = std::env::current_exe().ok()?;
            let target_dir = executable.parent()?.parent()?;
            let direct = target_dir.join("pump_gpui_input");
            direct.is_file().then_some(direct).or_else(|| {
                let example = target_dir.join("examples/pump_gpui_input");
                example.is_file().then_some(example)
            })
        })
        .expect("Cargo should build the Pump GPUI input runner");

    let _pasteboard_restore = unsafe { macos_clipboard::PasteboardRestore::capture() };
    let status = std::process::Command::new(runner)
        .status()
        .expect("Pump GPUI input runner should start");
    assert!(
        status.success(),
        "Pump GPUI native input runner failed: {status}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn native_gpui_input_regression() {
    // The regression exercises AppKit NSEvents against a real NSView. The
    // shared hosted editor remains covered by the non-macOS GPUI tests.
}
