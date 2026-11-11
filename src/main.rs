mod action;
mod app;
mod auth;
mod cache;
mod clipboard;
mod components;
mod compose;
mod config;
mod icons;
mod ilias;
mod keymap;
mod open;
mod seen;
mod session;
mod sync;
mod theme;

use color_eyre::{Result, eyre::bail};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use url::Url;

use crate::ilias::Client;

/// The version, from `Cargo.toml` (semantic versioning; see CHANGELOG.md).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
ilias-tui — browse ILIAS courses from the terminal

USAGE:
    ilias-tui            Start the TUI (logs in first if needed; starts offline
                         with cached pages if ILIAS can't be reached)
    ilias-tui --offline  Start the TUI offline: cached pages and saved files only
    ilias-tui login      Force a fresh browser login
    ilias-tui logout     Delete the saved session file
    ilias-tui show URL   Print the parsed page (tabs, headings, text, items)
    ilias-tui dump URL   Print the HTML ILIAS returns for URL (for debugging)
    ilias-tui ls [URL] [--deep]
                         Print the items of a container (default: my courses),
                         with --deep everything below it (tab-separated)
    ilias-tui get URL…   Download files (e.g. goto.php/file/123) to the download dir
    ilias-tui todo       Print assignments still to hand in, soonest first
    ilias-tui --version  Print the version

ENVIRONMENT (each wins over the settings page, c):
    ILIAS_URL            ILIAS root URL (default: https://elearning.hslu.ch/ilias/)
    ILIAS_DOWNLOAD_DIR   Where files are saved (default: ~/Downloads/ILIAS)
    ILIAS_NERD_FONT=0    Plain Unicode icons instead of Nerd Font glyphs
    ILIAS_MOUSE=0        Turn off mouse support
    ILIAS_CACHE=0        Turn off the page cache
    ILIAS_SYNC_MAX_MB    Sync skips files bigger than this (default: 50)
    ILIAS_OFFLINE=1      Same as --offline";

fn main() -> Result<()> {
    color_eyre::install()?;
    let config = config::Config::load();
    let base = Url::parse(&config.url())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--offline"] => {
            let offline = args.first().is_some_and(|a| a == "--offline") || config.offline();
            // The session check runs in the TUI, which starts at once with cached pages.
            let client = Client::new(base.clone(), session::load(&base))?;
            client.set_offline(offline);
            let mut terminal = ratatui::init();
            let mouse = config.mouse();
            if mouse {
                execute!(std::io::stdout(), EnableMouseCapture)?;
                // ratatui's own panic hook restores the screen; also give the mouse back.
                let hook = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                    hook(info);
                }));
            }
            let result = app::App::new(client, config).run(&mut terminal);
            if mouse {
                execute!(std::io::stdout(), DisableMouseCapture)?;
            }
            ratatui::restore();
            result
        }
        ["login"] => {
            connect(&base, true)?;
            println!("Logged in to {base}");
            Ok(())
        }
        ["logout"] => {
            let client = Client::new(base.clone(), session::load(&base))?;
            match client.logout() {
                Ok(true) => println!("Ended your ILIAS session on the server."),
                Ok(false) => println!("No active ILIAS session on the server."),
                Err(e) => eprintln!("Could not log out on the server: {e:#}"),
            }
            if client.login_server_session_active().unwrap_or(false) {
                println!(
                    "Your HSLU login (Keycloak) is still active on the server and would log you in again.\n\
                     To end it, log out of ILIAS / Switch edu-ID in your browser, or let it expire."
                );
            }
            session::clear(&base)?;
            cache::Cache::new(&base).clear();
            println!("Removed the saved session and page cache for {base}");
            Ok(())
        }
        ["show", url] => {
            let client = connect(&base, false)?;
            let fetched = client.get_authed(&base.join(url)?)?;
            let page = ilias::parse_page(&fetched.html, &fetched.url);
            let tabs: Vec<String> = page
                .tabs
                .iter()
                .map(|t| {
                    if t.active {
                        format!("[{}]", t.title)
                    } else {
                        t.title.clone()
                    }
                })
                .collect();
            println!("tabs: {}", tabs.join(" | "));
            if !page.rights.is_empty() {
                println!("rights: {}", page.rights);
            }
            for block in &page.blocks {
                match block {
                    ilias::Block::Heading(text) => println!("\n## {text}"),
                    ilias::Block::Text(text) => println!("{text}"),
                    ilias::Block::Post(post) => {
                        let new = if post.unread { " [new]" } else { "" };
                        let indent = "  ".repeat(post.depth);
                        println!(
                            "\n{indent}{} | {}{new}\n{indent}{}",
                            post.author, post.date, post.title
                        );
                        for paragraph in &post.body {
                            println!("{indent}  {paragraph}");
                        }
                    }
                    ilias::Block::Items { title, items } => {
                        println!("\n## {title}");
                        for i in items {
                            println!(
                                "  [{}] {}  ({} | {})",
                                i.kind,
                                i.title,
                                i.description,
                                i.properties.join(", ")
                            );
                        }
                    }
                }
            }
            Ok(())
        }
        ["dump", url] => {
            let client = connect(&base, false)?;
            print!("{}", client.get_authed(&base.join(url)?)?.html);
            Ok(())
        }
        ["ls", rest @ ..] => {
            let deep = rest.contains(&"--deep");
            let item = match rest.iter().find(|a| **a != "--deep") {
                Some(url) => Some(item_from_url(&base, url)?),
                None => None,
            };
            let client = connect(&base, false)?;
            let items = match (&item, deep) {
                (_, true) => client.descendants(item.as_ref(), |_| {})?,
                (Some(item), false) => client.children(item)?,
                (None, false) => client.course_list()?,
            };
            for i in items {
                println!("{}\t{}\t{}", i.kind, i.id, i.title);
            }
            Ok(())
        }
        ["get", urls @ ..] if !urls.is_empty() => {
            let client = connect(&base, false)?;
            let dir = config.download_dir();
            let mut saved = sync::model::Base::load(&base, &dir);
            for url in urls {
                let item = item_from_url(&base, url)?;
                let path = client.download(&item, &dir)?;
                saved.record(&path, item.key(), sync::version(&item))?;
                println!("{}", path.display());
            }
            Ok(())
        }
        ["todo"] => {
            let client = connect(&base, false)?;
            for todo in client.my_courses()?.todo {
                println!(
                    "{} · {}\t{}\t{}",
                    todo.due,
                    todo.deadline,
                    todo.assignment.title,
                    todo.course.unwrap_or_default()
                );
            }
            Ok(())
        }
        ["-V" | "--version" | "version"] => {
            println!("ilias-tui {VERSION}");
            Ok(())
        }
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("unknown arguments\n\n{USAGE}"),
    }
}

