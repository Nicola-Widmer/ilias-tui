//! My courses, the start page: favourites first, then the course list (newest year first),
//! with the To-Do beside them. Everything that goes into it lives here; callers only ask
//! for [`Client::my_courses`] (or [`Client::course_list`] for a deep search).

use std::collections::HashMap;
use std::thread;

use color_eyre::Result;
use jiff::civil::DateTime;

use super::parse::{self, Dashboard};
use super::{Block, Client, Id, Item, MEMBERSHIPS, Page, Todo, Urgency, parse_page};

/// The dashboard; only its favourites and "To-Do" blocks are used.
const DASHBOARD: &str = "ilias.php?baseClass=ilDashboardGUI";

impl Client {
    /// The complete start page. The course list and the dashboard are fetched at the same
    /// time; if the dashboard fails, the course list is still shown (without favourites and
    /// To-Do).
    pub fn my_courses(&self) -> Result<Page> {
        self.my_courses_after(None)
    }

    /// Like [`Client::my_courses`], taking the To-Do's courses from `previous` (an earlier
    /// My courses, e.g. the cached one) where it has them, so only new exercises are read.
    pub fn my_courses_after(&self, previous: Option<&Page>) -> Result<Page> {
        let known: HashMap<Id, String> = previous
            .into_iter()
            .flat_map(|page| &page.todo)
            .filter_map(|todo| Some((todo.assignment.id, todo.course.clone()?)))
            .collect();
        thread::scope(|scope| {
            let dashboard = scope.spawn(|| self.dashboard(&known));
            let courses = self.course_list_page()?;
            let dashboard = dashboard.join().ok().and_then(Result::ok);
            Ok(assemble(courses, dashboard, jiff::Zoned::now().datetime()))
        })
    }

    /// Your courses and groups, newest year first: where a deep search on My courses
    /// starts. The dashboard is not fetched for it.
    pub fn course_list(&self) -> Result<Vec<Item>> {
        Ok(newest_first(self.course_list_page()?)
            .items()
            .cloned()
            .collect())
    }

    fn course_list_page(&self) -> Result<Page> {
        let fetched = match self.take_checked_courses() {
            Some(fetched) => fetched,
            None => self.get_authed(&self.url(MEMBERSHIPS)?)?,
        };
        Ok(parse_page(&fetched.html, &fetched.url))
    }

    /// Favourites and To-Do from the dashboard, and the course of each To-Do exercise. The
    /// To-Do block only names the exercise, so each exercise's page is fetched (in parallel)
    /// for its course, unless `known` has it (an exercise stays in its course).
    fn dashboard(&self, known: &HashMap<Id, String>) -> Result<(Dashboard, HashMap<Id, String>)> {
        let fetched = self.get_authed(&self.url(DASHBOARD)?)?;
        let dashboard = parse::parse_dashboard(&fetched.html, &fetched.url);
        let mut exercises: Vec<Id> = dashboard
            .todo
            .iter()
            .map(|i| i.id)
            .filter(|id| !known.contains_key(id))
            .collect();
        exercises.sort_unstable();
        exercises.dedup();
        let mut courses: HashMap<Id, String> = thread::scope(|scope| {
            let jobs: Vec<_> = exercises
                .iter()
                .map(|&id| {
                    scope.spawn(move || -> Result<_> {
                        let page = self.get_authed(&self.url(&format!("goto.php/exc/{id}"))?)?;
                        Ok((id, parse::course_title(&page.html)))
                    })
                })
                .collect();
            jobs.into_iter()
                .filter_map(|job| match job.join() {
                    Ok(Ok((id, Some(course)))) => Some((id, course)),
                    _ => None,
                })
                .collect()
        });
        courses.extend(known.iter().map(|(id, course)| (*id, course.clone())));
        Ok((dashboard, courses))
    }
}

/// Builds My courses from the parsed course list and dashboard, as of `now`:
/// - the course list's year groups newest first,
/// - the favourites on top, with the full course names from the course list,
/// - the To-Do soonest first, with its course and how long is left.
fn assemble(
    courses: Page,
    dashboard: Option<(Dashboard, HashMap<Id, String>)>,
    now: DateTime,
) -> Page {
    let mut page = newest_first(courses);
    let Some((dashboard, todo_courses)) = dashboard else {
        return page;
    };
    if let Some(mut favorites) = dashboard.favorites {
        // Favourite tiles only carry the short title.
        if let Block::Items { items, .. } = &mut favorites {
            for favorite in items.iter_mut().filter(|f| f.description.is_empty()) {
                if let Some(known) = page.items().find(|i| i.id == favorite.id) {
                    favorite.description.clone_from(&known.description);
                }
            }
        }
        page.blocks.insert(0, favorites);
    }
    let mut todo: Vec<(Option<DateTime>, Todo)> = dashboard
        .todo
        .into_iter()
        .map(|assignment| {
            let deadline = parse::parse_date(&assignment.description, now.date());
            let (due, urgency) =
                deadline.map_or((String::new(), Urgency::Later), |d| due_in(now, d));
            let todo = Todo {
                course: todo_courses.get(&assignment.id).cloned(),
                deadline: assignment.description.clone(),
                due,
                urgency,
                assignment,
            };
            (deadline, todo)
        })
        .collect();
    // Unknown dates last.
    todo.sort_by_key(|(deadline, _)| (deadline.is_none(), *deadline));
    page.todo = todo.into_iter().map(|(_, todo)| todo).collect();
    page
}

