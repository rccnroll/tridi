//! STEP (.step/.stp) through the system's OpenCASCADE, 7.6 or later. The
//! tessellation runs in a helper process, a small program embedded in this
//! crate: it hands back each solid as it's done and is killed once none has
//! come for a while, so a part that takes forever to heal, a crash in C++ or
//! a runaway memory never reach the program using the crate.
//!
//! Linux only. Building needs OpenCASCADE's headers and libraries; running
//! needs its libraries.

// ======================================= Lint Settings ======================================= {{{

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "a test may panic: that is its failure"
    )
)]

// }}}

// ======================================== Sub-modules ======================================== {{{

mod helper;
mod parent;

// }}}

// ======================================== Re-exports ========================================= {{{

pub use parent::{Purpose, StepError, StepMesh, StepResult, load};

// }}}

// ========================================== Imports ========================================== {{{

#[macro_use]
extern crate tracing;

// }}}
