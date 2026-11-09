<?php
// Fills the local test ILIAS with courses, folders, files, a forum and exercises, through its
// SOAP interface. Run by seed.sh, see README.md.

chdir('/var/www/html');
require_once './vendor/composer/vendor/autoload.php';

const ILIAS_HTTP_PATH = 'http://localhost:8080';
const ROOT = 1;
const USER_ROLE = 4;

$soap = new SoapClient('http://localhost/soap/server.php?wsdl', [
    'location' => 'http://localhost/soap/server.php',
]);
$sid = $soap->login('default', 'root', 'ilias-dev');
(new ilCronStartUp('default', 'root'))->authenticate();

if (str_contains($soap->getTreeChilds($sid, ROOT, ['crs'], 0), 'DEMO.101')) {
    exit("Already seeded.\n");
}

function create($parent, $type, $title, $description = '')
{
    global $soap, $sid;
    return (int) $soap->addObject($sid, $parent, sprintf(
        '<Objects><Object type="%s" offline="0"><Title>%s</Title><Description>%s</Description><ImportId/></Object></Objects>',
        $type,
        htmlspecialchars($title),
        htmlspecialchars($description)
    ));
}

// SOAP refuses file contents in this version, so files go through ILIAS' own classes.
function file_in($parent, $name, $content)
{
    $file = new ilObjFile();
    $file->setTitle(pathinfo($name, PATHINFO_FILENAME));
    $file->create();
    $file->createReference();
    $file->putInTree($parent);
    $file->setPermissions($parent);
    $file->appendStream(ILIAS\Filesystem\Stream\Streams::ofString($content), $name);
}

const PDF = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n"
    . "2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n"
    . "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\n"
    . "trailer<</Root 1 0 R>>\n%%EOF\n";

function exercise($parent, $title, $instruction, $days, $student)
{
    $exercise = new ilObjExercise();
    $exercise->setTitle($title);
    $exercise->create();
    $exercise->createReference();
    $exercise->putInTree($parent);
    $exercise->setPermissions($parent);
    $exercise->setOfflineStatus(false);
    $exercise->update();

    $assignment = new ilExAssignment();
    $assignment->setExerciseId($exercise->getId());
    $assignment->setTitle($title);
    $assignment->setInstruction($instruction);
    $assignment->setType(ilExAssignment::TYPE_UPLOAD);
    $assignment->setDeadlineMode(ilExAssignment::DEADLINE_ABSOLUTE);
    $assignment->setDeadline(time() + $days * 86400);
    $assignment->setPeerReviewValid(ilExAssignment::PEER_REVIEW_VALID_NONE);
    $assignment->save();
    (new ilExerciseMembers($exercise))->assignMember($student);
    return $assignment->getId();
}

function temp_file($name, $content)
{
    // ILIAS only imports from its own storage folders.
    $dir = ilFileUtils::ilTempnam();
    mkdir($dir);
    file_put_contents("$dir/$name", $content);
    return $dir;
}

function attach($assignment, $name, $content)
{
    global $DIC;
    $files = $DIC->exercise()->internal()->domain()->assignment()->instructionFiles($assignment);
    $files->createCollection();
    $files->importFromDirectory(temp_file($name, $content));
}

function hand_in($assignment, $student, $name, $content)
{
    global $DIC;
    // Only the student themselves may hand in.
    $root = $DIC->user();
    unset($DIC['ilUser']);
    $DIC['ilUser'] = new ilObjUser($student);
    $path = temp_file($name, $content) . "/$name";
    if (!$DIC->exercise()->internal()->domain()->submission($assignment)->addLocalFile($student, $path, $name)) {
        echo "Could not hand in $name\n";
    }
    unset($DIC['ilUser']);
    $DIC['ilUser'] = $root;
}

$users = '<Users><User Action="Insert"><Login>student</Login><Role Id="' . USER_ROLE . '" Type="Global">User</Role>'
    . '<Password Type="PLAIN">ilias-dev</Password><Firstname>Test</Firstname><Lastname>Student</Lastname>'
    . '<Email>student@ilias.test</Email><TimeLimitUnlimited>1</TimeLimitUnlimited>'
    . '<Active>true</Active></User></Users>';
$soap->importUsers($sid, -1, $users, 1, 0);
$student = (int) $soap->lookupUser($sid, 'student');


// Two semesters, so the list shows its newest-first order and the year headings.
$algebra = create(ROOT, 'crs', 'DEMO.101 Algebra HS26', 'Linear algebra, autumn 2026');
$biology = create(ROOT, 'crs', 'DEMO.102 Biologie FS26', 'Spring 2026');
$old = create(ROOT, 'crs', 'DEMO.100 Grundlagen HS24', 'An old course');
foreach ([$algebra, $biology, $old] as $course) {
    $soap->assignCourseMember($sid, $course, $student, 'Member');
}

$documents = create($algebra, 'fold', 'Unterlagen', 'Slides and scripts');
$week1 = create($documents, 'fold', 'Woche 1');
$week2 = create($documents, 'fold', 'Woche 2');
$deep = create($week2, 'fold', 'Zusatzmaterial');
file_in($algebra, 'Modulbeschrieb.pdf', PDF);
file_in($week1, 'Folien Woche 1.pdf', PDF);
file_in($week1, 'Übung 1.docx', 'A Word document, in spirit.');
file_in($week2, 'Folien Woche 2.pdf', PDF);
file_in($week2, 'Daten.xlsx', 'A spreadsheet, in spirit.');
file_in($deep, 'Formelsammlung.pdf', PDF);
file_in($deep, 'Beispiele.zip', 'A zip file, in spirit.');
file_in($deep, 'notizen.md', "# Notes\n");
create($algebra, 'frm', 'Fragen und Antworten', 'Course forum');
$sheet = exercise($algebra, 'Blatt 1', 'Solve the first sheet.', 3, $student);
attach($sheet, 'Blatt 1.pdf', PDF);
$past = exercise($algebra, 'Blatt 0', 'The warm-up, already over.', 1, $student);
attach($past, 'Blatt 0.pdf', PDF);
hand_in($past, $student, 'Loesung Blatt 0.pdf', PDF);
// Only now: nothing can be handed in after the deadline.
$over = new ilExAssignment($past);
$over->setDeadline(time() - 5 * 86400);
$over->update();
exercise($biology, 'Laborbericht', 'Hand in the lab report.', 10, $student);

file_in($biology, 'Skript Zellbiologie.pdf', PDF);
create($biology, 'fold', 'Praktikum');
// More files than the folder shows at once: ILIAS lists 5, then a "Show more" button.
$exams = create($biology, 'fold', 'Alte Prüfungen', 'More files than ILIAS shows at once');
for ($i = 1; $i <= 14; $i++) {
    file_in($exams, sprintf('Pruefung %02d.pdf', $i), PDF);
}
ilContainer::_writeContainerSetting(ilObject::_lookupObjId($exams), 'block_limit', '5');
file_in($old, 'Archiv.pdf', PDF);

echo "Seeded.\n";
