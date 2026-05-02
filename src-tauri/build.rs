use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHANGELOG_ENTRY_LIMIT: usize = 18;
const CHANGELOG_SUBJECT_LIMIT: usize = 110;

struct ChangelogEntry {
    subject: String,
    short_hash: String,
    committed_at: String,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf();
    configure_git_rerun(&repo_root);

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let about_build_path = out_dir.join("about_build.rs");
    fs::write(&about_build_path, render_about_build(&repo_root)).expect("failed to write about build metadata");

    tauri_build::build()
}

fn configure_git_rerun(repo_root: &Path) {
    let git_dir = git_output(repo_root, &["rev-parse", "--git-dir"])
        .map(|path| {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() {
                candidate
            } else {
                repo_root.join(candidate)
            }
        })
        .unwrap_or_else(|| repo_root.join(".git"));

    println!("cargo:rerun-if-changed={}", git_dir.display());

    for arg in ["HEAD", "packed-refs", "refs", "logs/HEAD"] {
        if let Some(path) = git_output(repo_root, &["rev-parse", "--git-path", arg]) {
            let candidate = PathBuf::from(path);
            let resolved = if candidate.is_absolute() {
                candidate
            } else {
                repo_root.join(candidate)
            };
            println!("cargo:rerun-if-changed={}", resolved.display());
        }
    }
}

fn render_about_build(repo_root: &Path) -> String {
    let repository = git_output(repo_root, &["remote", "get-url", "origin"])
        .and_then(|value| parse_github_slug(&value));
    let commit = git_output(repo_root, &["rev-parse", "--short=9", "HEAD"]);
    let changelog = read_changelog(repo_root);

    let mut output = String::new();
    output.push_str("pub const BUILD_GITHUB_REPOSITORY: Option<&str> = ");
    output.push_str(&render_optional_str(repository.as_deref()));
    output.push_str(";\n");
    output.push_str("pub const BUILD_GIT_COMMIT: Option<&str> = ");
    output.push_str(&render_optional_str(commit.as_deref()));
    output.push_str(";\n");
    output.push_str("pub const BUILD_CHANGELOG: &[EmbeddedChangelogEntry] = &[\n");

    for entry in changelog {
        output.push_str("    EmbeddedChangelogEntry { subject: ");
        output.push_str(&render_str(&entry.subject));
        output.push_str(", short_hash: ");
        output.push_str(&render_str(&entry.short_hash));
        output.push_str(", committed_at: ");
        output.push_str(&render_str(&entry.committed_at));
        output.push_str(" },\n");
    }

    output.push_str("];\n");
    output
}

fn read_changelog(repo_root: &Path) -> Vec<ChangelogEntry> {
    let format = "%h%x1f%s%x1f%cI%x1e";
    let Some(raw) = git_output(
        repo_root,
        &[
            "log",
            "--no-merges",
            &format!("-n{}", CHANGELOG_ENTRY_LIMIT),
            &format!("--pretty=format:{format}"),
        ],
    ) else {
        return Vec::new();
    };

    raw.split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim();
            if record.is_empty() {
                return None;
            }

            let mut fields = record.split('\u{1f}');
            let short_hash = fields.next()?.trim();
            let subject = fields.next()?.trim();
            let committed_at = fields.next()?.trim();

            Some(ChangelogEntry {
                subject: truncate_chars(subject, CHANGELOG_SUBJECT_LIMIT),
                short_hash: short_hash.to_string(),
                committed_at: committed_at.to_string(),
            })
        })
        .collect()
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_github_slug(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/');
    let path = trimmed
        .strip_prefix("git@github.com:")
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
        .or_else(|| trimmed.strip_prefix("https://github.com/"))
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("git://github.com/"))?;

    let slug = path.trim_end_matches(".git");
    let mut parts = slug.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }

    Some(format!("{owner}/{repo}"))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn render_optional_str(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("Some({})", render_str(value)),
        None => "None".to_string(),
    }
}

fn render_str(value: &str) -> String {
    format!("{value:?}")
}
