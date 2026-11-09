use super::*;
use pretty_assertions::assert_eq;

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn finds_show_more_buttons() {
    // As on the local ILIAS 10: the script twice, once escaped inside JSON.
    let html = r#"<div class="ilContainerBlock"><ul class="ilContainerItemsContainer">
        <li><div class="ilContainerListItemOuter" data-list-item-id="lg_div_110_pref_109"></div></li>
        <li><div class="ilContainerListItemOuter" data-list-item-id="lg_div_111_pref_109"></div></li>
        <li class="ilContainerShowMore"><button class="btn btn-default" data-action="" id="il_ui_fw_1">Show More</button></li>
        </ul></div>
        <script>il.Container.initShowMore('il_ui_fw_1', '_other', 'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch');</script>
        <script>var x = {"code": "il.Container.initShowMore(\'il_ui_fw_1\', \'_other\', \'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch\')"};</script>"#;
    let found = show_more(html, &url("https://ilias.example/ilias/goto.php/fold/109"));
    assert_eq!(
        found,
        [ShowMore {
            url: url(
                "https://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch"
            ),
            shown: vec!["lg_div_110_pref_109".into(), "lg_div_111_pref_109".into()],
        }]
    );
    assert_eq!(found[0].last_shown(), Some(Id::Ref(111)));
    assert!(has_show_more(html));
    assert!(show_more("<p>no button</p>", &url("https://ilias.example/ilias/")).is_empty());
}

#[test]
fn permalinks_to_share() {
    let base = url("https://ilias.example/ilias/");
    let item = |kind: &str, id: Id, link: &str| Item {
        kind: kind.into(),
        id,
        url: link.into(),
        ..Item::default()
    };
    let sendfile =
        "https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&cmd=sendfile&ref_id=5";
    assert_eq!(
        item("file", Id::Ref(5), sendfile).permalink(&base),
        "https://ilias.example/ilias/goto.php/file/5"
    );
    assert_eq!(
        item("crs", Id::Ref(1), "x").permalink(&base),
        "https://ilias.example/ilias/goto.php/crs/1"
    );
    // Only reference ids have permalinks; the rest share their own link.
    let thread = "https://ilias.example/ilias/ilias.php?cmd=viewThread&thr_pk=42";
    let zoom = "https://zoom.us/j/1";
    let attachment = "https://ilias.example/ilias/ilias.php?cmd=downloadFile";
    for own in [
        item("thr", Id::Thread(42), thread),
        item("link", Id::local(zoom), zoom),
        item("file", Id::local(attachment), attachment),
    ] {
        assert_eq!(own.permalink(&base), own.url);
    }
}

#[test]
fn parses_permalinks() {
    let base = "https://elearning.hslu.ch/ilias/";
    assert_eq!(
        parse_target(&url(&format!("{base}goto.php/crs/3035013"))),
        Some(("crs".into(), Id::Ref(3035013)))
    );
    assert_eq!(
        parse_target(&url(&format!("{base}goto.php?target=file_42_download"))),
        Some(("file".into(), Id::Ref(42)))
    );
    assert_eq!(
        parse_target(&url(&format!(
            "{base}ilias.php?baseClass=ilrepositorygui&cmdNode=yb:o1&cmdClass=ilObjFileGUI&cmd=sendfile&ref_id=123456"
        ))),
        Some(("file".into(), Id::Ref(123456)))
    );
    assert_eq!(
        parse_target(&url(&format!(
            "{base}ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=123456&thr_pk=42&page=0"
        ))),
        Some(("thr".into(), Id::Thread(42)))
    );
    assert_eq!(
        parse_target(&url(&format!("{base}ilias.php?ref_id=1"))),
        None
    );
}

#[test]
fn finds_the_logout_link() {
    let html = r#"<a href="https://ilias.example/ilias/ilias.php?baseClass=ilstartupgui&amp;cmd=doLogout&amp;rtoken=abc&amp;lang=de">Abmelden</a>"#;
    let link = logout_link(html, &url("https://ilias.example/ilias/ilias.php")).unwrap();
    assert!(link.as_str().contains("cmd=doLogout&rtoken=abc"));
    assert_eq!(
        logout_link("<a href='x'>y</a>", &url("https://ilias.example/")),
        None
    );
}

#[test]
fn parses_root_page_without_breadcrumbs() {
    let page_url = url("https://elearning.hslu.ch/ilias/goto.php/root/1");
    let page = parse_page(include_str!("../../../tests/fixtures/root.html"), &page_url);
    let items: Vec<_> = page
        .items()
        .map(|i| (i.kind.code(), i.key(), i.title.as_str()))
        .collect();
    assert_eq!(items, vec![("cat", 5066130, "Öffentlicher Bereich")]);
}

#[test]
fn parses_category_contents() {
    let page_url = url("https://elearning.hslu.ch/ilias/goto.php/cat/5066132");
    let page = parse_page(
        include_str!("../../../tests/fixtures/category.html"),
        &page_url,
    );
    let items: Vec<_> = page.items().collect();
    assert!(items.len() > 3, "found {items:?}");
    assert!(
        items
            .iter()
            .all(|i| i.kind.code() == "grp" && !i.title.is_empty())
    );
    assert!(
        items
            .iter()
            .all(|i| i.key() != 5066130 && i.key() != 5066132)
    );
}

