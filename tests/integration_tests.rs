use std::process;
use std::fs;
use std::path::PathBuf;
use std::error;
use std::str;
use assert_cmd::prelude::CommandCargoExt;
use std::env;
use tempfile::TempDir;

#[test]
fn respect_git_ignore() -> Result<(), Box<dyn error::Error>> {
    let mut tre = process::Command::cargo_bin("tre")?;
    let fixture_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
    ].iter().collect();
    // this path is ignored by fixtures/.gitignore
    let ignored_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
        "ignore_me"
    ].iter().collect();
    fs::write(ignored_path, "")?;
    env::set_current_dir(fixture_path)?;
    let output = tre.output()?.stdout;
    let text = str::from_utf8(&output)?;
    assert!(text.contains("."));
    assert!(text.contains("── .gitignore"));
    assert!(text.contains("── a"));
    assert!(text.contains("── b"));
    assert!(text.contains("── c"));
    assert!(text.contains("── d"));
    assert!(text.contains("── e"));
    assert!(text.contains("── h"));
    assert!(text.contains("── f"));
    assert!(text.contains("── g"));
    assert!(!text.contains("ignore_me"));
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn ignore_hidden() -> Result<(), Box<dyn error::Error>> {
    let mut tre = process::Command::cargo_bin("tre")?;
    let fixture_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
    ].iter().collect();
    // this path is ignored by fixtures/.gitignore, but we aren't using .gitignore
    let ignored_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
        "ignore_me"
    ].iter().collect();
    fs::write(ignored_path, "")?;
    env::set_current_dir(fixture_path)?;
    let output = tre.arg("-s").output()?.stdout;
    let text = str::from_utf8(&output)?;
    assert!(text.contains("."));
    assert!(!text.contains("── .gitignore")); // hidden files should be hidden
    assert!(text.contains("── a"));
    assert!(text.contains("── b"));
    assert!(text.contains("── c"));
    assert!(text.contains("── d"));
    assert!(text.contains("── e"));
    assert!(text.contains("── h"));
    assert!(text.contains("── f"));
    assert!(text.contains("── g"));
    assert!(text.contains("ignore_me"));
    Ok(())
}

#[test]
fn all_files() -> Result<(), Box<dyn error::Error>> {
    let mut tre = process::Command::cargo_bin("tre")?;
    let fixture_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
    ].iter().collect();
    // this path is ignored by fixtures/.gitignore, but we aren't using .gitignore
    let ignored_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "fixtures",
        "ignore_me"
    ].iter().collect();
    fs::write(ignored_path, "")?;
    env::set_current_dir(fixture_path)?;
    let output = tre.arg("-a").output()?.stdout;
    let text = str::from_utf8(&output)?;
    assert!(text.contains("."));
    assert!(text.contains("── .gitignore")); // hidden files should be hidden
    assert!(text.contains("── a"));
    assert!(text.contains("── b"));
    assert!(text.contains("── c"));
    assert!(text.contains("── d"));
    assert!(text.contains("── e"));
    assert!(text.contains("── h"));
    assert!(text.contains("── f"));
    assert!(text.contains("── g"));
    assert!(text.contains("ignore_me"));
    Ok(())
}

#[test]
fn external_path_shows_contents() -> Result<(), Box<dyn error::Error>> {
    // Create temp dir outside any git repo
    // Note: tempfile creates hidden dirs (.tmpXXX), so we create a visible subdir
    let temp_dir = TempDir::new()?;
    let test_dir = temp_dir.path().join("test_external");
    fs::create_dir(&test_dir)?;

    let test_file = test_dir.join("test_file.txt");
    fs::write(&test_file, "content")?;

    // Create a subdirectory with a file
    let sub_dir = test_dir.join("subdir");
    fs::create_dir(&sub_dir)?;
    fs::write(sub_dir.join("nested.txt"), "nested content")?;

    // Run tre on the external path (using absolute path)
    let mut tre = process::Command::cargo_bin("tre")?;
    let output = tre.arg(&test_dir).output()?.stdout;
    let text = str::from_utf8(&output)?;

    // Should show contents of the external directory
    assert!(text.contains("test_file.txt"), "Should show test_file.txt, got: {}", text);
    assert!(text.contains("subdir"), "Should show subdir, got: {}", text);
    assert!(text.contains("nested.txt"), "Should show nested.txt, got: {}", text);
    Ok(())
}
