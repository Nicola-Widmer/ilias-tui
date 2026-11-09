//! Tab completion of file paths, as a shell does it: the path being typed is completed to the
//! only file or folder it can mean, or as far as all of them agree.

use std::path::MAIN_SEPARATOR;

use crate::config;

/// Completing the input once.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// The input with the path completed (the same if nothing fits).
    pub input: String,
    /// The names it could still be, when there are several (sorted, folders end in `/`).
    pub choices: Vec<String>,
}

/// Where the path being completed starts in `input`, and the quote it is in (if any). With
/// `several`, paths are separated by spaces outside quotes; else the whole input is one path.
fn last_path(input: &str, several: bool) -> (usize, Option<char>) {
    let quoted = |at: usize| {
        input[at..]
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"'))
    };
    if !several {
        let at = input.len() - input.trim_start().len();
        return (at, quoted(at));
    }
    let (mut start, mut quote, mut escaped) = (0, None, false);
    for (i, c) in input.char_indices() {
        match (quote, c) {
            _ if escaped => escaped = false,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\\') if !cfg!(windows) => escaped = true,
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c.is_whitespace() => start = i + c.len_utf8(),
            _ => {}
        }
    }
    (start, quote.or_else(|| quoted(start)))
}

/// The path as meant: without its opening quote and, outside quotes, without escapes.
fn unescape(raw: &str, quote: Option<char>) -> String {
    match quote {
        Some(q) => raw.trim_start_matches(q).to_string(),
        None if cfg!(windows) => raw.to_string(),
        None => {
            let mut out = String::new();
            let mut chars = raw.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            out
        }
    }
}

/// The path written back so that it reads as one path again.
fn escape(path: &str, quote: Option<char>, several: bool) -> String {
    match quote {
        Some(q) => format!("{q}{path}"),
        // One path may contain spaces as they are.
        None if !several => path.to_string(),
        None if cfg!(windows) && path.contains(' ') => format!("\"{path}"),
        None if cfg!(windows) => path.to_string(),
        None => path.replace('\\', "\\\\").replace(' ', "\\ "),
    }
}

/// The longest start all `names` share.
fn common_start(names: &[String]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut end = first.len();
    for name in &names[1..] {
        end = first
            .char_indices()
            .zip(name.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8())
            .min(end);
    }
    first[..end].to_string()
}

/// Completes the path at the end of `input`. With `several`, `input` holds several paths
/// (the upload dialog), and a completed file gets a space after it for the next one.
pub fn complete(input: &str, several: bool) -> Completion {
    let unchanged = || Completion {
        input: input.to_string(),
        choices: Vec::new(),
    };
    let (start, quote) = last_path(input, several);
    let typed = unescape(&input[start..], quote);
    // The folder part as typed, and the start of the name in it.
    let split = typed.rfind(['/', MAIN_SEPARATOR]).map_or(0, |i| i + 1);
    let (folder, name) = typed.split_at(split);
    let Ok(dir) = config::parse_dir(if folder.is_empty() { &typed } else { folder }) else {
        return unchanged();
    };
    if folder.is_empty() {
        // Only `~`: it means the home folder.
        return match typed.as_str() {
            "~" => Completion {
                input: format!("{}{}", &input[..start], escape("~/", quote, several)),
                choices: Vec::new(),
            },
            _ => unchanged(),
        };
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return unchanged();
    };
    let mut found: Vec<(String, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            // Hidden files only when asked for.
            let shown = file.starts_with(name) && (name.starts_with('.') || !file.starts_with('.'));
            shown.then(|| (file, dir.join(e.file_name()).is_dir()))
        })
        .collect();
    found.sort();
    let complete = |rest: &str| {
        let path = escape(&format!("{folder}{rest}"), quote, several);
        format!("{}{path}", &input[..start])
    };
    match found.as_slice() {
        [] => unchanged(),
        [(file, true)] => Completion {
            input: complete(&format!("{file}/")),
            choices: Vec::new(),
        },
        [(file, false)] => {
            let mut input = complete(file);
            if let Some(q) = quote {
                input.push(q);
            }
            if several {
                input.push(' ');
            }
            Completion {
                input,
                choices: Vec::new(),
            }
        }
        _ => {
            let names: Vec<String> = found.iter().map(|(f, _)| f.clone()).collect();
            Completion {
                input: complete(&common_start(&names)),
                choices: found
                    .into_iter()
                    .map(|(file, dir)| if dir { format!("{file}/") } else { file })
                    .collect(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// A folder with `Blatt 1.pdf`, `Blatt 2.pdf`, `Briefe/`, `notes.txt` and `.hidden`.
    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ilias-tui-complete-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Briefe")).unwrap();
        for file in ["Blatt 1.pdf", "Blatt 2.pdf", "notes.txt", ".hidden"] {
            std::fs::write(dir.join(file), "x").unwrap();
        }
        dir
    }

    fn at(dir: &Path, rest: &str) -> String {
        format!("{}/{rest}", dir.display())
    }

    #[test]
    fn completes_the_only_match_and_folders_with_a_slash() {
        let dir = folder();
        let done = complete(&at(&dir, "no"), false);
        assert_eq!(done.input, at(&dir, "notes.txt"));
        assert!(done.choices.is_empty());
        assert_eq!(complete(&at(&dir, "Bri"), false).input, at(&dir, "Briefe/"));
        // Nothing fits, or not a full path: unchanged.
        assert_eq!(complete(&at(&dir, "zz"), false).input, at(&dir, "zz"));
        assert_eq!(complete("notes", false).input, "notes");
    }

    #[test]
    fn several_matches_complete_as_far_as_they_agree_and_are_listed() {
        let dir = folder();
        let done = complete(&at(&dir, "B"), false);
        assert_eq!(done.input, at(&dir, "B"));
        assert_eq!(done.choices, ["Blatt 1.pdf", "Blatt 2.pdf", "Briefe/"]);
        let done = complete(&at(&dir, "Bl"), false);
        assert_eq!(done.input, at(&dir, "Blatt "));
        assert_eq!(done.choices, ["Blatt 1.pdf", "Blatt 2.pdf"]);
        // Hidden files only when the name starts with a dot.
        assert!(
            !complete(&at(&dir, ""), false)
                .choices
                .contains(&".hidden".to_string())
        );
        assert_eq!(complete(&at(&dir, ".h"), false).input, at(&dir, ".hidden"));
    }

    #[test]
    #[cfg(not(windows))]
    fn several_paths_keep_their_escapes_and_quotes() {
        let dir = folder();
        let first = at(&dir, "notes.txt");
        // The second path is completed, escaped, and followed by a space for the next one.
        let done = complete(&format!("{first} {}", at(&dir, "Blatt\\ 1")), true);
        assert_eq!(
            done.input,
            format!("{first} {} ", at(&dir, "Blatt\\ 1.pdf"))
        );
        let done = complete(&format!("'{}", at(&dir, "Blatt 2")), true);
        assert_eq!(done.input, format!("'{}' ", at(&dir, "Blatt 2.pdf")));
        assert_eq!(
            common_start(&["Blätter".into(), "Blättchen".into()]),
            "Blätt"
        );
    }

    #[test]
    fn a_tilde_is_the_home_folder() {
        assert_eq!(complete("~", false).input, "~/");
    }
}
