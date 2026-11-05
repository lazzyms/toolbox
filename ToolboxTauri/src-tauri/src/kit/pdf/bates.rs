//! Bates numbering: one identifier per page, continuing across an ordered set of PDFs.
//!
//! Ranges are set aside by file order from each file's page count before any file is stamped, so files can run in
//! parallel and a file that fails leaves its range as a gap instead of shifting the numbers of the files after it.
use std::{collections::{HashMap, HashSet}, io::Write, path::{Path, PathBuf}};
use lopdf::Document;
use serde::Deserialize;
use crate::kit::{common::{JobOutcome, OutputLocation, OutputNaming}, contracts::{OutcomeState, ToolError}};
use super::scene::{self, Kind, Rect, SceneObject, TextAlign};

pub const SUFFIX: &str = "-bates";
const MARGIN: f32 = 24.;
const MAX_DIGITS: u32 = 12;
const MAX_AFFIX_CHARACTERS: usize = 40;
const MAX_START: u64 = 999_999_999_999;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BatesPosition { BottomRight, BottomCenter, BottomLeft, TopRight }

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatesFormat {
    pub prefix: String,
    pub suffix: String,
    pub start: u64,
    pub digits: u32,
    pub position: BatesPosition,
    #[serde(default)] pub font_family: Option<String>,
    pub font_size: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatesRequest {
    /// The whole set, in the order the numbers run.
    pub paths: Vec<PathBuf>,
    /// The files to stamp this time, when only some of the set runs again (a retry). They keep their set-aside numbers.
    #[serde(default)] pub run: Option<Vec<PathBuf>>,
    #[serde(flatten)] pub format: BatesFormat,
    pub output_location: OutputLocation,
}

impl BatesFormat {
    pub fn identifier(&self, number: u64) -> String {
        format!("{}{:0width$}{}", self.prefix, number, self.suffix, width = self.digits as usize)
    }

    pub fn range(&self, range: &BatesRange) -> String {
        if range.pages == 1 { return self.identifier(range.first); }
        format!("{} to {}", self.identifier(range.first), self.identifier(range.last()))
    }

    fn stamp(&self, number: u64, width: f32, height: f32) -> SceneObject {
        let line = self.font_size * 1.2;
        let (top, align) = match self.position {
            BatesPosition::BottomRight => (false, TextAlign::Right),
            BatesPosition::BottomCenter => (false, TextAlign::Center),
            BatesPosition::BottomLeft => (false, TextAlign::Left),
            BatesPosition::TopRight => (true, TextAlign::Right),
        };
        let margin = MARGIN.min(width / 4.).min(height / 4.);
        let rect = Rect { x: margin, y: if top { margin } else { height - margin - line }, width: (width - 2. * margin).max(1.), height: line };
        SceneObject {
            kind: Kind::Text, rect, text: self.identifier(number), font_size: self.font_size, color: "#000000".into(), opacity: 1.,
            strokes: Vec::new(), shape: None, highlight_mode: None, signature_mode: None, signature_path: None,
            font_family: self.font_family.clone(), watermark_pattern: None, bold: false, italic: false, align, page_range: None,
        }
    }

    fn validate(&self) -> Result<(), ToolError> {
        let affix = |value: &str, name: &str| {
            if value.chars().count() > MAX_AFFIX_CHARACTERS { return Err(ToolError::invalid_input(format!("The {name} can be at most {MAX_AFFIX_CHARACTERS} characters."))); }
            if value.chars().any(char::is_control) { return Err(ToolError::invalid_input(format!("The {name} can't contain line breaks or tabs."))); }
            Ok(())
        };
        affix(&self.prefix, "prefix")?;
        affix(&self.suffix, "suffix")?;
        if !(1..=MAX_DIGITS).contains(&self.digits) { return Err(ToolError::invalid_input(format!("Digits must be from 1 to {MAX_DIGITS}."))); }
        if self.start > MAX_START { return Err(ToolError::invalid_input(format!("Start at must be {MAX_START} or less."))); }
        if !self.font_size.is_finite() || !(6. ..=72.).contains(&self.font_size) { return Err(ToolError::invalid_input("Size must be from 6 to 72 points.")); }
        scene::check_text(&self.stamp(self.start, 612., 792.))
    }
}

/// The numbers set aside for one file: `pages` identifiers starting at `first`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatesRange { pub first: u64, pub pages: u64 }
impl BatesRange { pub fn last(&self) -> u64 { self.first + self.pages - 1 } }

/// Sets numbers aside by file order. A file whose page count is unknown gets no range, and the next file continues
/// from the next unused number.
pub fn assign(start: u64, page_counts: &[Result<u64, ToolError>]) -> Vec<Result<BatesRange, ToolError>> {
    let mut next = start;
    page_counts.iter().map(|count| count.clone().map(|pages| {
        let range = BatesRange { first: next, pages };
        next += pages;
        range
    })).collect()
}

fn load(path: &Path) -> Result<Document, String> {
    let document = Document::load(path).map_err(|error| error.to_string())?;
    if document.is_encrypted() { return Err("The PDF is password-protected. Unlock it first.".into()); }
    Ok(document)
}

fn page_count(path: &Path) -> Result<u64, ToolError> {
    let unreadable = |reason: String| ToolError::processing(format!(
        "Couldn't read how many pages this PDF has, so no numbers were set aside for it. Later files continue from the next number. {reason}"));
    let pages = load(path).map_err(unreadable)?.get_pages().len() as u64;
    if pages == 0 { return Err(unreadable("It has no pages.".into())); }
    Ok(pages)
}

/// Every path gets the same validation failure, so nothing is written.
fn reject_all(paths: &[PathBuf], error: ToolError) -> Vec<JobOutcome> {
    paths.iter().map(|path| JobOutcome::failure(path.clone(), error.clone())).collect()
}

pub struct BatesPlan { format: BatesFormat, ranges: HashMap<PathBuf, Result<BatesRange, ToolError>>, output_location: OutputLocation }

/// Validates the request and sets every file's numbers aside. Returns the files to run, or the outcomes when the whole
/// request is rejected before any file is read.
pub fn plan(request: &BatesRequest) -> Result<(BatesPlan, Vec<PathBuf>), Vec<JobOutcome>> {
    let run = request.run.clone().unwrap_or_else(|| request.paths.clone());
    if let Err(error) = request.format.validate() { return Err(reject_all(&run, error)); }
    let mut seen = HashSet::new();
    if let Some(repeated) = request.paths.iter().find(|path| !seen.insert(*path)) {
        let name = repeated.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(reject_all(&run, ToolError::invalid_input(format!("{name} is in the list twice. Each file can be numbered once, so remove the second copy."))));
    }
    if let Some(stray) = run.iter().find(|path| !seen.contains(path)) {
        return Err(reject_all(&run, ToolError::invalid_input(format!("{} is not in the numbered set.", stray.display()))));
    }
    let counts: Vec<_> = request.paths.iter().map(|path| page_count(path)).collect();
    let ranges = request.paths.iter().cloned().zip(assign(request.format.start, &counts)).collect();
    Ok((BatesPlan { format: request.format.clone(), ranges, output_location: request.output_location.clone() }, run))
}

/// Stamps one file with its set-aside numbers and saves it as "-bates", then "-1", "-2" when that name is taken.
pub fn number(plan: &BatesPlan, input: PathBuf) -> JobOutcome {
    let range = match &plan.ranges[&input] { Ok(range) => *range, Err(error) => return JobOutcome::failure(input, error.clone()) };
    let result = (|| {
        let mut doc = load(&input).map_err(ToolError::processing)?;
        let pages = scene::page_sizes(&doc).map_err(ToolError::processing)?;
        if pages.len() as u64 != range.pages {
            return Err(ToolError::processing("The PDF changed after its numbers were set aside. Run the set again."));
        }
        let stamps: Vec<_> = pages.iter().enumerate()
            .map(|(index, (id, width, height))| (*id, plan.format.stamp(range.first + index as u64, *width, *height))).collect();
        scene::stamp_text(&mut doc, &stamps)?;
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).map_err(|error| ToolError::processing(error.to_string()))?;
        let (reservation, mut file) = OutputNaming::create_single_output(None, &input, &plan.output_location, SUFFIX, "pdf")
            .map_err(|error| ToolError::processing(format!("Could not reserve the numbered PDF: {error}")))?;
        file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|error| ToolError::processing(error.to_string()))?;
        drop(file);
        reservation.publish().map_err(|error| ToolError::processing(error.to_string()))
    })();
    match result {
        Ok(path) => JobOutcome::succeeded(input, vec![path], plan.format.range(&range)),
        Err(error) => JobOutcome::failure(input, error),
    }
}

