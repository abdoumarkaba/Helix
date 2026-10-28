#![allow(
    clippy::pedantic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::must_use_candidate,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_cast,
    clippy::nursery
)]
pub mod binary;
pub mod execution;
pub mod guards;
pub mod models;
pub mod modules;
pub mod orchestrator;
pub mod tweak_state;
