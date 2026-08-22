//! Design tokens for Zero Desktop.
//!
//! Three layers, exactly as specified in `skills/feature/personal-ui-design-system`:
//!
//! ```text
//! primitive  →  raw values, never referenced by UI code
//! semantic   →  role-named values, this is what UI code uses
//! component  →  per-component overrides, derived from semantic
//! ```
//!
//! No widget is allowed to hardcode a colour, spacing step or radius. If a value
//! is missing here, add it here — do not inline it at the call site.

#![forbid(unsafe_code)]

pub mod color;
pub mod metrics;
pub mod theme;
pub mod typography;

pub use color::Rgba;
pub use metrics::{Radius, Shell, Spacing};
pub use theme::{Semantic, Theme};
pub use typography::{FontRole, TextStyle, Typography};

/// The default Zero Desktop appearance: dark, neutral, no dominant accent hue.
pub fn default_theme() -> Theme {
    Theme::zero_dark()
}
