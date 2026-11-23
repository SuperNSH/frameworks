unsafe extern "C" {
    fn scanner_value() -> i32;
    fn codec_value() -> i32;
}

/// The shared library's value and the static one's.
pub fn values() -> (i32, i32) {
    unsafe { (scanner_value(), codec_value()) }
}
