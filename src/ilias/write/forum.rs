//! Writes for forums: starting a thread and replying to a post, through ILIAS' own forms.
//! Everyone who can read the forum can read what is posted.

use std::sync::atomic::Ordering;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use url::Url;

use super::super::pages::Form;
use super::super::{Block, Client, Id, Item, Kind, parse};
use parse::param;

/// The start of what ILIAS shows of a post's text, to find it again.
fn start_of(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(6).collect();
    words.join(" ")
}

impl Client {
    /// Starts a thread `title` with the post `text` in `forum`, through its "New Thread" form.
    /// Returns `false` in dry-run mode, where nothing is sent.
    pub fn start_thread(&self, forum: &Item, title: &str, text: &str) -> Result<bool> {
        let Id::Ref(ref_id) = forum.id else {
            bail!("{} is not a forum", forum.title);
        };
        let (title, text) = (title.trim(), text.trim());
        if title.is_empty() || text.is_empty() {
            bail!("a thread needs a title and a text");
        }
        let own = ref_id.to_string();
        // Only ever send what this function is for: a thread in this very forum.
        let in_forum = |url: &Url, command: &str| {
            param(url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case("ilObjForumGUI"))
                && param(url, "ref_id").as_deref() == Some(own.as_str())
                && (param(url, "cmd").as_deref() == Some(command)
                    || param(url, "fallbackCmd").as_deref() == Some(command))
        };
        let page = self.get_authed(&Url::parse(&forum.url)?)?;
        let link = parse::forum_link(&page.html, &page.url, |url| in_forum(url, "createThread"))
            .ok_or_else(|| eyre!("ILIAS doesn't let you start a thread in {}", forum.title))?;
        let screen = self.get_authed(&link)?;
        let form = parse::post_form(&screen.html, &screen.url, "addThread", |url| {
            in_forum(url, "addThread")
        })
        .ok_or_else(|| eyre!("unexpected form for a new thread in {}", forum.title))?;
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }

        self.send_post(&form, title, text)?;
        let listed = self
            .page(forum, None)?
            .items()
            .any(|i| i.kind == Kind::Thread && i.title == title);
        if !listed {
            bail!(
                "sent, but {} doesn't list the thread {title}: check it in ILIAS",
                forum.title
            );
        }
        Ok(true)
    }

    /// Replies with `text` to post `post` of `thread`, through that post's Reply form. The
    /// subject stays what ILIAS suggests. Returns `false` in dry-run mode, where nothing is
    /// sent.
    pub fn reply(&self, thread: &Item, post: u64, text: &str) -> Result<bool> {
        let Id::Thread(thread_id) = thread.id else {
            bail!("{} is not a forum thread", thread.title);
        };
        let text = text.trim();
        if text.is_empty() {
            bail!("a reply needs a text");
        }
        let url = Url::parse(&thread.url)?;
        let (forum, thread_id, post_id) = (
            param(&url, "ref_id"),
            thread_id.to_string(),
            post.to_string(),
        );
        // Only ever send what this function is for: a reply to this post in this thread.
        let this_post = |url: &Url| {
            param(url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case("ilObjForumGUI"))
                && forum.is_some()
                && param(url, "ref_id") == forum
                && param(url, "thr_pk").as_deref() == Some(thread_id.as_str())
                && param(url, "pos_pk").as_deref() == Some(post_id.as_str())
        };
        let page = self.get_authed(&url)?;
        let link = parse::forum_link(&page.html, &page.url, |url| {
            this_post(url)
                && param(url, "cmd").as_deref() == Some("viewThread")
                && param(url, "action").as_deref() == Some("showreply")
        })
        .ok_or_else(|| eyre!("ILIAS doesn't let you reply to this post"))?;
        let screen = self.get_authed(&link)?;
        let form = parse::post_form(&screen.html, &screen.url, "savePost", |url| {
            this_post(url) && param(url, "fallbackCmd").as_deref() == Some("savePost")
        })
        .ok_or_else(|| eyre!("unexpected reply form in {}", thread.title))?;
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }

        let subject = form.subject.1.clone();
        self.send_post(&form, &subject, text)?;
        let start = start_of(text);
        let listed = self.page(thread, None)?.blocks.iter().any(
            |b| matches!(b, Block::Post(p) if p.body.iter().any(|line| line.contains(&start))),
        );
        if !listed {
            bail!(
                "sent, but {} doesn't show the reply: check it in ILIAS",
                thread.title
            );
        }
        Ok(true)
    }

    /// Posts a forum form as the browser does: its hidden fields, subject, message, button.
    fn send_post(&self, form: &parse::PostForm, subject: &str, text: &str) -> Result<()> {
        let message = match form.rich {
            true => parse::message_html(text),
            false => text.to_string(),
        };
        let mut fields: Vec<Form> = form
            .hidden
            .iter()
            .map(|(name, value)| Form::Field { name, value })
            .collect();
        fields.extend([
            Form::Field {
                name: &form.subject.0,
                value: subject,
            },
            Form::Field {
                name: &form.message,
                value: &message,
            },
        ]);
        fields.extend(form.files.iter().map(|name| Form::EmptyFile { name }));
        fields.push(Form::Field {
            name: &form.button.0,
            value: &form.button.1,
        });
        self.ensure_online()?;
        self.request(&form.action, Some(&fields), true)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::ilias::pages::{FakeIlias, reply as answer};

    fn base() -> Url {
        Url::parse("https://ilias.example/ilias/").unwrap()
    }

    fn forum() -> Item {
        Item {
            kind: Kind::Forum,
            id: Id::Ref(101),
            title: "Fragen".into(),
            url: "https://ilias.example/ilias/goto.php/frm/101".into(),
            ..Item::default()
        }
    }

    fn thread() -> Item {
        Item {
            kind: Kind::Thread,
            id: Id::Thread(3),
            title: "Termin".into(),
            url: "https://ilias.example/ilias/ilias.php?cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=101&thr_pk=3".into(),
            ..Item::default()
        }
    }

    /// A post form as the local ILIAS 10 renders it (shortened), posting to `action`.
    fn post_form(action: &str, button: &str, subject: &str) -> String {
        format!(
            r#"<form class="form-horizontal" enctype="multipart/form-data" action="{}" method="post">
            <input type="hidden" name="alias" value="student" />
            <input class="form-control" type="text" id="subject" maxlength="255" name="subject" value="{subject}" />
            <textarea class="form-control  RTEditor" name="message" id="message"></textarea>
            <input type="file" class="form-control" id="userfile[0]" name="userfile[0]" />
            <input type="hidden" id="ilfilehash" name="ilfilehash" value="h" />
            <input class="btn btn-default" type="submit" name="cmd[{button}]" value="Save" />
            <input class="btn btn-default" type="submit" name="cmd[cancelPost]" value="Cancel" /></form>"#,
            action.replace('&', "&amp;")
        )
    }

    const CREATE: &str =
        "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=createThread&ref_id=101";
    const ADD: &str = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=addThread&ref_id=101&rtoken=t";

    /// Forum 101 with "New Thread" going to `create` (if any), its form posting to `add`;
    /// after a post it lists the thread "Termin".
    fn forum_ilias(create: Option<&'static str>, add: &'static str) -> Arc<FakeIlias> {
        let posted = Arc::new(AtomicBool::new(false));
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("fallbackCmd=") {
                posted.store(true, Ordering::SeqCst);
                return answer(url, "<p>New topic has been created</p>");
            }
            if url.as_str().contains("cmd=createThread") {
                return answer(url, &post_form(add, "addThread", ""));
            }
            let button = create.map_or(String::new(), |c| {
                format!(
                    r#"<button class="btn btn-default" data-action="{}">New Thread</button>"#,
                    c.replace('&', "&amp;")
                )
            });
            let listed = match posted.load(Ordering::SeqCst) {
                true => {
                    r#"<div class="il-std-item"><div class="il-item-title"><a href="ilias.php?cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=101&amp;thr_pk=3">Termin</a></div></div>"#
                }
                false => "",
            };
            answer(
                url,
                &format!(r#"{button}<div id="il_center_col">{listed}</div>"#),
            )
        }))
    }

    #[test]
    fn starts_a_thread_through_the_forums_form() {
        let ilias = forum_ilias(Some(CREATE), ADD);
        let client = Client::with_pages(base(), ilias.clone());
        assert!(
            client
                .start_thread(&forum(), "Termin", "Wann?\n\nIch <frage> & so")
                .unwrap()
        );
        assert_eq!(
            ilias.posts(&base()),
            [format!(
                "{ADD} <- alias=student&ilfilehash=h&subject=Termin\
                 &message=<p>Wann?</p>\n<p>Ich &lt;frage&gt; &amp; so</p>&userfile[0]=@&cmd[addThread]=Save"
            )]
        );
    }

    #[test]
    fn a_dry_run_starts_no_thread() {
        let ilias = forum_ilias(Some(CREATE), ADD);
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.start_thread(&forum(), "Termin", "Wann?").unwrap());
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn starts_no_thread_but_in_this_forum() {
        for (create, add) in [
            (None, ADD),
            // "New Thread" of another forum, or on another site.
            (
                Some("ilias.php?cmdClass=ilObjForumGUI&cmd=createThread&ref_id=102"),
                ADD,
            ),
            (
                Some(
                    "https://evil.example/ilias/ilias.php?cmdClass=ilObjForumGUI&cmd=createThread&ref_id=101",
                ),
                ADD,
            ),
            // The form posts to another forum, another site or another command.
            (
                Some(CREATE),
                "ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=addThread&ref_id=102",
            ),
            (
                Some(CREATE),
                "https://evil.example/ilias/ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=addThread&ref_id=101",
            ),
            (
                Some(CREATE),
                "ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=deleteThread&ref_id=101",
            ),
        ] {
            let ilias = forum_ilias(create, add);
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.start_thread(&forum(), "Termin", "Wann?").is_err(),
                "{create:?} {add}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{create:?} {add}");
        }
        let client = Client::with_pages(base(), forum_ilias(Some(CREATE), ADD));
        assert!(
            client.start_thread(&forum(), "Termin", " ").is_err(),
            "no text"
        );
    }

    const SHOW: &str = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=101&action=showreply&pos_pk=6&thr_pk=3";
    const SAVE: &str = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=savePost&ref_id=101&action=ready_showreply&pos_pk=6&thr_pk=3&rtoken=t";

    /// Thread 3 with post 6 offering Reply via `show`, the reply form posting to `save`; after
    /// a post it shows the reply.
    fn thread_ilias(show: &'static str, save: &'static str) -> Arc<FakeIlias> {
        let posted = Arc::new(AtomicBool::new(false));
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("fallbackCmd=savePost") {
                posted.store(true, Ordering::SeqCst);
            }
            let form = match url.as_str().contains("action=showreply") {
                true => post_form(save, "savePost", "Termin"),
                false => String::new(),
            };
            let reply = match posted.load(Ordering::SeqCst) {
                true => {
                    r#"<li class="ilFrmPostRow ilFrmPost-level-2"><a id="7"></a><div class="ilFrmPostContent"><p>Am 15. Januar, siehe Modulbeschrieb.</p></div></li>"#
                }
                false => "",
            };
            answer(
                url,
                &format!(
                    r#"<ul><li class="ilFrmPostRow ilFrmPost-level-1"><a id="6"></a>
                    <div class="ilFrmPostContent"><p>Wann?</p></div>
                    <button class="btn btn-default" data-action="{}">Reply</button></li>{reply}</ul>{form}"#,
                    show.replace('&', "&amp;")
                ),
            )
        }))
    }

    #[test]
    fn replies_to_a_post_through_its_reply_form() {
        let ilias = thread_ilias(SHOW, SAVE);
        let client = Client::with_pages(base(), ilias.clone());
        assert!(
            client
                .reply(&thread(), 6, "Am 15. Januar, siehe Modulbeschrieb.")
                .unwrap()
        );
        assert_eq!(
            ilias.posts(&base()),
            [format!(
                "{SAVE} <- alias=student&ilfilehash=h&subject=Termin\
                 &message=<p>Am 15. Januar, siehe Modulbeschrieb.</p>&userfile[0]=@&cmd[savePost]=Save"
            )]
        );
    }

    #[test]
    fn a_dry_run_replies_nothing() {
        let ilias = thread_ilias(SHOW, SAVE);
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.reply(&thread(), 6, "Danke").unwrap());
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn replies_to_nothing_but_this_post() {
        // No Reply for post 5 on the page.
        let ilias = thread_ilias(SHOW, SAVE);
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.reply(&thread(), 5, "Danke").is_err());
        assert!(ilias.posts(&base()).is_empty());
        for (show, save) in [
            // Reply of another thread's post, or on another site.
            (
                "ilias.php?cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=101&action=showreply&pos_pk=6&thr_pk=4",
                SAVE,
            ),
            (
                "https://evil.example/ilias/ilias.php?cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=101&action=showreply&pos_pk=6&thr_pk=3",
                SAVE,
            ),
            // The form posts for another post, another forum or another command.
            (
                SHOW,
                "ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=savePost&ref_id=101&pos_pk=5&thr_pk=3",
            ),
            (
                SHOW,
                "ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=savePost&ref_id=102&pos_pk=6&thr_pk=3",
            ),
            (
                SHOW,
                "ilias.php?cmdClass=ilObjForumGUI&cmd=post&fallbackCmd=deletePost&ref_id=101&pos_pk=6&thr_pk=3",
            ),
        ] {
            let ilias = thread_ilias(show, save);
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.reply(&thread(), 6, "Danke").is_err(),
                "{show} {save}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{show} {save}");
        }
    }
}