/// Puts each file's range in its result: the numbers stamped, or the numbers left unused when it failed.
pub fn report(plan: &BatesPlan, outcomes: Vec<JobOutcome>) -> Vec<JobOutcome> {
    outcomes.into_iter().map(|mut outcome| {
        if let Some(Ok(range)) = plan.ranges.get(&outcome.input_path) {
            let range = plan.format.range(range);
            outcome.detail = match outcome.state { OutcomeState::Succeeded { .. } => range, _ => format!("{range} unused") };
        }
        outcome
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, Stream};
    use std::fs;

    fn pdf(path: &Path, pages: usize) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let kids: Vec<Object> = (0..pages).map(|index| {
            let content = d.add_object(Stream::new(dictionary!{}, format!("BT /F1 12 Tf 72 720 Td (Page {}) Tj ET", index + 1).into_bytes()));
            Object::Reference(d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
                "Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}, "Contents"=>content}))
        }).collect();
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>pages as i64}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>root});
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
    }

    /// Readable page count, but a page without a MediaBox can't be stamped.
    fn unstampable(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let kids: Vec<Object> = (0..3).map(|_| Object::Reference(d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root}))).collect();
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>3}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>root});
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("toolbox-bates-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn format() -> BatesFormat {
        BatesFormat { prefix: "HL".into(), suffix: String::new(), start: 1, digits: 6, position: BatesPosition::BottomRight, font_family: None, font_size: 10. }
    }

    fn request(paths: &[PathBuf]) -> BatesRequest {
        BatesRequest { paths: paths.to_vec(), run: None, format: format(), output_location: OutputLocation::AlongsideInput }
    }

    fn run(request: &BatesRequest) -> Vec<JobOutcome> {
        let (plan, run) = match plan(request) { Ok(planned) => planned, Err(outcomes) => return outcomes };
        let outcomes = run.into_iter().map(|path| number(&plan, path)).collect();
        report(&plan, outcomes)
    }

    fn output(outcome: &JobOutcome) -> PathBuf {
        match &outcome.state { OutcomeState::Succeeded { output_paths } => output_paths[0].clone(), state => panic!("expected success, got {state:?}") }
    }

    fn stamped_text(path: &Path) -> Vec<String> {
        let doc = Document::load(path).unwrap();
        doc.get_pages().values().map(|id| {
            let content = String::from_utf8_lossy(&doc.get_page_content(*id)).into_owned();
            assert!(content.contains("/Artifact BMC"), "the stamp is marked as an artifact");
            let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
            let page = pages[doc.get_pages().values().position(|value| value == id).unwrap()];
            doc.extract_text(&[page]).unwrap_or_default()
        }).collect()
    }

    #[test]
    fn identifiers_follow_the_format_and_ranges_skip_unreadable_files() {
        let f = format();
        assert_eq!(f.identifier(1), "HL000001");
        assert_eq!(BatesFormat { suffix: "-C".into(), digits: 2, ..format() }.identifier(123), "HL123-C");
        let unreadable = ToolError::processing("unreadable");
        let ranges = assign(1, &[Ok(14), Err(unreadable.clone()), Ok(2), Ok(3)]);
        assert_eq!(ranges[0], Ok(BatesRange { first: 1, pages: 14 }));
        assert_eq!(ranges[1], Err(unreadable));
        assert_eq!(ranges[2], Ok(BatesRange { first: 15, pages: 2 }));
        assert_eq!(ranges[3], Ok(BatesRange { first: 17, pages: 3 }));
        assert_eq!(f.range(&BatesRange { first: 17, pages: 3 }), "HL000017 to HL000019");
        assert_eq!(f.range(&BatesRange { first: 5, pages: 1 }), "HL000005");
    }

    #[test]
    fn identifiers_continue_across_three_files_in_list_order() {
        let dir = folder("continuous");
        let paths: Vec<_> = [("c.pdf", 2), ("a.pdf", 3), ("b.pdf", 1)].iter().map(|(name, pages)| { let path = dir.join(name); pdf(&path, *pages); path }).collect();
        let outcomes = run(&request(&paths));
        let details: Vec<_> = outcomes.iter().map(|outcome| outcome.detail.as_str()).collect();
        assert_eq!(details, ["HL000001 to HL000002", "HL000003 to HL000005", "HL000006"]);
        assert_eq!(stamped_text(&output(&outcomes[0])).iter().filter(|text| text.contains("HL00000")).count(), 2);
        let texts: Vec<_> = outcomes.iter().flat_map(|outcome| stamped_text(&output(outcome))).collect();
        for (index, text) in texts.iter().enumerate() {
            assert!(text.contains(&format!("HL{:06}", index + 1)), "page {} reads {text:?}", index + 1);
            assert!(text.contains("Page"), "original page content is kept");
        }
        assert_eq!(output(&outcomes[1]).file_name().unwrap(), "a-bates.pdf");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failing_middle_file_leaves_a_reported_gap_and_the_others_keep_their_numbers() {
        let dir = folder("gap");
        let (first, middle, last) = (dir.join("first.pdf"), dir.join("middle.pdf"), dir.join("last.pdf"));
        pdf(&first, 2);
        unstampable(&middle);
        pdf(&last, 2);
        let outcomes = run(&request(&[first, middle.clone(), last]));
        assert_eq!(outcomes[0].detail, "HL000001 to HL000002");
        assert!(matches!(outcomes[1].state, OutcomeState::Failed { .. }));
        assert_eq!(outcomes[1].detail, "HL000003 to HL000005 unused");
        assert_eq!(outcomes[2].detail, "HL000006 to HL000007");
        assert!(stamped_text(&output(&outcomes[2]))[0].contains("HL000006"));
        assert!(!dir.join("middle-bates.pdf").exists(), "nothing is saved for the failed file");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_page_count_gets_no_range_and_later_files_continue() {
        let dir = folder("unreadable");
        let (first, broken, last) = (dir.join("first.pdf"), dir.join("broken.pdf"), dir.join("last.pdf"));
        pdf(&first, 1);
        fs::write(&broken, b"not a pdf").unwrap();
        pdf(&last, 2);
        let outcomes = run(&request(&[first, broken, last]));
        assert_eq!(outcomes[0].detail, "HL000001");
        let error = outcomes[1].error().expect("unreadable file fails");
        assert!(error.message.contains("no numbers were set aside") && error.message.contains("Later files continue"), "{}", error.message);
        assert_eq!(outcomes[1].detail, "");
        assert_eq!(outcomes[2].detail, "HL000002 to HL000003");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn outputs_are_named_bates_then_numbered_and_never_replace_files() {
        let dir = folder("naming");
        let input = dir.join("brief.pdf");
        pdf(&input, 1);
        let first = output(&run(&request(&[input.clone()]))[0]);
        let second = output(&run(&request(&[input.clone()]))[0]);
        assert_eq!(first.file_name().unwrap(), "brief-bates.pdf");
        assert_eq!(second.file_name().unwrap(), "brief-bates-1.pdf");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn repeated_runs_are_byte_identical_and_originals_are_unchanged() {
        let dir = folder("deterministic");
        let paths: Vec<_> = ["one.pdf", "two.pdf"].iter().map(|name| { let path = dir.join(name); pdf(&path, 2); path }).collect();
        let originals: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
        let first: Vec<_> = run(&request(&paths)).iter().map(|outcome| fs::read(output(outcome)).unwrap()).collect();
        let second: Vec<_> = run(&request(&paths)).iter().map(|outcome| fs::read(output(outcome)).unwrap()).collect();
        assert_eq!(first, second);
        assert_eq!(paths.iter().map(|path| fs::read(path).unwrap()).collect::<Vec<_>>(), originals);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_retry_keeps_the_numbers_set_aside_for_that_file() {
        let dir = folder("retry");
        let paths: Vec<_> = ["one.pdf", "two.pdf"].iter().map(|name| { let path = dir.join(name); pdf(&path, 2); path }).collect();
        let outcomes = run(&BatesRequest { run: Some(vec![paths[1].clone()]), ..request(&paths) });
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].detail, "HL000003 to HL000004");
        assert!(!dir.join("one-bates.pdf").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_formats_and_repeated_files_are_rejected_before_output() {
        let dir = folder("invalid");
        let input = dir.join("one.pdf");
        pdf(&input, 1);
        for format in [BatesFormat { digits: 0, ..format() }, BatesFormat { prefix: "A\nB".into(), ..format() }, BatesFormat { font_size: 200., ..format() }, BatesFormat { prefix: "雪".into(), ..format() }] {
            let outcomes = run(&BatesRequest { format, ..request(&[input.clone()]) });
            assert!(outcomes[0].error().is_some());
        }
        let outcomes = run(&request(&[input.clone(), input.clone()]));
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some_and(|error| error.message.contains("in the list twice"))));
        assert!(!dir.join("one-bates.pdf").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rotated_pages_are_stamped_upright_inside_the_visible_page() {
        let dir = folder("rotated");
        let input = dir.join("turned.pdf");
        pdf(&input, 1);
        let mut doc = Document::load(&input).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
        doc.save(&input).unwrap();
        let outcomes = run(&request(&[input]));
        let out = Document::load(output(&outcomes[0])).unwrap();
        let page = *out.get_pages().values().next().unwrap();
        let content = String::from_utf8_lossy(&out.get_page_content(page)).into_owned();
        // The inverse of the 90-degree visible-space matrix maps the stamp into the rotated page's user space.
        assert!(content.contains("0 1 1 0 0 0 cm"), "{content}");
        let _ = fs::remove_dir_all(dir);
    }
}
