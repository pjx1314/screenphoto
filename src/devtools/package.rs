use std::io::Write as _;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::devtools::util::{self, Error, Result};

pub struct Options {
    pub binary: PathBuf,
    pub target: String,
    pub out: PathBuf,
}

pub fn run(root: &Path, options: &Options) -> Result<()> {
    let binary = root.join(&options.binary);
    if !binary.is_file() {
        return Err(Error::usage(format!(
            "binary not found: {}",
            binary.display()
        )));
    }
    let skill = root.join("SKILL.md");
    if !skill.is_file() {
        return Err(Error::usage(format!(
            "SKILL.md not found: {}",
            skill.display()
        )));
    }
    check_skill_version(&skill)?;
    let license = root.join("LICENSE");
    if !license.is_file() {
        return Err(Error::usage(format!(
            "LICENSE not found: {}",
            license.display()
        )));
    }

    let out = root.join(&options.out);
    let target_out = out.join(&options.target);
    let staging = target_out.join("screenphoto");
    std::fs::create_dir_all(staging.join("bin"))?;

    let executable = if options.target.contains("windows")
        || binary
            .extension()
            .map(|extension| extension.to_string_lossy().eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
    {
        "screenphoto.exe"
    } else {
        "screenphoto"
    };
    let staged_binary = staging.join("bin").join(executable);
    if same_file::is_same_file(&binary, &staged_binary).unwrap_or(false) {
        return Err(Error::usage(format!(
            "refusing to overwrite the input binary: {}",
            binary.display()
        )));
    }
    std::fs::copy(&binary, &staged_binary)?;
    std::fs::copy(&skill, staging.join("SKILL.md"))?;
    std::fs::copy(&license, staging.join("LICENSE"))?;
    let executable_mode = if executable.ends_with(".exe") {
        0o644
    } else {
        0o755
    };
    set_mode(&staged_binary, executable_mode)?;

    let versioned = out.join(format!(
        "screenphoto-{}-{}.zip",
        super::VERSION,
        options.target
    ));
    let entries = vec![
        (
            "screenphoto/SKILL.md".to_string(),
            staging.join("SKILL.md"),
            0o644,
        ),
        (
            "screenphoto/LICENSE".to_string(),
            staging.join("LICENSE"),
            0o644,
        ),
        (
            format!("screenphoto/bin/{executable}"),
            staged_binary.clone(),
            executable_mode,
        ),
    ];
    write_zip(&versioned, &entries)?;

    let fixed = target_out.join("screenphoto.zip");
    std::fs::copy(&versioned, &fixed)?;

    let mut sums = String::new();
    for path in [&versioned, &fixed, &staged_binary] {
        let name = path
            .strip_prefix(&out)?
            .to_string_lossy()
            .replace('\\', "/");
        sums.push_str(&format!("{}  {name}\n", util::sha256_file(path)?));
    }
    let checksums = target_out.join("SHA256SUMS");
    std::fs::write(&checksums, sums)?;

    println!("staged: {}", staging.display());
    println!("zip: {}", versioned.display());
    println!("zip: {}", fixed.display());
    println!("checksums: {}", checksums.display());
    Ok(())
}

/// Fail packaging when the Skill metadata drifts from the crate version.
fn check_skill_version(skill: &Path) -> Result<()> {
    let text = std::fs::read_to_string(skill)?;
    let version = metadata_version(&text).ok_or_else(|| {
        Error::internal(format!(
            "SKILL.md has no metadata.version: {}",
            skill.display()
        ))
    })?;
    if version != super::VERSION {
        return Err(Error::internal(format!(
            "SKILL.md metadata.version {version} does not match crate version {}",
            super::VERSION
        )));
    }
    Ok(())
}

/// Read `metadata.version` from the YAML frontmatter without a YAML dependency.
fn metadata_version(text: &str) -> Option<&str> {
    let mut frontmatter = text.lines().skip_while(|line| line.trim_end() != "---");
    frontmatter.next()?;
    let mut metadata = false;
    for line in frontmatter {
        if line.trim_end() == "---" {
            break;
        }
        if line.starts_with("metadata:") {
            metadata = true;
            continue;
        }
        if metadata {
            if let Some(value) = line.strip_prefix("  version:") {
                return Some(value.trim().trim_matches('"').trim_matches('\''));
            }
            if !line.starts_with(' ') {
                metadata = false;
            }
        }
    }
    None
}

fn write_zip(path: &Path, entries: &[(String, PathBuf, u32)]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut archive = ZipWriter::new(file);
    for (name, source, mode) in entries {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(*mode);
        archive.start_file(name.clone(), options)?;
        archive.write_all(&std::fs::read(source)?)?;
    }
    drop(archive.finish()?);
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::metadata_version;

    #[test]
    fn metadata_version_reads_the_frontmatter_value() {
        let text = "---\nname: screenphoto\nmetadata:\n  version: \"1.2.3\"\n---\n# body\n";
        assert_eq!(metadata_version(text), Some("1.2.3"));
        assert_eq!(metadata_version("---\nname: x\n---\n"), None);
        assert_eq!(metadata_version("# no frontmatter"), None);
    }

    #[test]
    fn checked_in_skill_version_matches_the_crate() {
        assert_eq!(
            metadata_version(include_str!("../../SKILL.md")),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
