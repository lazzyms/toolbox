use std::fmt;
use std::path::PathBuf;

use crate::kit::common::JobOutcome;
use crate::kit::contracts::ToolError;

const EXAMPLE: &str = "1-3, 5, 8-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSpan {
    pub first: usize,
    pub last: Option<usize>,
}

impl fmt::Display for PageSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.last {
            Some(last) if last == self.first => write!(formatter, "{}", self.first),
            Some(last) => write!(formatter, "{}-{last}", self.first),
            None => write!(formatter, "{}-", self.first),
        }
    }
}

impl PageSpan {
    fn described(self) -> String {
        if self.last == Some(self.first) { format!("page {self}") } else { format!("pages {self}") }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRanges(Vec<PageSpan>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClampedRanges {
    pub spans: Vec<(usize, usize)>,
    pub notes: Vec<String>,
}

impl ClampedRanges {
    pub fn is_empty(&self) -> bool { self.spans.is_empty() }
}

impl PageRanges {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let entries = raw.split(',').map(str::trim).collect::<Vec<_>>();
        if entries.iter().all(|entry| entry.is_empty()) {
            return Err(format!("Enter at least one page or range, for example {EXAMPLE}."));
        }
        entries.into_iter().enumerate().map(|(index, entry)| parse_entry(index, entry)).collect::<Result<_, _>>().map(Self)
    }

    pub fn spans(&self) -> &[PageSpan] { &self.0 }

    pub fn within(&self, page_count: usize) -> Result<Vec<(usize, usize)>, String> {
        self.0.iter().map(|span| {
            let last = span.last.unwrap_or(page_count);
            if span.first > page_count || last > page_count {
                return Err(format!("The PDF has {}, so {} can't be used.", pages_label(page_count), span.described()));
            }
            Ok((span.first, last))
        }).collect()
    }

    pub fn clamp(&self, page_count: usize) -> ClampedRanges {
        let mut clamped = ClampedRanges { spans: Vec::new(), notes: Vec::new() };
        for span in &self.0 {
            if span.first > page_count {
                clamped.notes.push(format!("Skipped {} because the PDF has {}.", span.described(), pages_label(page_count)));
                continue;
            }
            let last = span.last.map_or(page_count, |last| last.min(page_count));
            if span.last.is_some_and(|requested| requested > last) {
                let kept = PageSpan { first: span.first, last: Some(last) };
                clamped.notes.push(format!("Shortened {} to {kept} because the PDF has {}.", span.described(), pages_label(page_count)));
            }
            clamped.spans.push((span.first, last));
        }
        clamped
    }
}

pub fn validate_page_ranges(paths: &[PathBuf], raw: Option<&str>) -> Result<(), Vec<JobOutcome>> {
    let Some(Err(error)) = raw.filter(|value| !value.trim().is_empty()).map(PageRanges::parse) else { return Ok(()); };
    if paths.is_empty() {
        return Err(vec![JobOutcome::failure(PathBuf::new(), ToolError::invalid_input(error))]);
    }
    Err(paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input(error.clone()))).collect())
}

fn parse_entry(index: usize, entry: &str) -> Result<PageSpan, String> {
    if entry.is_empty() {
        return Err(format!("Entry {} of the page range is empty. Remove the extra comma.", index + 1));
    }
    let (first, last) = match entry.split_once('-') {
        Some((first, last)) => (first.trim(), Some(last.trim())),
        None => (entry, None),
    };
    let first = page_number(entry, first)?;
    let last = match last {
        None => Some(first),
        Some("") => None,
        Some(last) => Some(page_number(entry, last)?),
    };
    if last.is_some_and(|last| last < first) {
        return Err(format!("\"{entry}\" runs backwards. Put the lower page first, for example {}-{first}.", last.unwrap_or(first)));
    }
    Ok(PageSpan { first, last })
}

fn page_number(entry: &str, value: &str) -> Result<usize, String> {
    let not_a_page = || format!("\"{entry}\" isn't a page or range. Use pages and ranges like {EXAMPLE}.");
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) { return Err(not_a_page()); }
    match value.parse::<usize>() {
        Ok(0) => Err(format!("\"{entry}\" uses page 0. Page numbers start at 1.")),
        Ok(page) => Ok(page),
        Err(_) => Err(not_a_page()),
    }
}

