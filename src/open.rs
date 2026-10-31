//! Opening files and links outside the TUI, with guards against running code by accident.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use color_eyre::Result;
use url::Url;

/// File types that can run code when opened with their default app (macOS, Linux, Windows).
/// Anyone who can upload to a shared folder or forum could place such a file, so these are
/// only shown in the file manager, never opened.
const RISKY_EXTENSIONS: &[&str] = &[
    // Scripts and shells
    "command",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "fish",
    "tool",
    "py",
    "pl",
    "rb",
    "php",
    "js",
    "jse",
    "vbs",
    "vbe",
    "wsf",
    "ps1",
    "psm1",
    "bat",
    "cmd",
    "scpt",
    "applescript",
    // Apps, installers, binaries
    "app",
    "pkg",
    "mpkg",
    "jar",
    "desktop",
    "appimage",
    "run",
    "bin",
    "deb",
    "rpm",
    "flatpakref",
    "exe",
    "msi",
    "com",
    "scr",
    "pif",
    "hta",
    "cpl",
    "lnk",
    "reg",
    // macOS files that launch apps or commands
    "terminal",
    "fileloc",
    "inetloc",
    "workflow",
    "action",
    "prefpane",
    "kext",
];

/// Whether opening `path` could run code.
pub fn is_risky(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| RISKY_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Only plain web links to the ILIAS server itself are handed to the browser, never other
/// schemes (`file:`, app URL schemes, …) that `open` / `xdg-open` would also accept.
pub fn is_ilias_link(url: &str, base: &Url) -> bool {
    Url::parse(url).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http") && url.host_str() == base.host_str()
    })
}

/// Opens a URL in the web browser, or a file with its default app.
pub fn external(target: &OsStr) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    command.arg(target).spawn()?;
    Ok(())
}

/// Shows a file in the file manager without opening it.
pub fn reveal(path: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()?;
    } else if cfg!(windows) {
        Command::new("explorer").arg("/select,").arg(path).spawn()?;
    } else {
        // xdg-open has no "select this file"; open its folder instead.
        external(path.parent().unwrap_or(path).as_os_str())?;
    }
    Ok(())
}

/// Marks a downloaded file like browsers do, so macOS Gatekeeper checks it before anything
/// in it runs. Other systems have no equivalent; there, downloads are simply never made
/// executable. Best effort: a failure only means no extra check.
pub fn quarantine(path: &Path) {
    if cfg!(target_os = "macos") {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = Command::new("xattr")
            .args([
                "-w",
                "com.apple.quarantine",
                &format!("0081;{now:08x};ilias-tui;"),
            ])
            .arg(path)
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_files_that_can_run_code() {
        for name in [
            "notes.command",
            "setup.SH",
            "tool.desktop",
            "x.AppImage",
            "a.jar",
            "b.exe",
        ] {
            assert!(is_risky(Path::new(name)), "{name}");
        }
        for name in [
            "slides.pdf",
            "sheet.xlsx",
            "data.zip",
            "README",
            "image.png",
        ] {
            assert!(!is_risky(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn only_opens_web_links_to_ilias() {
        let base = Url::parse("https://elearning.hslu.ch/ilias/").unwrap();
        assert!(is_ilias_link(
            "https://elearning.hslu.ch/ilias/goto.php/crs/1",
            &base
        ));
        assert!(!is_ilias_link(
            "https://evil.example/ilias/goto.php/crs/1",
            &base
        ));
        assert!(!is_ilias_link("file:///etc/passwd", &base));
        assert!(!is_ilias_link("vscode://some/command", &base));
        assert!(!is_ilias_link("not a url", &base));
    }
}
