//! The flight model, ported one checked piece at a time.
//!
//! The original writes a report of its own aerodynamic set-up to `Cycle Dump.txt` when it loads
//! an aircraft (`veh_class::load_veh`, oracle `0x1002300b0`, the stream `_cyc`). That report is
//! the oracle for everything in this module: a piece counts as ported when it reproduces the
//! report's numbers for the shipped Cessna 172SP, as printed.

pub mod wing;
