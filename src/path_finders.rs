use crate::file_tree::FileType;
use std::path::Path;
use std::process::Command;
use std::{fs, path};
use walkdir::{DirEntry, WalkDir};

/// Only check paths that could be outside repo (absolute or contains ..)
fn needs_repo_check(path: &str) -> bool {
    Path::new(path).is_absolute() || path.contains("..")
}

/// Check if the given path is inside a git repository
fn is_inside_git_repo(path: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn find_all_paths(
    root: &str,
    directories_only: bool,
    max_depth: usize,
) -> Vec<(String, FileType)> {
    let mut result: Vec<(String, FileType)> = Vec::new();
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(meta) = entry.metadata() {
            if directories_only && !meta.is_dir() {
                continue;
            }

            if let Some(path) = entry.path().to_str() {
                let path = path.to_string();
                if path != root {
                    result.push((path, FileType::new(meta)))
                }
            }
        }
    }
    result
}

fn is_hidden(name: &str) -> bool {
    name != "." && name.starts_with('.') && name != ".."
}

fn should_include(entry: &DirEntry, root: &str) -> bool {
    // Always include the root directory, even if it's hidden
    if entry.path().to_str() == Some(root) {
        return true;
    }
    // For other entries, exclude hidden files
    entry
        .file_name()
        .to_str()
        .map(|s| !is_hidden(s))
        .unwrap_or(true)
}

pub fn find_non_hidden_paths(
    root: &str,
    directories_only: bool,
    max_depth: usize,
) -> Vec<(String, FileType)> {
    let walker = WalkDir::new(root).max_depth(max_depth).into_iter();
    let mut result: Vec<(String, FileType)> = Vec::new();

    for entry in walker
        .filter_entry(|e| should_include(e, root))
        .filter_map(|e| e.ok())
    {
        if let Ok(meta) = entry.metadata() {
            if directories_only && !meta.is_dir() {
                continue;
            }
            if let Some(path) = entry.path().to_str() {
                let path = path.to_string();
                if path != root {
                    result.push((path, FileType::new(meta)))
                }
            }
        }
    }
    result
}

pub fn find_non_git_ignored_paths(
    root: &str,
    directories_only: bool,
    max_depth: usize,
) -> Vec<(String, FileType)> {
    // Early return if path is outside any git repo
    if needs_repo_check(root) && !is_inside_git_repo(root) {
        return find_non_hidden_paths(root, directories_only, max_depth);
    }

    let mut git_command = Command::new("git");
    if directories_only {
        git_command
            .arg("ls-tree")
            .arg("-r")
            .arg("-d")
            .arg("--name-only")
            .arg("HEAD")
            .arg(root);
    } else {
        git_command
            .arg("ls-files")
            .arg("-o")
            .arg("-c")
            .arg("--exclude-standard")
            .arg(root);
    };

    if let Ok(git_output) = git_command.output() {
        if git_output.status.success() {
            if let Ok(paths_buf) = String::from_utf8(git_output.stdout) {
                return paths_buf
                    .split('\n')
                    .filter_map(|p| {
                        let path_string = if max_depth != usize::MAX {
                            path::Path::new(p)
                                .components()
                                .take(max_depth)
                                .collect::<path::PathBuf>()
                                .as_path()
                                .to_str()
                                .unwrap()
                                .to_string()
                        } else {
                            p.to_string()
                        };
                        fs::metadata(&path_string)
                            .map(|m| (path_string, FileType::new(m)))
                            .ok()
                    })
                    .collect();
            }
        }
    }

    find_non_hidden_paths(root, directories_only, max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_repo_check() {
        // Absolute paths should need checking
        assert!(needs_repo_check("/absolute/path"));
        assert!(needs_repo_check("/tmp"));

        // Paths with .. should need checking
        assert!(needs_repo_check("../parent"));
        assert!(needs_repo_check("foo/../bar"));

        // Simple relative paths should not need checking
        assert!(!needs_repo_check("relative/path"));
        assert!(!needs_repo_check("./current"));
        assert!(!needs_repo_check("."));
        assert!(!needs_repo_check("src"));
    }

    #[test]
    fn test_is_inside_git_repo() {
        // Current directory (this project) should be inside a git repo
        assert!(is_inside_git_repo("."));

        // /tmp should not be inside a git repo
        assert!(!is_inside_git_repo("/tmp"));
    }
}