/// Builds an [`ilias::Item`] from a link or a path relative to the ILIAS root.
fn item_from_url(base: &Url, url: &str) -> Result<ilias::Item> {
    let url = base.join(url)?;
    let Some((kind, id)) = ilias::parse_target(&url) else {
        bail!("not an ILIAS object link: {url}");
    };
    Ok(ilias::Item {
        title: format!("{kind}_{id}"),
        kind,
        id,
        url: url.into(),
        ..Default::default()
    })
}

/// Restores the saved session, refreshing it silently via Keycloak, and falls back to
/// the interactive browser login.
fn connect(base: &Url, force_login: bool) -> Result<Client> {
    let client = Client::new(base.clone(), session::load(base))?;
    if force_login || !client.check_session()? {
        eprintln!("Opening Chrome for the ILIAS / Switch edu-ID login…");
        client.set_cookies(auth::browser_login(base)?);
        if !client.check_session()? {
            bail!("login finished, but ILIAS does not recognise the session");
        }
    }
    session::save(base, &client.cookies())?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    /// Every version has its entry in the changelog.
    #[test]
    fn the_changelog_has_this_version() {
        let changelog = include_str!("../CHANGELOG.md");
        let heading = format!("\n## {} - ", super::VERSION);
        assert!(
            changelog.contains(&heading),
            "CHANGELOG.md has no section for {}",
            super::VERSION
        );
    }
}