#[test]
fn lists_the_assignments_of_an_exercise_with_their_modes_as_tabs() {
    let base = Url::parse("https://ilias.example/ilias/").unwrap();
    let html = r##"<ul id="ilTab">
          <li id="tab_content" class="active"><a href="ilias.php?cmd=showOverview&amp;ref_id=7">Assignments</a></li>
          <li id="tab_info"><a href="ilias.php?cmd=showSummary&amp;ref_id=7">Info</a></li></ul>
        <div id="il_center_col">
          <div class="il-viewcontrol-mode">
            <button class="btn engaged" aria-pressed="true" data-action="ilias.php?baseClass=ilexercisehandlergui&amp;mode=ongoing&amp;ref_id=7">Ongoing</button>
            <button class="btn" aria-pressed="false" data-action="ilias.php?baseClass=ilexercisehandlergui&amp;mode=past&amp;ref_id=7">Past</button>
          </div>
          <div class="il-item-group"><h3>Ongoing</h3><ul><li class="il-std-item-container"><div class="il-item il-std-item">
            <div class="row"><div class="col-sm-3">2 Days to Submit</div><div class="col-sm-9">
              <h4 class="il-item-title"><a href="ilias.php?baseClass=ilexercisehandlergui&amp;cmdClass=ilAssignmentPresentationGUI&amp;ref_id=7&amp;ass_id=7">Blatt 1</a></h4>
              <div class="il-item-actions"><button data-action="ilias.php?cmd=submissionScreen&amp;ref_id=7&amp;ass_id=7">Hand In</button></div>
              <span class="il-item-property-name">Edit Until</span><span class="il-item-property-value">9. Nov 2026, 17:47</span>
              <span class="il-item-property-name">Last Submission</span><span class="il-item-property-value">No submission yet.</span>
            </div></div></div></li></ul></div>
        </div>"##;
    let mut page = parse_page(html, &base);
    let titles: Vec<(&str, bool)> = page
        .tabs
        .iter()
        .map(|t| (t.title.as_str(), t.active))
        .collect();
    assert_eq!(
        titles,
        [("Ongoing", true), ("Past", false), ("Info", false)]
    );
    assert!(page.tabs[1].url.contains("mode=past"));

    // The assignment has the same number as the exercise's reference id; it stays.
    page.remove(Id::Ref(7));
    let assignment = page.items().next().unwrap();
    assert_eq!(assignment.kind.code(), "ass");
    assert_eq!(assignment.key(), 7);
    assert_eq!(assignment.title, "Blatt 1");
    assert_eq!(assignment.description, "2 Days to Submit");
    assert_eq!(
        assignment.properties,
        [
            "Edit Until: 9. Nov 2026, 17:47",
            "Last Submission: No submission yet."
        ]
    );
    assert!(assignment.is_container());
}

#[test]
fn lists_handed_in_files_and_finds_their_delete_form() {
    let base = Url::parse("https://ilias.example/ilias/ilias.php?cmd=submissionScreen").unwrap();
    let html = r#"<h3>Submitted Files - Laborbericht</h3>
        <form action="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=post&amp;ref_id=7&amp;ass_id=3">
        <table><tr><th></th><th>File Name</th><th>Date</th><th>Action</th></tr>
        <tr class="tblrow1">
          <td class="std"><input type="checkbox" id="deli_2" name="delivered[]" value="2" /></td>
          <td class="std"><label for="deli_2">Bericht.pdf</label></td>
          <td class="std">6. Nov 2026, 18:05</td>
          <td class="std"><a href="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=download&amp;ref_id=7&amp;ass_id=3&amp;delivered=2">Download</a></td>
        </tr></table>
        <input class="btn btn-default" type="submit" name="cmd[confirmDeleteDelivered]" value="Delete" />
        </form>"#;
    let page = parse_page(html, &base);
    let file = page.items().next().unwrap();
    assert_eq!(file.kind.code(), "file");
    assert_eq!(file.title, "Bericht.pdf");
    assert_eq!(file.properties, ["6. Nov 2026, 18:05"]);
    assert!(
        file.url
            .ends_with("cmd=download&ref_id=7&ass_id=3&delivered=2")
    );

    let form = delete_form(html, &base, "2", "confirmDeleteDelivered").unwrap();
    assert!(form.as_str().ends_with("cmd=post&ref_id=7&ass_id=3"));
    assert!(delete_form(html, &base, "3", "confirmDeleteDelivered").is_none());
    assert!(delete_form(html, &base, "2", "deleteDelivered").is_none());
    assert!(lists_delivered(html, "2"));
    assert!(!lists_delivered(html, "3"));
}

#[test]
fn finds_the_upload_form_of_an_assignment() {
    let base = Url::parse("https://ilias.example/ilias/ilias.php?cmd=uploadForm").unwrap();
    let html = r##"<form class="c-form" action="ilias.php?cmd=addUpload&amp;ref_id=7&amp;ass_id=1" method="post">
          <template><input id="x" type="hidden" name="form/input_0/input_1[input_0][]" value="" /></template>
        </form>
        <script>il.UI.Input.File.init(
          'il_ui_fw_1',
          'ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1',
          'ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=remove&ref_id=7&ass_id=1',
          'filename', 0, 20);</script>"##;
    let form = parse_upload_form(html, &base).unwrap();
    assert_eq!(
        form.upload.as_str(),
        "https://ilias.example/ilias/ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1"
    );
    assert!(
        form.finish
            .as_str()
            .ends_with("cmd=addUpload&ref_id=7&ass_id=1")
    );
    assert_eq!(form.field, "form/input_0/input_1[input_0][]");

    assert_eq!(parse_upload_form("<p>Time is up.</p>", &base), None);
}

