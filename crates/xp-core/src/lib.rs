//! Deterministic, window-free core. No GPU, no OS handles, no wall-clock time, no global RNG.
//! All time enters as `dt`, all randomness as an explicit seeded generator, so golden tests
//! are reproducible.
//!
//! Everything here decodes X-Plane 10.51r2 content or runs its rules. Nothing in this crate
//! may depend on a window, a device, or `wgpu`.

pub mod acf;
pub mod airport;
pub mod apt_index;
pub mod aptdat;
pub mod btnwin;
pub mod content;
pub mod dsf;
pub mod flight;
pub mod geoid;
pub mod gpsdb;
pub mod menu;
pub mod navaid;
pub mod obj8;
pub mod oracle;
pub mod panel;
pub mod png;
pub mod scene;
pub mod scenery;
pub mod tex;
pub mod view;

/// Read a shipped text file as UTF-8, falling back to a lossy decode.
///
/// Needed because the install is not uniformly UTF-8: `Resources/default scenery/default
/// apt dat/Mars nav data/apt.dat` carries a Latin-1 copyright sign inside its GPL notice,
/// and a strict reader fails the whole file on one byte. Substituted characters are
/// preserved as U+FFFD so the position is still visible to a caller that cares.
pub fn read_text_lossy(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let bytes = std::fs::read(path.as_ref())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Locate the original installation.
///
/// Content goldens are extracted from the shipped install; tests that need the whole
/// installation read it through this helper and skip (loudly) when it is absent, so the
/// crate still builds and unit-tests on a machine without the 6 GB of content.
pub fn install_root() -> Option<std::path::PathBuf> {
    std::env::var_os("XP_INSTALL").map(std::path::PathBuf::from)
}
