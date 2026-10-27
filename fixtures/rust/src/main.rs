//! A Rust demo for PDB fixtures. It uses enums with data, generics, trait objects,
//! closures, an `async fn`, thread-local storage, statics, a union and a caught panic.

use std::cell::Cell;
use std::hint::black_box;
use std::panic;
use std::pin::{Pin, pin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};

/// A shape with data in each variant.
#[derive(Debug, Clone)]
enum Shape {
    Circle { radius: f64 },
    Rect(f64, f64),
    Label(String),
    Empty,
}

/// An object-safe trait for the trait-object records.
trait Area {
    fn area(&self) -> f64;
    fn label(&self) -> &'static str {
        "shape"
    }
}

impl Area for Shape {
    fn area(&self) -> f64 {
        match self {
            Shape::Circle { radius } => 3.25 * radius * radius,
            Shape::Rect(width, height) => width * height,
            Shape::Label(text) => text.len() as f64,
            Shape::Empty => 0.0,
        }
    }
}

/// A generic wrapper with a tag byte.
struct Wrapper<T> {
    inner: T,
    tag: u8,
}

impl<T: Copy + Into<u64>> Wrapper<T> {
    fn widened(&self) -> u64 {
        self.inner.into() + u64::from(self.tag)
    }
}

/// A C-compatible point.
#[repr(C)]
#[derive(Clone, Copy)]
struct CPoint {
    x: i32,
    y: i32,
}

/// A C-compatible union.
#[repr(C)]
union Bits {
    int: u32,
    float: f32,
}

static GREETING: &str = "pdb-rs";
static COUNTER: AtomicU64 = AtomicU64::new(0);
const LIMIT: usize = 4;

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// A future that returns `Pending` once before it completes.
struct YieldOnce {
    done: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.done {
            Poll::Ready(())
        } else {
            self.done = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

async fn add_later(a: u64, b: u64) -> u64 {
    YieldOnce { done: false }.await;
    a + b
}

/// Poll a future on the current thread until it completes.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

fn total_area(shapes: &[Box<dyn Area>]) -> f64 {
    shapes.iter().map(|shape| shape.area()).sum()
}

fn caught_panic() -> bool {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(|| {
        if black_box(true) {
            panic!("expected demo panic");
        }
    });
    panic::set_hook(previous);
    result.is_err()
}

fn main() {
    let shapes: Vec<Box<dyn Area>> = vec![
        Box::new(Shape::Circle { radius: 2.0 }),
        Box::new(Shape::Rect(3.0, 4.0)),
        Box::new(Shape::Label(GREETING.to_owned())),
        Box::new(Shape::Empty),
    ];
    let offset = 10.0;
    let scaled: Vec<f64> = shapes.iter().take(LIMIT).map(|shape| shape.area() + offset).collect();
    let wrapper = Wrapper { inner: 7_u32, tag: 3 };
    let point = CPoint { x: 4, y: 5 };
    let bits = Bits { float: 1.0 };
    // SAFETY: both fields are 32-bit plain data, so every bit pattern is valid.
    let raw = unsafe { bits.int };
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    COUNTER.fetch_add(wrapper.widened(), Ordering::Relaxed);
    let sum = block_on(add_later(2, 3));
    let panicked = caught_panic();
    println!(
        "rust demo: {} {:.1} {} {} {} {} {} {} {}",
        shapes[0].label(),
        total_area(&shapes),
        scaled.len(),
        point.x + point.y,
        raw,
        DEPTH.with(Cell::get),
        COUNTER.load(Ordering::Relaxed),
        sum,
        panicked,
    );
}
