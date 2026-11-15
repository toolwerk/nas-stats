//! Swift face of the NAS Stats core, via UniFFI.
//!
//! For now only the version of the core, so the chain from Rust to the apps
//! is built and checked. Records and calls for the apps follow (#12).

uniffi::setup_scaffolding!();

/// Version of the Rust core, as the apps show it ("Core 0.1.0").
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_is_the_workspace_version() {
        assert_eq!(core_version(), "0.1.0");
    }
}
