use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use crate::kit::common::JobOutcome;
use crate::kit::contracts::{SkipCode, SkipReason, ToolError};

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

impl fmt::Display for PageRanges {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let spans = self.0.iter().map(PageSpan::to_string).collect::<Vec<_>>();
        formatter.write_str(&spans.join(", "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClampedRanges {
    pub spans: Vec<(usize, usize)>,
    pub notes: Vec<String>,
}

impl ClampedRanges {
    pub fn is_empty(&self) -> bool { self.spans.is_empty() }

    pub fn page_indices(&self) -> Vec<usize> {
        self.spans.iter().flat_map(|&(first, last)| first - 1..last).collect::<BTreeSet<_>>().into_iter().collect()
    }
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

    pub fn clamp_or_skip(&self, page_count: usize) -> Result<ClampedRanges, SkipReason> {
        let clamped = self.clamp(page_count);
        if clamped.is_empty() {
            return Err(SkipReason::new(SkipCode::RangeEmptyAfterClamping, format!("No pages in {self} are in this PDF, which has {}.", pages_label(page_count))));
        }
        Ok(clamped)
    }
}

pub fn validate_page_ranges(paths: &[PathBuf], raw: Option<&str>) -> Result<(), Vec<JobOutcome>> {
    match raw.filter(|value| !value.trim().is_empty()) {
        Some(raw) => require_page_ranges(paths, raw).map(drop),
        None => Ok(()),
    }
}

pub fn require_page_ranges(paths: &[PathBuf], raw: &str) -> Result<PageRanges, Vec<JobOutcome>> {
    PageRanges::parse(raw).map_err(|error| {
        if paths.is_empty() {
            return vec![JobOutcome::failure(PathBuf::new(), ToolError::invalid_input(error))];
        }
        paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input(error.clone()))).collect()
    })
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

fn pages_label(count: usize) -> String {
    if count == 1 { "1 page".to_string() } else { format!("{count} pages") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(first: usize, last: Option<usize>) -> PageSpan { PageSpan { first, last } }

    #[test]
    fn parser_accepts_and_rejects_the_shared_cases_with_their_messages() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../../../../shared/page-range-cases.json")).unwrap();
        for case in cases["accepted"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let expected = case["spans"].as_array().unwrap().iter()
                .map(|pair| span(pair[0].as_u64().unwrap() as usize, pair[1].as_u64().map(|last| last as usize)))
                .collect::<Vec<_>>();
            assert_eq!(PageRanges::parse(input).expect(input).spans(), expected, "{input}");
        }
        for case in cases["rejected"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(PageRanges::parse(input).expect_err(input), case["message"].as_str().unwrap(), "{input}");
        }
        for case in cases["within"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let within = PageRanges::parse(input).unwrap().within(case["pageCount"].as_u64().unwrap() as usize);
            match case.get("pages") {
                Some(pages) => assert_eq!(serde_json::to_value(within.unwrap()).unwrap(), *pages, "{input}"),
                None => assert_eq!(within.unwrap_err(), case["message"].as_str().unwrap(), "{input}"),
            }
        }
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
    fn page_indices_are_zero_based_sorted_and_unique() {
        assert_eq!(PageRanges::parse("5-6, 1-3, 2").unwrap().clamp(5).page_indices(), vec![0, 1, 2, 4]);
    }

    #[test]
    fn clamp_or_skip_names_the_range_and_length_when_nothing_remains() {
        let ranges = PageRanges::parse("2-4, 9-").unwrap();
        assert_eq!(ranges.clamp_or_skip(3).unwrap().spans, vec![(2, 3)]);
        assert_eq!(ranges.clamp_or_skip(1).unwrap_err(), SkipReason::new(SkipCode::RangeEmptyAfterClamping, "No pages in 2-4, 9- are in this PDF, which has 1 page."));
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
        assert_eq!(require_page_ranges(&paths, " ").unwrap_err().len(), 2);
        assert_eq!(require_page_ranges(&paths, "2-4, 9-").unwrap().to_string(), "2-4, 9-");
    }
}
