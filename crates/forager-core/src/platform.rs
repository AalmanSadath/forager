use std::path::PathBuf;

use crate::model::Format;

/// Native package manager found on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Dnf,
    Zypper,
    Apt,
}

#[derive(Debug, Clone)]
pub struct Platform {
    /// Architecture as reported by Rust, e.g. `x86_64` or `aarch64`.
    pub arch: &'static str,
    pub package_manager: Option<PackageManager>,
}

impl Platform {
    pub fn detect() -> Platform {
        let package_manager = if has_command("dnf") {
            Some(PackageManager::Dnf)
        } else if has_command("zypper") {
            Some(PackageManager::Zypper)
        } else if has_command("apt-get") {
            Some(PackageManager::Apt)
        } else {
            None
        };
        Platform {
            arch: std::env::consts::ARCH,
            package_manager,
        }
    }

    /// Formats this system can install, most preferred first.
    pub fn formats(&self) -> Vec<Format> {
        match self.package_manager {
            Some(PackageManager::Dnf | PackageManager::Zypper) => vec![Format::Rpm, Format::AppImage, Format::Tarball],
            Some(PackageManager::Apt) => vec![Format::Deb, Format::AppImage, Format::Tarball],
            None => vec![Format::AppImage, Format::Tarball],
        }
    }

    /// Names releases use for this architecture.
    pub fn arch_aliases(&self) -> &'static [&'static str] {
        arch_aliases(self.arch)
    }
}

pub fn arch_aliases(arch: &str) -> &'static [&'static str] {
    match arch {
        "x86_64" => &["x86_64", "x86-64", "amd64", "x64"],
        "aarch64" => &["aarch64", "arm64"],
        "arm" => &["armv7", "armv7l", "armhf"],
        "x86" => &["i386", "i686", "x86_32"],
        "riscv64" => &["riscv64"],
        "powerpc64" => &["ppc64le"],
        "s390x" => &["s390x"],
        _ => &[],
    }
}

/// Architectures recognised when rejecting assets built for another one.
pub const KNOWN_ARCHES: [&str; 7] = ["x86_64", "aarch64", "arm", "x86", "riscv64", "powerpc64", "s390x"];

pub fn has_command(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// `~/Applications`, where AppImages are placed.
pub fn appimage_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Applications"))
}

/// `~/.local/opt`, where tarballs are unpacked.
pub fn opt_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("opt"))
}

/// `~/.local/bin`, where executables from tarballs are linked.
pub fn bin_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("bin"))
}
