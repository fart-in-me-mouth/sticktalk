//! Shared input-only adapter around iced's native Scrollable. Keeps native drag,
//! touch, focus, overlays, operations and precise pixel/momentum behavior intact.
mod controller;
mod widget;
pub use widget::SmoothScrollable;

#[cfg(feature = "wgpu")]
mod gpu_counter;
#[cfg(feature = "wgpu")]
pub use gpu_counter::GpuCounter;

mod subpixel;

pub mod pacing;
