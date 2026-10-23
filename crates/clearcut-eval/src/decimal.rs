// Written by Kone Tshivhinda & Claude
//! The strict decimal reader now lives in `clearcut-core` so that gold-set
//! labels and what a model read are parsed by the same code (ADR-0013). This
//! module keeps the old path working.

pub use clearcut_core::decimal::*;