pub(crate) fn pages_label(count: usize) -> String {
    if count == 1 { "1 page".to_string() } else { format!("{count} pages") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(first: usize, last: Option<usize>) -> PageSpan { PageSpan { first, last } }

    #[test]
    fn parses_pages_spans_and_open_ends_in_written_order() {
        let ranges = PageRanges::parse(" 1-3, 5,8 - ,10 - 12 ").unwrap();
        assert_eq!(ranges.spans(), [span(1, Some(3)), span(5, Some(5)), span(8, None), span(10, Some(12))]);
    }

    #[test]
    fn rejects_malformed_entries_naming_the_bad_one() {
        for (raw, expected) in [
            ("", "Enter at least one page or range"),
            (" , ", "Enter at least one page or range"),
            ("1-3,,5", "Entry 2 of the page range is empty"),
            ("1-3,", "Entry 2 of the page range is empty"),
            ("5-3", "\"5-3\" runs backwards"),
            ("0", "\"0\" uses page 0"),
            ("2, 0-4", "\"0-4\" uses page 0"),
            ("1-0", "\"1-0\" uses page 0"),
            ("1, two", "\"two\" isn't a page or range"),
            ("-4", "\"-4\" isn't a page or range"),
            ("1-2-3", "\"1-2-3\" isn't a page or range"),
            ("+2", "\"+2\" isn't a page or range"),
            ("99999999999999999999999", "isn't a page or range"),
        ] {
            let error = PageRanges::parse(raw).expect_err(raw);
            assert!(error.contains(expected), "{raw}: {error}");
        }
    }

    #[test]
    fn within_resolves_open_ends_and_fails_on_pages_past_the_end() {
        let ranges = PageRanges::parse("1-3, 5, 8-").unwrap();
        assert_eq!(ranges.within(10).unwrap(), vec![(1, 3), (5, 5), (8, 10)]);
        assert_eq!(ranges.within(7).unwrap_err(), "The PDF has 7 pages, so pages 8- can't be used.");
        assert_eq!(PageRanges::parse("2-4").unwrap().within(3).unwrap_err(), "The PDF has 3 pages, so pages 2-4 can't be used.");
    }

    #[test]
    fn clamp_keeps_fitting_spans_without_notes() {
        let clamped = PageRanges::parse("1-3, 5, 8-").unwrap().clamp(10);
        assert_eq!(clamped, ClampedRanges { spans: vec![(1, 3), (5, 5), (8, 10)], notes: vec![] });
        assert!(!clamped.is_empty());
    }

    #[test]
    fn clamp_shortens_spans_past_the_end_and_drops_spans_that_start_after_it() {
        let clamped = PageRanges::parse("2-6, 4, 7, 9-, 3-").unwrap().clamp(4);
        assert_eq!(clamped.spans, vec![(2, 4), (4, 4), (3, 4)]);
        assert_eq!(clamped.notes, vec![
            "Shortened pages 2-6 to 2-4 because the PDF has 4 pages.",
            "Skipped page 7 because the PDF has 4 pages.",
            "Skipped pages 9- because the PDF has 4 pages.",
        ]);
        assert!(!clamped.is_empty());
    }

    #[test]
    fn clamp_reports_an_empty_result_when_nothing_remains() {
        let clamped = PageRanges::parse("3-5, 8").unwrap().clamp(1);
        assert!(clamped.is_empty());
        assert_eq!(clamped.notes, vec![
            "Skipped pages 3-5 because the PDF has 1 page.",
            "Skipped page 8 because the PDF has 1 page.",
        ]);
    }

    #[test]
    fn boundary_rejects_every_input_as_invalid_input_and_ignores_blank_ranges() {
        let paths = vec![PathBuf::from("a.pdf"), PathBuf::from("b.pdf")];
        assert!(validate_page_ranges(&paths, None).is_ok());
        assert!(validate_page_ranges(&paths, Some("  ")).is_ok());
        assert!(validate_page_ranges(&paths, Some("1-3, 5, 8-")).is_ok());
        let outcomes = validate_page_ranges(&paths, Some("3-1")).unwrap_err();
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some_and(|error| error.kind == crate::kit::contracts::ErrorKind::InvalidInput && error.message.contains("\"3-1\""))));
    }
}
