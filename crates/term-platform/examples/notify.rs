//! Ad-hoc native-notification smoke test used while landing ADR 0035. Build it,
//! wrap the binary in a throwaway `.app` (so `NSBundle.mainBundle` has an
//! identifier), sign ad-hoc and run it. It stays alive long enough to answer the
//! first-run authorization prompt, then posts a few banners. Not part of the
//! shipped hosts.

fn main() {
    for index in 1..=4 {
        miao_term_platform::notify("mtty", &format!("native notification smoke test #{index}"));
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
}