#[test]
fn reads_an_assignment_with_its_files_and_submission() {
    let base = Url::parse("https://ilias.example/ilias/").unwrap();
    let html = r##"<div id="il_center_col">
          <h3>Work Instructions</h3><p>Solve the first sheet.</p>
          <h3>Files</h3>
          <div class="row"><div class="control-label col-sm-4"><p>Blatt 1.pdf</p></div>
            <div class="col-sm-8"><p><a href="ilias.php?cmd=downloadFile&amp;ref_id=7&amp;ass_id=1&amp;file=Blatt+1.pdf">Download</a></p></div></div>
          <h3>Submission</h3>
          <div class="row"><div class="control-label col-sm-4"><p>Last Submission</p></div>
            <div class="col-sm-8"><p>Today, 17:58</p></div></div>
          <div class="row"><div class="control-label col-sm-4"></div>
            <div class="col-sm-8"><button data-action="ilias.php?cmd=submissionScreen">Hand In</button></div></div>
        </div>"##;
    let page = parse_page(html, &base);
    let file = page.items().next().unwrap();
    assert_eq!(file.kind.code(), "file");
    assert_eq!(file.title, "Blatt 1.pdf");
    assert_eq!(file.extension().as_deref(), Some("pdf"));
    assert!(file.url.contains("cmd=downloadFile"));
    assert!(file.key() > 1 << 62, "no real reference id");

    let text: Vec<&str> = page
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text(text) | Block::Heading(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text,
        [
            "Work Instructions",
            "Solve the first sheet.",
            "Files",
            "Submission",
            "Last Submission: Today, 17:58"
        ]
    );
}

