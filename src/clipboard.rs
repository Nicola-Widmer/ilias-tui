//! Copying text to the clipboard without a clipboard library: `pbcopy` on macOS, `clip` on
//! Windows (fine for links, which are ASCII), elsewhere the terminal's OSC 52 escape sequence
//! (works in most terminals, also over SSH).

use std::io::Write;
use std::process::{Command, Stdio};

use color_eyre::Result;

pub fn copy(text: &str) -> Result<()> {
    let tool = match () {
        _ if cfg!(target_os = "macos") => Some("pbcopy"),
        _ if cfg!(windows) && text.is_ascii() => Some("clip"),
        _ => None,
    };
    if let Some(tool) = tool
        && let Ok(mut child) = Command::new(tool).stdin(Stdio::piped()).spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }
    let mut out = std::io::stdout();
    out.write_all(osc52(text).as_bytes())?;
    out.flush()?;
    Ok(())
}

/// "Put this in the clipboard" for the terminal.
fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(match i <= chunk.len() {
                true => ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char,
                false => '=',
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(
            base64("goto.php/crs/1 ü".as_bytes()),
            "Z290by5waHAvY3JzLzEgw7w="
        );
    }

    #[test]
    fn asks_the_terminal_with_osc52() {
        assert_eq!(osc52("foo"), "\x1b]52;c;Zm9v\x07");
    }
}
