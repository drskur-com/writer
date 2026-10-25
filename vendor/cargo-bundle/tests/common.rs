use std::fs;
use std::path::PathBuf;

pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn cargo_bundle_bin() -> PathBuf {
    project_root().join("target/debug/cargo-bundle")
}

#[cfg(test)]
#[allow(dead_code)] // Used in tests
pub fn parse_bundle_paths(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .filter(|line| line.starts_with("        "))
        .map(|line| PathBuf::from(line.trim()))
        .collect()
}

/// Places a valid-enough binary at `target/debug/examples/<name>` so that
/// cargo-bundle (running with CARGO_BUNDLE_SKIP_BUILD) has something to package.
pub fn setup_example_binary(name: &str) {
    let bin_dir = project_root().join("target/debug/examples");
    fs::create_dir_all(&bin_dir).unwrap();
    let bin_path = bin_dir.join(name);
    #[cfg(target_os = "macos")]
    {
        // macOS copies the binary into the .app; use a for realsies Mach-O so the copy runs.
        fs::copy(cargo_bundle_bin(), &bin_path).unwrap();
    }
    #[cfg(not(target_os = "macos"))]
    {
        fs::write(&bin_path, b"dummy binary content").unwrap();
    }
}