#[test]
fn an_assignment_lists_its_submission_below_the_section_of_that_name() {
    let base = Url::parse("https://ilias.example/ilias/").unwrap();
    let tabs = r#"<ul id="ilTab">
          <li id="tab_overview" class="active"><a href="ilias.php?cmdClass=ilAssignmentPresentationGUI&amp;ass_id=1">Overview</a></li>
          <li id="tab_submission"><a href="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=submissionScreen&amp;ass_id=1">Abgabe</a></li>
        </ul>"#;
    let html = format!(
        r#"{tabs}<div id="il_center_col"><h3>Files</h3><h3>Abgabe</h3><p>Nothing yet.</p></div>"#
    );
    let page = parse_page(&html, &base);
    let submission = page.items().next().unwrap();
    assert_eq!(submission.kind.code(), "sub");
    assert_eq!(submission.title, "Handed-in files");
    assert!(submission.url.ends_with("cmd=submissionScreen&ass_id=1"));
    assert!(submission.is_container());
    // It takes the place of the section's heading.
    assert!(matches!(&page.blocks[1], Block::Items { title, .. } if title == "Abgabe"));
    assert!(matches!(&page.blocks[2], Block::Text(t) if t == "Nothing yet."));

    // On the Submission tab itself, no such item.
    let active = tabs
        .replace(r#"tab_overview" class="active""#, r#"tab_overview""#)
        .replace(
            r#"id="tab_submission""#,
            r#"id="tab_submission" class="active""#,
        );
    let page = parse_page(&format!("{active}<p>Files</p>"), &base);
    assert_eq!(page.items().count(), 0);
}

/// Markup modelled on a real ILIAS 10 course: tabs, page-editor text, item groups,
/// and files whose links are `cmd=sendfile` GUI links.
const COURSE: &str = r##"
    <ul id="ilTab">
      <li><a href="ilias.php?baseClass=ildashboardgui&amp;cmd=show">Zu Favoriten</a></li>
      <li id="tab_view_content" class="active"><a href="ilias.php?cmd=view&amp;ref_id=7">Inhalt <span class="ilAccHidden">(Ausgewählte)</span></a></li>
      <li id="tab_info_short"><a href="ilias.php?cmd=showSummary&amp;ref_id=7">Info</a></li>
      <li id="ilLastTab"><a href="#">... </a></li>
    </ul>
    <div id="il_center_col">
      <a id="ilPageShowAdvContent" style="display:none" href="#">Vertiefungswissen anzeigen</a>
      <h1 class="il-page-content-header media-heading ilHeader">DEMO.101</h1>
      <h1 class="ilc_Paragraph ilc_heading1_Headline1">Willkommen</h1>
      <p class="ilc_Paragraph">Willkommen im Modul <strong>DEMO</strong></p>
      <p class="ilc_Paragraph">Ort: Raum B-042</p>
      <div class="ilContainerBlock">
        <div class="ilContainerBlockHeader"><h2 class="ilHeader">Unterlagen</h2></div>
        <ul class="ilContainerItemsContainer"><li class="ilCLI">
          <div class="ilContainerListItemOuter">
            <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle"
              href="https://elearning.hslu.ch/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Lecture_01</a></h3>
            <div class="dropdown"><button>Aktionen</button><ul><li>Download</li></ul></div>
            <div class="il_Description">Folien Woche 1</div>
            <div class="il_ItemProperties"><span class="il_ItemProperty">pdf&nbsp;&nbsp;</span>
              <span class="il_ItemProperty">1.2 MB&nbsp;&nbsp;</span></div>
          </div>
        </li><li class="ilCLI">
          <div class="ilContainerListItemOuter">
            <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle"
              href="https://elearning.hslu.ch/ilias/goto.php/fold/8">Übungen</a></h3>
          </div>
        </li><li class="ilCLI">
          <div class="ilContainerListItemOuter">
            <img src="./Customizing/skin/hslu/hslu/images/standard/icon_webr.svg" class="ilListItemIcon"/>
            <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle" target="_blank"
              href="ilias.php?baseClass=ilLinkResourceHandlerGUI&amp;ref_id=10&amp;cmd=calldirectlink">Zoom-Link</a></h3>
            <div class="il_ItemProperties"><span class="il_ItemProperty">Lernfortschritt:</span></div>
          </div>
        </li></ul>
      </div>
    </div>"##;

#[test]
fn parses_course_layout() {
    let page = parse_page(
        COURSE,
        &url("https://elearning.hslu.ch/ilias/goto.php/crs/7"),
    );
    assert_eq!(
        page.tabs,
        vec![
            Tab {
                title: "Inhalt".into(),
                url: "https://elearning.hslu.ch/ilias/ilias.php?cmd=view&ref_id=7".into(),
                active: true,
            },
            Tab {
                title: "Info".into(),
                url: "https://elearning.hslu.ch/ilias/ilias.php?cmd=showSummary&ref_id=7".into(),
                active: false,
            },
        ]
    );
    let [
        Block::Heading(heading),
        Block::Text(welcome),
        Block::Text(place),
        Block::Items { title, items },
    ] = page.blocks.as_slice()
    else {
        panic!("unexpected blocks: {:#?}", page.blocks);
    };
    assert_eq!(heading, "Willkommen");
    assert_eq!(welcome, "Willkommen im Modul DEMO");
    assert_eq!(place, "Ort: Raum B-042");
    assert_eq!(title, "Unterlagen");
    assert_eq!(items.len(), 3);
    let file = &items[0];
    assert_eq!((file.kind.code(), file.key()), ("file", 9));
    assert_eq!(file.description, "Folien Woche 1");
    assert_eq!(file.properties, vec!["pdf", "1.2 MB"]);
    assert_eq!(file.extension().as_deref(), Some("pdf"));
    assert_eq!(items[1].kind.code(), "fold");
    let link = &items[2];
    assert_eq!((link.kind.code(), link.key()), ("webr", 10));
    assert!(link.properties.is_empty());
}

#[test]
fn links_in_page_text_follow_their_paragraph() {
    // As the local ILIAS 10 renders a page-editor paragraph with links.
    let html = r##"<ul id="ilTab"><li id="tab_view_content"><a href="ilias.php?cmd=view">Content</a></li></ul>
        <div id="il_center_col">
          <div class="ilc_Paragraph ilc_text_block_Standard">Lecture on Mondays. Online:
            <a class="ilc_link_ExtLink" href="https://zoom.us/j/123456789" target="_blank">Zoom-Meeting</a>
            and the <a class="ilc_link_ExtLink" href="goto.php/fold/89">Unterlagen</a>
            (<a href="#top">top</a>, <a href="mailto:a@b.ch">mail</a>,
            <a href="ilias.php?cmd=markPostRead&amp;pos_pk=1">read</a>,
            <a href="https://zoom.us/j/123456789">again</a>).</div>
          <div class="ilContainerBlock"><h2>Inhalt</h2>
            <div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/file/7">Skript</a></div></div>
        </div>"##;
    let page = parse_page(html, &url("https://ilias.example/ilias/goto.php/crs/1"));
    assert!(
        matches!(&page.blocks[0], Block::Text(t) if t.starts_with("Lecture on Mondays. Online: Zoom-Meeting and the Unterlagen"))
    );
    let Block::Items { title, items } = &page.blocks[1] else {
        panic!("links after the paragraph: {:?}", page.blocks);
    };
    assert!(title.is_empty());
    let links: Vec<(&str, &str, &str)> = items
        .iter()
        .map(|i| (i.kind.code(), i.title.as_str(), i.url.as_str()))
        .collect();
    // Only web links, each once, and none that would change something on ILIAS.
    assert_eq!(
        links,
        [
            ("link", "Zoom-Meeting", "https://zoom.us/j/123456789"),
            (
                "link",
                "Unterlagen",
                "https://ilias.example/ilias/goto.php/fold/89"
            ),
        ]
    );
    assert_eq!(items[0].description, "https://zoom.us/j/123456789");
    assert!(matches!(&page.blocks[2], Block::Items { title, .. } if title == "Inhalt"));
}

#[test]
fn joins_info_screen_label_and_value() {
    let html = r#"<div id="il_center_col"><div class="ilInfoScreenSec">
        <div class="ilFormHeader"><h2 class="ilHeader">Allgemein</h2></div>
        <div class="form-group row"><div class="il_InfoScreenProperty control-label">Sprache</div>
          <div class="il_InfoScreenPropertyValue"> Deutsch </div></div></div></div>"#;
    let page = parse_page(html, &url("https://elearning.hslu.ch/ilias/ilias.php"));
    assert_eq!(
        page.blocks,
        vec![
            Block::Heading("Allgemein".into()),
            Block::Text("Sprache: Deutsch".into())
        ]
    );
}

#[test]
fn parses_ui_framework_item_groups() {
    let html = r#"<main><div class="il-item-group"><h3>2026</h3><div class="il-item-group-items"><ul>
        <li><div class="il-item il-std-item"><h4 class="il-item-title">
          <a href="https://elearning.hslu.ch/ilias/goto.php/crs/654321">DEMO.102</a></h4>
          <div class="il-item-description">Examples &amp; Exercises</div>
          <span class="il-item-property-name">Anmeldung</span><span class="il-item-property-value">Keine</span>
          <span class="il-item-property-name"></span><span class="il-item-property-value"></span>
        </div></li></ul></div></div></main>"#;
    let page = parse_page(html, &url("https://elearning.hslu.ch/ilias/ilias.php"));
    let [Block::Items { title, items }] = page.blocks.as_slice() else {
        panic!("unexpected blocks: {:#?}", page.blocks);
    };
    assert_eq!(title, "2026");
    assert_eq!(items[0].description, "Examples & Exercises");
    assert_eq!(items[0].properties, vec!["Anmeldung: Keine"]);
}

#[test]
fn parses_forum_threads_without_rating_buttons() {
    let html = r#"<div id="il_center_col"><div class="il-item-group"><div class="il-item-group-items"><ul>
        <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=123456&amp;thr_pk=1&amp;page=0">Frage zu Blatt 1</a></h4>
          <span class="il-item-property-name">Beiträge</span><span class="il-item-property-value">2 (1 Ungelesen)</span>
          <span class="il-item-property-name">Bewertung</span><span class="il-item-property-value"><div class="il-standard-popover-content" style="display:none;"><div class="ilRatingOverlay"><p>Ihre Bewertung</p></div></div><button>Bewertungsskala aufklappen</button></span>
        </div></li>
        <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=123456&amp;thr_pk=2&amp;page=0">Termine</a></h4>
        </div></li></ul></div></div></div>"#;
    let page = parse_page(
        html,
        &url("https://ilias.example/ilias/goto.php/frm/123456"),
    );
    let threads: Vec<_> = page.items().collect();
    assert_eq!(
        threads.len(),
        2,
        "both threads, although they share the forum's ref_id"
    );
    assert_eq!((threads[0].kind.code(), threads[0].key()), ("thr", 1));
    assert!(threads[0].is_container());
    assert_eq!(threads[0].properties, vec!["Beiträge: 2 (1 Ungelesen)"]);
}

#[test]
fn parses_forum_posts() {
    let html = r##"<div id="il_center_col"><ul>
        <li class="ilFrmPostRow ilModeratorPosting ilFrmPost-level-1"><a id="2"></a><div class="ilFrmPostContentContainer">
          <div class="ilFrmPostHeader"><span class="small">Ada Lovelace | <a href="#">alovelace</a> (Moderator) | 19. Feb 2026, 07:12</span>
            <div class="ilFrmPostTitle">Willkommen</div></div>
          <div class="ilFrmPostContent"><p>Hallo zusammen!</p><p>Fragen bitte hier.</p></div></div>
          <div class="il-item-actions"><button class="btn btn-default" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=101&amp;action=showreply&amp;pos_pk=2&amp;page=0&amp;thr_pk=3#reply_2">Antworten</button></div></li>
        <li class="ilFrmPostRow tblrow2 ilFrmPost-level-2"><div class="ilFrmPostContentContainer">
          <div class="ilFrmPostHeader"><span class="small">Alan Turing | <a href="#">aturing</a> | 20. Feb 2026, 10:00</span>
            <div class="ilFrmPostTitle"><a href="ilias.php?cmd=markPostRead&amp;pos_pk=2"><b>Re: Willkommen</b></a></div></div>
          <div class="ilFrmPostContent"><span>Danke!<br />Bis bald</span></div></div></li>
        </ul></div>"##;
    let page = parse_page(
        html,
        &url(
            "https://ilias.example/ilias/ilias.php?cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=101&thr_pk=3",
        ),
    );
    // Only Ada's post offers Reply here.
    assert_eq!(page.rights.reply, [2]);
    assert_eq!(page.rights.to_string(), "reply: 1 posts");
    let posts: Vec<&Post> = page
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Post(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(page.blocks.len(), 2, "only the posts: {:#?}", page.blocks);
    assert_eq!(
        *posts[0],
        Post {
            id: 2,
            author: "Ada Lovelace · alovelace (Moderator)".into(),
            date: "19. Feb 2026, 07:12".into(),
            title: "Willkommen".into(),
            body: vec!["Hallo zusammen!".into(), "Fragen bitte hier.".into()],
            depth: 0,
            unread: false,
        }
    );
    assert_eq!(posts[1].title, "Re: Willkommen");
    assert_eq!(posts[1].body, vec!["Danke!", "Bis bald"]);
    assert_eq!(posts[1].depth, 1);
    assert!(posts[1].unread);
}

#[test]
fn parses_the_todo_block() {
    let html = r#"<div id="il_center_col"></div><aside id="il_right_col">
        <div id="block_pdtasks_0"><div class="il-item-group"><ul>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/123456/1">Abgabe zur Übungseinheit "Blatt 1"</a></h4>
            <div class="il-item-description">17. Jan 2027, 23:55</div>
            <span class="il-item-property-name">Übung</span><span class="il-item-property-value">Abgabeordner</span></div></li>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/123456/2">Abgabe zur Übungseinheit "Blatt 2"</a></h4>
            <div class="il-item-description">5. Nov 2026, 00:00</div></div></li>
        </ul></div></div>
        <div id="block_pdnews_0"><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/file/9">Neue Datei</a></h4></div></div>
        </aside>"#;
    let todo = parse_dashboard(html, &url("https://ilias.example/ilias/ilias.php")).todo;
    let found: Vec<_> = todo
        .iter()
        .map(|i| {
            (
                i.kind.code(),
                i.key(),
                i.title.as_str(),
                i.description.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![
            ("exc", 123456, "Blatt 1", "17. Jan 2027, 23:55"),
            ("exc", 123456, "Blatt 2", "5. Nov 2026, 00:00"),
        ]
    );
    assert_eq!(todo[0].properties, vec!["Übung: Abgabeordner"]);
}

#[test]
fn finds_the_course_in_the_breadcrumb() {
    let html = r#"<nav class="breadcrumb"><span class="breadcrumb-crumb"><a href="ilias.php?ref_id=3">Abgabeordner</a></span>
        <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/crs/2">DEMO.101</a></span>
        <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/cat/1">2026</a></span></nav>"#;
    assert_eq!(course_title(html).as_deref(), Some("DEMO.101"));
    assert_eq!(course_title("<p>nothing</p>"), None);
}

#[test]
fn parses_ilias_dates() {
    use jiff::civil::{date, datetime};
    let today = date(2026, 11, 1);
    assert_eq!(
        parse_date("5. Nov 2026, 00:00", today),
        Some(datetime(2026, 11, 5, 0, 0, 0, 0))
    );
    assert_eq!(
        parse_date("3. Mär 2027, 23:55", today),
        Some(datetime(2027, 3, 3, 23, 55, 0, 0))
    );
    assert_eq!(
        parse_date("1. Oct 2026, 8:05", today),
        Some(datetime(2026, 10, 1, 8, 5, 0, 0))
    );
    assert_eq!(
        parse_date("Morgen, 12:00", today),
        Some(datetime(2026, 11, 2, 12, 0, 0, 0))
    );
    assert_eq!(parse_date("bald", today), None);
}

#[test]
fn reads_favourites_and_their_menu_links() {
    let html = r#"<div id="il_center_col">
        <div id="block_dash_fav_0"><h2>Favoriten</h2><div class="il-deck">
          <div class="il-card thumbnail">
            <ul class="dropdown-menu"><li><button data-action="ilias.php?baseClass=ildashboardgui&amp;cmd=removeFromDesk&amp;type=crs&amp;item_ref_id=101">Von Favoriten entfernen</button></li></ul>
            <div class="caption card-title"><a href="https://ilias.example/ilias/goto.php/crs/101">DEMO.101</a></div>
          </div></div></div>
        <div class="il-item-group"><h3>2026</h3><div class="il-item-group-items"><ul>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/102">DEMO.102</a></h4>
            <ul class="dropdown-menu"><li><button data-action="ilias.php?cmd=addToDesk&amp;type=crs&amp;item_ref_id=102">Zu Favoriten hinzufügen</button></li></ul></div></li>
          <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/103">DEMO.103</a></h4>
            <ul class="dropdown-menu"><li><button data-action="ilias.php?cmd=addToDesk&amp;type=crs&amp;item_ref_id=999">Zu Favoriten hinzufügen</button></li></ul></div></li>
        </ul></div></div></div>"#;
    let base = url("https://ilias.example/ilias/ilias.php");
    let Some(Block::Items { title, items }) = parse_dashboard(html, &base).favorites else {
        panic!("no favourites");
    };
    assert_eq!(title, "Favoriten");
    assert_eq!((items[0].key(), items[0].favorite), (101, true));
    assert!(
        items[0]
            .favorite_toggle
            .as_deref()
            .unwrap()
            .contains("cmd=removeFromDesk")
    );

    let page = parse_page(html, &base);
    let course = |id| page.items().find(|i| i.key() == id).unwrap();
    assert!(!course(102).favorite);
    assert_eq!(
        course(102).favorite_toggle.as_deref(),
        Some("https://ilias.example/ilias/ilias.php?cmd=addToDesk&type=crs&item_ref_id=102")
    );
    // A menu link for another object is not this item's.
    assert_eq!(course(103).favorite_toggle, None);
}

#[test]
fn files_know_their_folders() {
    let html = r#"<nav><div class="breadcrumb" dir="rtl">
          <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/fold/3">Woche 1</a></span>
          <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/fold/2">Unterlagen</a></span>
          <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/crs/1">DEMO.101</a></span>
          <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/cat/9">2026</a></span>
          <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/root/1">Magazin</a></span>
        </div></nav>
        <div id="il_center_col"><div class="ilContainerListItemOuter">
          <a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=4">Lecture_01</a></div>
          <div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/fold/5">Extras</a></div></div>"#;
    let page = parse_page(html, &url("https://ilias.example/ilias/goto.php/fold/3"));
    let items: Vec<_> = page.items().collect();
    assert_eq!(items[0].path, vec!["DEMO.101", "Unterlagen", "Woche 1"]);
    assert!(items[1].path.is_empty(), "only files need it");
}

#[test]
fn cached_pages_from_before_typed_ids_still_load() {
    // As older versions wrote them: kinds as ILIAS' codes, every id a plain `ref_id`.
    let json = r#"{"tabs":[],"blocks":[{"Items":{"title":"","items":[
        {"kind":"crs","ref_id":5,"title":"DEMO","url":"u","description":"","properties":[]},
        {"kind":"ass","ref_id":3,"title":"Blatt 1","url":"u","description":"","properties":[]},
        {"kind":"thr","ref_id":42,"title":"Frage","url":"u","description":"","properties":[]},
        {"kind":"file","ref_id":9223372036854775809,"title":"a.pdf","url":"u","description":"","properties":[]},
        {"kind":"lm","ref_id":8,"title":"Modul","url":"u","description":"","properties":[]}]}}],
        "todo":[{"kind":"exc","ref_id":7,"title":"Blatt 1","url":"u","description":"due in 3 days","properties":["DEMO"]}]}"#;
    let page: Page = serde_json::from_str(json).unwrap();
    let ids: Vec<(Kind, Id)> = page.items().map(|i| (i.kind.clone(), i.id)).collect();
    assert_eq!(
        ids,
        [
            (Kind::Course, Id::Ref(5)),
            (Kind::Assignment, Id::Assignment(3)),
            (Kind::Thread, Id::Thread(42)),
            (Kind::File, Id::Local(1 << 63 | 1)),
            (Kind::Other("lm".into()), Id::Ref(8)),
        ]
    );
    // The old To-Do shape is dropped (the refresh brings it back), not the whole page.
    assert!(page.todo.is_empty());
    // Written again the same way, so indexes keyed by these numbers stay valid.
    let again = serde_json::to_value(&page.blocks).unwrap();
    assert_eq!(again[0]["Items"]["items"][1]["kind"], "ass");
    assert_eq!(again[0]["Items"]["items"][1]["ref_id"], 3);
}

