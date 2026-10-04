//! Allocation gates for hot UI paths. Run in release mode with `--ignored`.
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
#[ignore = "release allocation gate"]
fn ui_allocation_budgets() {
    let mut screen = mtty_core::ATerm::new(100, 30, 10_000);
    for _ in 0..40 {
        screen.process(b"\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m text 1234567890\r\n");
    }
    let theme = mtty_ui::UiTheme::nord();
    let cursor = Some(screen.cursor());
    let (allocs, bytes) = measure(|| {
        std::hint::black_box(mtty_ui::build_rows(&screen, &theme, cursor));
    });
    println!(
        "build_rows 100x30: {allocs} allocations/reallocations, {bytes} requested bytes/frame"
    );
    assert!(
        allocs < 600,
        "row building must allocate by span, not by cell"
    );

    let entries: Vec<_> = (0..10_000).map(|i| format!("file-{i}.rs")).collect();
    let (allocs, bytes) = measure(|| {
        for label in &entries {
            std::hint::black_box(mtty_ui::palette::score(label, "file", "file-9"));
        }
    });
    println!("ASCII palette score 10k: {allocs} allocations, {bytes} requested bytes");
    assert_eq!(allocs, 0, "ASCII scoring must not allocate per entry");
}