/// Reverses the order of the year groups ("2024", "2025", …), leaving everything else in
/// place. ILIAS lists the oldest year first, but the current courses matter most.
fn newest_first(mut page: Page) -> Page {
    let is_year = |b: &Block| matches!(b, Block::Items { title, .. } if !title.is_empty() && title.chars().all(|c| c.is_ascii_digit()));
    let slots: Vec<usize> = (0..page.blocks.len())
        .filter(|&i| is_year(&page.blocks[i]))
        .collect();
    let mut years: Vec<Block> = slots.iter().map(|&i| page.blocks[i].clone()).collect();
    for &slot in &slots {
        page.blocks[slot] = years.pop().expect("one block per slot");
    }
    page
}

/// `due in 3 days`, `due in 5 hours`, `overdue`, and how urgent that is.
fn due_in(now: DateTime, deadline: DateTime) -> (String, Urgency) {
    let hours = now
        .until((jiff::Unit::Hour, deadline))
        .map_or(0, |span| span.get_hours());
    let text = match hours {
        ..0 => "overdue".into(),
        0 => "due in less than an hour".into(),
        1 => "due in 1 hour".into(),
        2..24 => format!("due in {hours} hours"),
        24..48 => "due in 1 day".into(),
        _ => format!("due in {} days", hours / 24),
    };
    let urgency = match hours {
        ..24 => Urgency::Now,
        24..96 => Urgency::Soon,
        _ => Urgency::Later,
    };
    (text, urgency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::datetime;
    use pretty_assertions::assert_eq;
    use url::Url;

    const COURSE_LIST: &str = r#"<main>
        <h1>Meine Kurse und Gruppen</h1>
        <div class="il-item-group"><h3>2025</h3><div class="il-item-group-items"><ul>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/101">DEMO.101</a></h4>
            <div class="il-item-description">Introduction to Examples</div></div></li></ul></div></div>
        <div class="il-item-group"><h3>2026</h3><div class="il-item-group-items"><ul>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/102">DEMO.102</a></h4>
            <div class="il-item-description">Advanced Examples</div></div></li></ul></div></div>
        </main>"#;

    const DASHBOARD_PAGE: &str = r#"<main>
        <div id="block_dash_fav_0"><h2>Favoriten</h2>
          <div class="il-card"><div class="caption card-title"><a href="https://ilias.example/ilias/goto.php/crs/102">DEMO.102</a></div></div></div>
        <div id="block_pdtasks_0"><ul>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/7/2">Abgabe zur Übungseinheit "Blatt 2"</a></h4>
            <div class="il-item-description">17. Jan 2027, 23:55</div></div></li>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/7/1">Abgabe zur Übungseinheit "Blatt 1"</a></h4>
            <div class="il-item-description">5. Nov 2026, 00:00</div></div></li>
        </ul></div></main>"#;

    fn parsed() -> (Page, Dashboard) {
        let url = Url::parse("https://ilias.example/ilias/ilias.php").unwrap();
        (
            parse_page(COURSE_LIST, &url),
            parse::parse_dashboard(DASHBOARD_PAGE, &url),
        )
    }

    fn titles(page: &Page) -> Vec<String> {
        page.blocks
            .iter()
            .map(|b| match b {
                Block::Heading(t) | Block::Items { title: t, .. } => t.clone(),
                _ => String::new(),
            })
            .collect()
    }

    /// The course of every To-Do exercise in `DASHBOARD_PAGE`.
    fn courses_of_todo() -> HashMap<Id, String> {
        HashMap::from([(Id::Ref(7), "DEMO.102".to_string())])
    }

    #[test]
    fn puts_favourites_first_and_the_newest_year_next() {
        let (courses, dashboard) = parsed();
        let page = assemble(
            courses,
            Some((dashboard, courses_of_todo())),
            datetime(2026, 11, 1, 14, 0, 0, 0),
        );
        assert_eq!(
            titles(&page),
            ["Favoriten", "Meine Kurse und Gruppen", "2026", "2025"]
        );
        let Block::Items { items, .. } = &page.blocks[0] else {
            panic!("favourites first");
        };
        assert_eq!(
            (items[0].title.as_str(), items[0].description.as_str()),
            ("DEMO.102", "Advanced Examples"),
            "a favourite gets its full name from the course list"
        );
    }

    #[test]
    fn lists_the_todo_soonest_first_with_the_time_left() {
        let (courses, dashboard) = parsed();
        let page = assemble(
            courses,
            Some((dashboard, courses_of_todo())),
            datetime(2026, 11, 1, 14, 0, 0, 0),
        );
        let todo: Vec<_> = page
            .todo
            .iter()
            .map(|t| {
                (
                    t.assignment.title.as_str(),
                    t.due.as_str(),
                    t.deadline.as_str(),
                    t.urgency,
                    t.course.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            todo,
            [
                (
                    "Blatt 1",
                    "due in 3 days",
                    "5. Nov 2026, 00:00",
                    Urgency::Soon,
                    Some("DEMO.102")
                ),
                (
                    "Blatt 2",
                    "due in 77 days",
                    "17. Jan 2027, 23:55",
                    Urgency::Later,
                    Some("DEMO.102")
                ),
            ]
        );
        assert!(
            page.items().all(|i| i.kind.code() != "exc"),
            "To-Do is beside the page"
        );
    }

    #[test]
    fn without_the_dashboard_shows_the_course_list() {
        let (courses, _) = parsed();
        let page = assemble(courses, None, datetime(2026, 11, 1, 14, 0, 0, 0));
        assert_eq!(titles(&page), ["Meine Kurse und Gruppen", "2026", "2025"]);
        assert!(page.todo.is_empty());
    }

    #[test]
    fn keeps_other_groups_in_place() {
        let group = |title: &str| Block::Items {
            title: title.into(),
            items: vec![Item::default()],
        };
        let page = Page {
            blocks: vec![
                group("2024"),
                group("2025"),
                group("2026"),
                group("Von Hand erstellte Kurse"),
            ],
            ..Page::default()
        };
        assert_eq!(
            titles(&newest_first(page)),
            ["2026", "2025", "2024", "Von Hand erstellte Kurse"]
        );
    }

    #[test]
    fn says_when_something_is_due() {
        let now = datetime(2026, 11, 1, 14, 0, 0, 0);
        let due = |y, m, d, h| due_in(now, datetime(y, m, d, h, 0, 0, 0));
        assert_eq!(due(2026, 11, 5, 0), ("due in 3 days".into(), Urgency::Soon));
        assert_eq!(
            due(2026, 11, 2, 13),
            ("due in 23 hours".into(), Urgency::Now)
        );
        assert_eq!(due(2026, 11, 2, 14), ("due in 1 day".into(), Urgency::Soon));
        assert_eq!(
            due(2026, 11, 8, 14),
            ("due in 7 days".into(), Urgency::Later)
        );
        assert_eq!(
            due(2026, 11, 1, 14),
            ("due in less than an hour".into(), Urgency::Now)
        );
        assert_eq!(due(2026, 10, 31, 0), ("overdue".into(), Urgency::Now));
    }

    #[test]
    fn my_courses_end_to_end() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        let ilias = Arc::new(FakeIlias::new(|url| {
            let query = url.query().unwrap_or("");
            let html = if query.contains("ilmembershipoverviewgui") {
                COURSE_LIST
            } else if query.contains("ilDashboardGUI") {
                DASHBOARD_PAGE
            } else if url.path().ends_with("goto.php/exc/7") {
                r#"<div class="breadcrumb"><a href="https://ilias.example/ilias/goto.php/crs/102">DEMO.102</a></div>"#
            } else {
                return Err(color_eyre::eyre::eyre!("unexpected {url}"));
            };
            reply(url, html)
        }));
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::with_pages(base.clone(), ilias.clone());
        let page = client.my_courses().unwrap();
        assert_eq!(titles(&page)[0], "Favoriten");
        assert_eq!(page.todo.len(), 2);
        assert!(
            page.todo
                .iter()
                .all(|t| t.course.as_deref() == Some("DEMO.102"))
        );
        let mut requests = ilias.requests(&base);
        requests.sort();
        assert_eq!(
            requests,
            [
                "goto.php/exc/7",
                "ilias.php?baseClass=ilDashboardGUI",
                "ilias.php?baseClass=ilmembershipoverviewgui"
            ],
            "one request per exercise, however many assignments"
        );

        // Right after the session check, with the previous My courses: the check's course
        // list and the known courses are used, so only the dashboard is read.
        assert!(client.check_session().unwrap());
        let before = ilias.requests(&base).len();
        let again = client.my_courses_after(Some(&page)).unwrap();
        assert_eq!(again.todo, page.todo);
        assert_eq!(
            ilias.requests(&base)[before..],
            ["ilias.php?baseClass=ilDashboardGUI"]
        );
        // The check's page is used once; later refreshes read the course list again.
        client.my_courses_after(Some(&page)).unwrap();
        assert!(
            ilias.requests(&base)[before + 1..]
                .contains(&"ilias.php?baseClass=ilmembershipoverviewgui".to_string())
        );
    }

    #[test]
    fn my_courses_without_a_working_dashboard() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        let ilias = Arc::new(FakeIlias::new(|url| match url.query() {
            Some(q) if q.contains("ilmembershipoverviewgui") => reply(url, COURSE_LIST),
            _ => Err(color_eyre::eyre::eyre!("500 Internal Server Error")),
        }));
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let page = Client::with_pages(base, ilias).my_courses().unwrap();
        assert_eq!(titles(&page), ["Meine Kurse und Gruppen", "2026", "2025"]);
        assert!(page.todo.is_empty());
    }
}