/// What root sees on a folder of the local ILIAS: "Add New Item", the upload form and the
/// items' action menus (anonymised, shortened).
const EDITABLE: &str = r#"<div id="il_center_col">
    <ul class="c-drilldown__menu">
      <li><a class="il-link link-bulky" href="http://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&amp;cmd=create&amp;ref_id=89&amp;new_type=fold">Folder</a></li>
      <li><a class="il-link link-bulky" href="http://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&amp;cmd=create&amp;ref_id=89&amp;new_type=file">File</a></li>
      <li><a href="http://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&amp;cmd=create&amp;ref_id=12&amp;new_type=wiki">Wiki somewhere else</a></li>
    </ul>
    <form enctype="multipart/form-data" action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mk&amp;cmdClass=ilObjFileGUI&amp;cmd=post&amp;fallbackCmd=uploadFiles&amp;ref_id=89&amp;new_type=file&amp;origin=dropzone&amp;rtoken=t" method="post"></form>
    <form enctype="multipart/form-data" action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mk&amp;cmdClass=ilObjFileGUI&amp;cmd=post&amp;fallbackCmd=uploadFiles&amp;ref_id=90&amp;new_type=file&amp;origin=dropzone&amp;rtoken=t" method="post"></form>
    <ul>
      <li><button class="btn btn-link" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mp&amp;cmdClass=ilObjFolderGUI&amp;cmd=delete&amp;ref_id=89&amp;item_ref_id=90">Delete</button></li>
      <li><button class="btn btn-link" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mp&amp;cmdClass=ilObjFolderGUI&amp;cmd=cut&amp;ref_id=89&amp;item_ref_id=90">Cut</button></li>
      <li><button class="btn btn-link" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mp&amp;cmdClass=ilObjFolderGUI&amp;cmd=delete&amp;ref_id=89&amp;item_ref_id=91">Delete</button></li>
      <li><button class="btn btn-link" data-action="https://evil.example/ilias/ilias.php?cmd=delete&amp;ref_id=89&amp;item_ref_id=92">Delete</button></li>
    </ul></div>"#;

