use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");

    if let Some(commit) = git_commit_hash() {
        println!("cargo:rustc-env=RAIRSTREAM_GIT_COMMIT={commit}");
    }
}

fn git_commit_hash() -> Option<String> {
    let git_dir = Path::new(".git");
    let head_path = git_dir.join("HEAD");
    let head = fs::read_to_string(&head_path).ok()?;
    let head = head.trim();

    if let Some(reference) = head.strip_prefix("ref: ") {
        let reference_path = git_dir.join(reference);
        println!("cargo:rerun-if-changed={}", reference_path.display());
        read_ref(&reference_path)
    } else if head.is_empty() {
        None
    } else {
        Some(short_hash(head))
    }
}

fn read_ref(path: &Path) -> Option<String> {
    if let Ok(commit) = fs::read_to_string(path) {
        let commit = commit.trim();
        if !commit.is_empty() {
            return Some(short_hash(commit));
        }
    }

    let packed_refs_path = PathBuf::from(".git").join("packed-refs");
    println!("cargo:rerun-if-changed={}", packed_refs_path.display());
    let packed_refs = fs::read_to_string(packed_refs_path).ok()?;
    let target = path.strip_prefix(".git").ok()?.to_string_lossy();
    for line in packed_refs.lines() {
        if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let commit = parts.next()?;
        let reference = parts.next()?;
        if reference == target {
            return Some(short_hash(commit));
        }
    }
    None
}

fn short_hash(commit: &str) -> String {
    commit.chars().take(7).collect()
}
