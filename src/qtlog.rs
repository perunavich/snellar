use std::ffi::{CStr, c_char, c_int};

use snellar_core::journal;

unsafe extern "C" {
    fn snellar_install_message_handler();
}

#[unsafe(no_mangle)]
extern "C" fn snellar_qt_message(level: c_int, text: *const c_char) {
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    match level {
        1 => journal::warn("qml", text.trim_end()),
        2 | 3 => journal::error("qml", text.trim_end()),
        _ => {}
    }
}

pub fn install() {
    unsafe { snellar_install_message_handler() };
}
