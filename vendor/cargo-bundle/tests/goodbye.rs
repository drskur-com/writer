mod common;

use std::process::Command;

#[test]
#[cfg(target_os = "macos")]
fn osx() {
    use std::fs;
    common::setup_example_binary("goodbye");

    let root = common::project_root();
    let output = Command::new(common::cargo_bundle_bin())
        .args(["bundle", "--example", "goodbye", "--format", "osx"])
        .current_dir(&root)
        .env("CARGO_BUNDLE_SKIP_BUILD", "1")
        .output()
        .expect("Failed to execute cargo-bundle");

    assert!(
        output.status.success(),
        "cargo-bundle failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let app_path = root.join("target/debug/examples/bundle/osx/Goodbye.app");
    assert!(app_path.exists(), "App bundle not found at {:?}", app_path);

    // PNG icons should have been packed into an ICNS file.
    let icns_path = app_path.join("Contents/Resources/Goodbye.icns");
    assert!(icns_path.exists(), "ICNS not found at {:?}", icns_path);
    assert!(
        fs::metadata(&icns_path).unwrap().len() > 0,
        "ICNS file is empty"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn deb() {
    common::setup_example_binary("goodbye");

    let root = common::project_root();
    let output = Command::new(common::cargo_bundle_bin())
        .args(["bundle", "--example", "goodbye", "--format", "deb"])
        .current_dir(&root)
        .env("CARGO_BUNDLE_SKIP_BUILD", "1")
        .output()
        .expect("Failed to execute cargo-bundle");

    assert!(
        output.status.success(),
        "cargo-bundle failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let bundle_paths = common::parse_bundle_paths(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(bundle_paths.len(), 1, "Expected exactly one bundle path");
    let deb_path = &bundle_paths[0];
    assert!(
        deb_path.exists(),
        "Debian package not found at {:?}",
        deb_path
    );

    // The 128x128 PNG should have been placed in the hicolor icon tree.
    let package_dir = deb_path
        .parent()
        .unwrap()
        .join(deb_path.file_stem().unwrap());
    let icon_path = package_dir.join("data/usr/share/icons/hicolor/128x128/apps/goodbye.png");
    assert!(
        icon_path.exists(),
        "PNG icon not found in deb data at {:?}",
        icon_path
    );
}
