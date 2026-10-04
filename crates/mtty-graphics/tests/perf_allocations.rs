//! Allocation gates for borrowed streams and image decoding (`--release --ignored`).
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static MEASURING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

fn record(bytes: usize) {
    if MEASURING.try_with(Cell::get).unwrap_or(false) {
        ALLOCS.with(|n| n.set(n.get() + 1));
        BYTES.with(|n| n.set(n.get() + bytes));
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure(f: impl FnOnce()) -> (usize, usize) {
    ALLOCS.with(|n| n.set(0));
    BYTES.with(|n| n.set(0));
    MEASURING.with(|active| active.set(true));
    f();
    MEASURING.with(|active| active.set(false));
    (ALLOCS.with(Cell::get), BYTES.with(Cell::get))
}

#[test]
#[ignore = "release stream/decoder allocation gate"]
fn streaming_and_rgba_allocation_budgets() {
    use base64::Engine;
    use mtty_graphics::{kitty, Scanner, StreamEvent};
    let mut scanner = Scanner::new();
    let text = b"normal \x1b[32mcolored\x1b[0m output\r\n";
    let (allocs, bytes) = measure(|| {
        for _ in 0..1000 {
            scanner.feed_with(text, |event| match event {
                StreamEvent::Text(text) => {
                    std::hint::black_box(text);
                }
                _ => panic!("unexpected event"),
            });
        }
    });
    println!("plain/color stream 1000 chunks: {allocs} allocations, {bytes} requested bytes");
    assert_eq!(allocs, 0);

    let pixels = vec![128u8; 1024 * 1024 * 4];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&pixels);
    let command = kitty::parse(format!("a=T,f=32,s=1024x1024;{encoded}").as_bytes());
    let mut decoded = None;
    let (allocs, bytes) = measure(|| {
        decoded = kitty::decode(&command, 1024 * 1024);
    });
    println!("4 MiB raw RGBA decode: {allocs} allocations, {bytes} requested bytes");
    assert_eq!(decoded.unwrap().rgba, pixels);
    assert!(
        bytes < pixels.len() * 2,
        "base64/RGBA must not create duplicate full-size buffers"
    );
}