#[test]
fn reads_the_rights_ilias_offers_on_a_page() {
    let page = parse_page(
        EDITABLE,
        &url("http://ilias.example/ilias/goto.php/fold/89"),
    );
    assert_eq!(
        page.rights,
        Rights {
            upload: true,
            create: vec![Kind::Folder, Kind::File],
            delete: vec![90, 91],
            ..Rights::default()
        }
    );
    assert_eq!(
        page.rights.to_string(),
        "upload · new: fold, file · delete: 2 items"
    );

    // After ILIAS' redirect the page is known by its `ref_id`.
    let redirected = url(
        "http://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&cmdClass=ilobjfoldergui&ref_id=89",
    );
    assert_eq!(parse_page(EDITABLE, &redirected).rights, page.rights);

    // On another page none of it is for that page.
    let other = parse_page(EDITABLE, &url("http://ilias.example/ilias/goto.php/fold/7"));
    assert!(other.rights.is_empty());
}

#[test]
fn a_student_has_no_rights_and_old_cached_pages_have_none_either() {
    let page = parse_page(
        r#"<div id="il_center_col"><a class="il_ContainerItemTitle" href="goto.php/file/90">a.pdf</a></div>"#,
        &url("http://ilias.example/ilias/goto.php/fold/89"),
    );
    assert!(page.rights.is_empty());
    let cached: Page = serde_json::from_str(r#"{"tabs":[],"blocks":[]}"#).unwrap();
    assert!(cached.rights.is_empty());
}

#[test]
fn a_forum_offers_new_threads_and_its_form_is_read() {
    let forum = url("https://ilias.example/ilias/goto.php/frm/101");
    let html = r#"<button class="btn btn-default" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=createThread&amp;ref_id=101">New Thread</button>"#;
    assert!(parse_page(html, &forum).rights.new_thread);
    let other = url("https://ilias.example/ilias/goto.php/frm/102");
    assert!(!parse_page(html, &other).rights.new_thread);

    // The form with text has a file field and its own button; the empty-thread one doesn't.
    let html = r#"<form action="ilias.php?cmdClass=ilObjForumGUI&amp;cmd=post&amp;fallbackCmd=addThread&amp;ref_id=101">
          <input type="hidden" name="alias" value="ada" /><input type="text" name="subject" />
          <textarea class="form-control RTEditor" name="message"></textarea>
          <input type="file" name="userfile[0]" /><input type="SUBMIT" name="cmd[addThread]" value="Create" /></form>
        <form action="ilias.php?cmdClass=ilObjForumGUI&amp;cmd=post&amp;fallbackCmd=addThread&amp;ref_id=101">
          <input type="text" name="subject" /><input type="submit" name="cmd[addEmptyThread]" value="Create" /></form>"#;
    let form = post_form(html, &forum, "addThread", |_| true).unwrap();
    assert_eq!(form.hidden, [("alias".to_string(), "ada".to_string())]);
    assert_eq!(form.files, ["userfile[0]"]);
    assert!(form.rich);
    assert_eq!(form.button, ("cmd[addThread]".into(), "Create".into()));
    assert!(post_form(html, &forum, "addThread", |_| false).is_none());
    assert_eq!(
        message_html("a\nb & <c>\n\n\nd"),
        "<p>a<br />b &amp; &lt;c&gt;</p>\n<p>d</p>"
    );
}

/// An assignment's "Submissions and Grades" page as the local ILIAS 10 renders it for root
/// (anonymised, shortened): two assignments, one participant who handed in, one who didn't.
const SUBMISSIONS: &str = r#"<ol class="breadcrumb">
      <li><a href="goto.php/exc/103">Blatt 0</a></li><li><a href="goto.php/crs/86">DEMO.101</a></li></ol>
    <ul id="ilTab"><li id="tab_content"><a href="ilias.php?cmdClass=ilobjexercisegui&amp;cmd=showOverview&amp;ref_id=103">Assignments</a></li>
      <li id="tab_grades" class="active"><a href="ilias.php?cmdClass=ilexercisemanagementgui&amp;cmd=members&amp;ref_id=103">Submissions and Grades</a></li></ul>
    <div id="il_center_col"><form id="ilToolbar"><select aria-label="Assignment" id="ass_id" name="ass_id">
      <option value="2" selected="selected">Blatt 0</option><option value="4">Blatt 0b</option></select></form>
    <table><tr class="tblrow1">
      <td class="std"><input type="checkbox" name="sel_part_ids[]" value="316" /><input type="hidden" name="listed_part_ids[]" value="316" /></td>
      <td class="std"> Muster, Anna </td><td class="std"> amuster </td><td class="std small"> 6. Nov 2026, 18:02:15 </td><td>&nbsp;</td>
      <td class="std small"><select name="status[316]"><option value="notgraded" selected="selected">Not Graded</option><option value="passed">Passed</option></select></td>
      <td><ul class="dropdown-menu"><li><button class="btn btn-link" data-action="ilias.php?baseClass=ilexercisehandlergui&amp;cmdClass=ilexsubmissionfilegui&amp;cmd=downloadReturned&amp;ref_id=103&amp;member_id=316&amp;ass_id=2">Download Submissions (2)</button></li>
        <li><button class="btn btn-link" data-action="ilias.php?cmdClass=ilExerciseManagementGUI&amp;cmd=redirectFeedbackMail&amp;ref_id=103">Send Evaluation Mail</button></li></ul></td></tr>
    <tr class="tblrow2">
      <td class="std"><input type="hidden" name="listed_part_ids[]" value="317" /></td>
      <td class="std"> Beispiel, Ben </td><td class="std"> bbeispiel </td><td class="std small"> &nbsp; </td><td>&nbsp;</td>
      <td></td><td></td></tr></table></div>"#;

#[test]
fn lists_every_participants_submission_with_assignments_as_tabs() {
    let page_url = url(
        "https://ilias.example/ilias/ilias.php?baseClass=ilexercisehandlergui&cmdClass=ilexercisemanagementgui&cmd=members&ref_id=103",
    );
    let page = parse_page(SUBMISSIONS, &page_url);
    let tabs: Vec<(&str, bool)> = page
        .tabs
        .iter()
        .map(|t| (t.title.as_str(), t.active))
        .collect();
    assert_eq!(
        tabs,
        [
            ("Blatt 0", true),
            ("Blatt 0b", false),
            ("Assignments", false)
        ]
    );
    assert!(
        page.tabs[1]
            .url
            .ends_with("cmd=members&ref_id=103&ass_id=4"),
        "{}",
        page.tabs[1].url
    );
    let items: Vec<&Item> = page.items().collect();
    assert_eq!(items.len(), 2);
    let anna = items[0];
    assert_eq!(anna.kind, Kind::Participant);
    assert_eq!(
        (anna.title.as_str(), anna.description.as_str()),
        ("Muster, Anna", "amuster")
    );
    assert_eq!(
        anna.properties,
        ["2 files", "handed in 6. Nov 2026, 18:02:15", "Not Graded"]
    );
    assert!(anna.url.contains("cmd=downloadReturned") && anna.url.contains("member_id=316"));
    assert_eq!(
        anna.path,
        ["DEMO.101", "Blatt 0", "Blatt 0", "Muster, Anna (amuster)"]
    );
    let ben = items[1];
    assert!(ben.url.is_empty());
    assert_eq!(ben.properties, ["nothing handed in"]);
    assert_ne!(anna.id, ben.id);
    // It only reads: no download goes past the write guard as a write.
    assert!(!crate::ilias::write::is_write(
        &Url::parse(&anna.url).unwrap()
    ));
}
