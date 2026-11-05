//! Privacy Check: reads what a PDF reveals besides its pages. It only reads, so it shares no code path that writes a PDF.

use std::path::{Path, PathBuf};

use lopdf::{Document, Object, Permissions};
use serde::{Deserialize, Serialize};

use super::document_info::{self, PdfProperties, PropertyConflict, XmpSummary};
use super::hidden_content::{self, HiddenContent};
use crate::kit::contracts::{JobOutcome, ToolError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub password: Option<String>,
}

/// One PDF's report. Every category is present; an empty one means none found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyReport {
    pub page_count: usize,
    pub document_information: PdfProperties,
    pub xmp: XmpReport,
    pub encryption: Option<EncryptionReport>,
    pub identifiers: IdentifierReport,
    #[serde(flatten)]
    pub hidden: HiddenContent,
    pub unsupported: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XmpReport {
    pub summary: Option<XmpSummary>,
    pub conflicts: Vec<PropertyConflict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Print,
    PrintHighQuality,
    Modify,
    Copy,
    CopyForAccessibility,
    Annotate,
    FillForms,
    Assemble,
}

impl Permission {
    const ALL: [(Self, Permissions); 8] = [
        (Self::Print, Permissions::PRINTABLE),
        (Self::PrintHighQuality, Permissions::PRINTABLE_IN_HIGH_QUALITY),
        (Self::Modify, Permissions::MODIFIABLE),
        (Self::Copy, Permissions::COPYABLE),
        (Self::CopyForAccessibility, Permissions::COPYABLE_FOR_ACCESSIBILITY),
        (Self::Annotate, Permissions::ANNOTABLE),
        (Self::FillForms, Permissions::FILLABLE),
        (Self::Assemble, Permissions::ASSEMBLABLE),
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionReport {
    /// False when anyone can open the file without a password.
    pub open_password: bool,
    pub version: i64,
    pub revision: i64,
    pub key_bits: Option<usize>,
    pub encrypts_metadata: bool,
    pub allowed: Vec<Permission>,
    pub denied: Vec<Permission>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentifierReport {
    pub document_id: Option<DocumentId>,
    pub producer: Option<ProducerSoftware>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentId {
    /// Hex of the ID set at the first save.
    pub permanent: String,
    /// Hex of the ID set at the latest save.
    pub changing: String,
    /// True when both halves match, so the file still carries its first-save ID unchanged.
    pub unchanged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerSoftware {
    pub name: String,
    pub version: Option<String>,
}

pub fn inspect(path: &Path, password: Option<&str>) -> Result<PrivacyReport, ToolError> {
    let bytes = document_info::read_bytes(path)?;
    let document = document_info::load(&bytes, password)?;
    let details = document_info::from_document(&document);
    let mut unsupported = details.unsupported;
    let hidden = hidden_content::read(&document, &bytes, &mut unsupported);
    let encryption = document.encryption_state.as_ref().map(|state| {
        let permissions = state.permissions();
        let (allowed, denied) = Permission::ALL.into_iter().partition::<Vec<_>, _>(|(_, flag)| permissions.contains(*flag));
        EncryptionReport {
            // The loader tries an empty password first, so a supplied one was needed only if the file won't open without it.
            open_password: password.is_some() && document_info::load(&bytes, None).is_err(),
            version: state.version(),
            revision: state.revision(),
            key_bits: state.key_length(),
            encrypts_metadata: state.encrypt_metadata(),
            allowed: allowed.into_iter().map(|(permission, _)| permission).collect(),
            denied: denied.into_iter().map(|(permission, _)| permission).collect(),
        }
    });
    let identifiers = IdentifierReport {
        document_id: document_id(&document, &mut unsupported),
        producer: details.info.producer.as_deref().map(|name| ProducerSoftware { name: name.to_owned(), version: software_version(name) }),
    };
    Ok(PrivacyReport {
        page_count: document.get_pages().len(),
        document_information: details.info,
        xmp: XmpReport { summary: details.xmp, conflicts: details.conflicts },
        encryption,
        identifiers,
        hidden,
        unsupported,
    })
}

/// The batch job: one report per file in the outcome's detail, and never an output path.
pub fn inspect_job(request: &PrivacyRequest, input: PathBuf) -> JobOutcome {
    match inspect(&input, request.password.as_deref()) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(detail) => JobOutcome::succeeded(input, Vec::new(), detail),
            Err(error) => JobOutcome::failure(input, ToolError::processing(format!("Could not build the report: {error}"))),
        },
        Err(error) => JobOutcome::failure(input, error),
    }
}

fn document_id(document: &Document, unsupported: &mut Vec<String>) -> Option<DocumentId> {
    let id = document.trailer.get(b"ID").ok()?;
    let halves = super::resolve(document, id).ok().and_then(|id| match id.as_array() {
        Ok(items) if items.len() == 2 => {
            let half = |item: &Object| super::resolve(document, item).ok().and_then(|item| item.as_str().ok().map(hex));
            Some((half(&items[0])?, half(&items[1])?))
        }
        _ => None,
    });
    match halves {
        Some((permanent, changing)) => Some(DocumentId { unchanged: permanent == changing, permanent, changing }),
        None => {
            unsupported.push("The document ID could not be read.".to_string());
            None
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The first dotted number in a producer string, such as 9.0.1 in "Acrobat Distiller 9.0.1 (Windows)".
fn software_version(producer: &str) -> Option<String> {
    producer
        .split(|character: char| character.is_whitespace() || matches!(character, '(' | ')' | ';' | ',' | '/'))
        .map(|token| token.trim_start_matches(['v', 'V']).trim_end_matches('.'))
        .find(|token| {
            token.starts_with(|character: char| character.is_ascii_digit())
                && token.contains('.')
                && token.chars().all(|character| character.is_ascii_digit() || character == '.')
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
    use crate::kit::contracts::{ErrorKind, OutcomeState};
    use document_info::{DocumentProperty, PdfDate};
    use hidden_content::{ActionTrigger, AnnotationGroup, AutomaticAction, EmbeddedFile, HiddenItem, HiddenReason, Layer, Leftovers, LinkHost, OptionalContent, PagePiece, Signature};
    use lopdf::{dictionary, Dictionary, EncryptionState, EncryptionVersion, Stream, StringFormat};
    use std::sync::Arc;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("toolbox-privacy-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn names(&self) -> Vec<String> {
            let mut names = std::fs::read_dir(&self.0).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
            names.sort();
            names
        }

        /// Inspects one file and checks that inspection wrote nothing and left the original byte-identical.
        fn inspect(&self, bytes: &[u8], password: Option<&str>) -> Result<PrivacyReport, ToolError> {
            let path = self.file("input.pdf", bytes);
            let report = inspect(&path, password);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the original must stay byte-identical");
            assert_eq!(self.names(), ["input.pdf"], "inspection must create no files");
            report
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn literal(bytes: &[u8]) -> Object {
        Object::String(bytes.to_vec(), StringFormat::Literal)
    }

    struct Fixture {
        info: Option<Dictionary>,
        xmp: Option<String>,
        id: Option<(&'static [u8], &'static [u8])>,
        encryption: Option<(&'static str, &'static str, Permissions)>,
        plant: Option<fn(&mut Document, &mut Dictionary, &mut Dictionary)>,
    }

    const CLEAN: Fixture = Fixture { info: None, xmp: None, id: None, encryption: None, plant: None };

    fn pdf(fixture: Fixture) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        };
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if let Some(plant) = fixture.plant {
            plant(&mut document, &mut catalog, &mut page);
        }
        let page_id = document.add_object(page);
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }.into());
        if let Some(xmp) = fixture.xmp {
            let stream = Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, xmp.into_bytes());
            catalog.set("Metadata", document.add_object(stream));
        }
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        if let Some(info) = fixture.info {
            let info_id = document.add_object(info);
            document.trailer.set("Info", info_id);
        }
        if let Some((permanent, changing)) = fixture.id {
            document.trailer.set("ID", Object::Array(vec![literal(permanent), literal(changing)]));
        }
        if let Some((owner_password, user_password, permissions)) = fixture.encryption {
            let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &document, owner_password, user_password, key_length: 128, permissions }).unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn xmp(description: &str) -> String {
        format!(r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{description}</rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#)
    }

    fn information() -> Fixture {
        Fixture {
            info: Some(dictionary! {
                "Author" => literal(b"Legal Ops, Acme Ltd"),
                "Title" => literal(b"NDA draft v7"),
                "Producer" => literal(b"Acme PDF Writer 4.2.1 (Windows)"),
                "CreationDate" => literal(b"D:20240102030405Z"),
            }),
            xmp: None,
            id: Some((b"\x9f\x3c\xa7\x1e", b"\x9f\x3c\xa7\x1e")),
            ..CLEAN
        }
    }

    fn conflicting_xmp() -> Fixture {
        Fixture {
            info: Some(dictionary! { "Author" => literal(b"A. Patel") }),
            xmp: Some(xmp(r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><rdf:Seq><rdf:li>A. Patel (draft)</rdf:li></rdf:Seq></dc:creator></rdf:Description>"#)),
            ..CLEAN
        }
    }

    fn restricted() -> Fixture {
        Fixture { info: None, xmp: None, id: Some((b"first", b"later")), encryption: Some(("owner", "", Permissions::PRINTABLE | Permissions::COPYABLE_FOR_ACCESSIBILITY)), plant: None }
    }

    fn name(value: &str) -> Object {
        Object::Name(value.as_bytes().to_vec())
    }

    fn rect(values: [i64; 4]) -> Object {
        Object::Array(values.into_iter().map(Object::Integer).collect())
    }

    /// Plants one or more items in every hidden-content category, plus items each category must leave out.
    fn plant_hidden_content(document: &mut Document, catalog: &mut Dictionary, page: &mut Dictionary) {
        let sheet = document.add_object(Stream::new(dictionary! {
            "Type" => "EmbeddedFile", "Subtype" => name("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), "Params" => dictionary! { "Size" => 49152 },
        }, b"PK".to_vec()));
        let sheet_spec = document.add_object(dictionary! { "Type" => "Filespec", "F" => literal(b"rent.xlsx"), "UF" => literal(b"rent-schedule.xlsx"), "EF" => dictionary! { "F" => sheet } });
        let script = document.add_object(dictionary! { "S" => "JavaScript", "JS" => literal(b"app.alert('hi')") });
        catalog.set("Names", dictionary! {
            "EmbeddedFiles" => dictionary! { "Names" => vec![literal(b"rent-schedule.xlsx"), sheet_spec.into()] },
            "JavaScript" => dictionary! { "Names" => vec![literal(b"init"), script.into()] },
        });
        catalog.set("OpenAction", dictionary! { "S" => "GoTo", "D" => vec![0.into(), name("Fit")] });
        catalog.set("AA", dictionary! { "WP" => dictionary! { "S" => "JavaScript", "JS" => literal(b"stamp()"), "Next" => dictionary! { "S" => "Launch", "F" => literal(b"cmd.exe") } } });

        let draft = document.add_object(dictionary! { "Type" => "OCG", "Name" => literal(b"Draft notes") });
        let final_layer = document.add_object(dictionary! { "Type" => "OCG", "Name" => literal(b"Final") });
        catalog.set("OCProperties", dictionary! { "OCGs" => vec![draft.into(), final_layer.into()], "D" => dictionary! { "OFF" => vec![draft.into()] } });

        let tenant_name = document.add_object(dictionary! { "T" => literal(b"name"), "V" => literal(b"Asha Patel") });
        let fields = [
            dictionary! { "FT" => "Tx", "T" => literal(b"tenant"), "Kids" => vec![tenant_name.into()] },
            dictionary! { "FT" => "Tx", "T" => literal(b"deposit"), "V" => literal(b"") },
            dictionary! { "FT" => "Btn", "T" => literal(b"agree"), "V" => name("Off") },
            dictionary! { "FT" => "Btn", "T" => literal(b"pets"), "V" => name("Yes") },
            dictionary! { "FT" => "Sig", "T" => literal(b"approver"), "V" => dictionary! { "Type" => "Sig", "Name" => literal(b"Acme Ltd"), "M" => literal(b"D:20260211093000+01'00'") } },
            dictionary! { "FT" => "Sig", "T" => literal(b"witness") },
        ].into_iter().map(|field| Object::Reference(document.add_object(field))).collect::<Vec<_>>();
        catalog.set("AcroForm", dictionary! { "Fields" => fields, "XFA" => literal(b"<xdp/>") });

        let photo = document.add_object(Stream::new(dictionary! { "Type" => "EmbeddedFile" }, b"jpegdata".to_vec()));
        let link = |uri: &[u8]| dictionary! { "Subtype" => "Link", "Rect" => rect([0, 0, 10, 10]), "A" => dictionary! { "S" => "URI", "URI" => literal(uri) } };
        let annotations = [
            dictionary! { "Subtype" => "Text", "Rect" => rect([10, 10, 30, 30]), "T" => literal(b"Asha Patel") },
            dictionary! { "Subtype" => "Text", "Rect" => rect([10, 40, 30, 60]), "T" => literal(b"J. Morgan"), "F" => 2 },
            dictionary! { "Subtype" => "Highlight", "Rect" => rect([50, 50, 90, 60]), "T" => literal(b"Asha Patel") },
            dictionary! { "Subtype" => "Square", "Rect" => rect([700, 800, 750, 850]) },
            dictionary! { "Subtype" => "Popup", "Rect" => rect([0, 0, 1, 1]), "F" => 2 },
            dictionary! { "Subtype" => "FileAttachment", "Rect" => rect([100, 100, 120, 120]), "T" => literal(b"Asha Patel"), "FS" => dictionary! { "Type" => "Filespec", "UF" => literal(b"photo.jpg"), "EF" => dictionary! { "F" => photo } } },
            dictionary! { "Subtype" => "FileAttachment", "Rect" => rect([130, 100, 150, 120]), "FS" => sheet_spec },
            link(b"https://Elmrow-Lettings.example/rent?x=1"),
            link(b"HTTP://user@GOV.UK:443/path"),
            link(b"https://gov.uk/other"),
            dictionary! { "Subtype" => "Link", "Rect" => rect([0, 0, 10, 10]), "A" => dictionary! { "S" => "GoTo", "D" => vec![0.into(), name("Fit")] } },
            dictionary! { "Subtype" => "Widget", "Rect" => rect([200, 200, 300, 220]), "AA" => dictionary! { "K" => dictionary! { "S" => "JavaScript", "JS" => literal(b"AFNumber_Keystroke()") } } },
        ].into_iter().map(|annotation| Object::Reference(document.add_object(annotation))).collect::<Vec<_>>();
        page.set("Annots", annotations);
        page.set("AA", dictionary! { "O" => dictionary! { "S" => "URI", "URI" => literal(b"https://tracker.example/opened") } });
        page.set("Thumb", document.add_object(Stream::new(dictionary! { "Width" => 1, "Height" => 1 }, vec![0, 0, 0])));
        page.set("PieceInfo", dictionary! { "Illustrator" => dictionary! { "LastModified" => literal(b"D:20260101000000Z"), "Private" => dictionary! {} } });
    }

    fn hidden_content() -> Fixture {
        Fixture { plant: Some(plant_hidden_content), ..CLEAN }
    }

    fn last_startxref(bytes: &[u8]) -> usize {
        let start = bytes.windows(9).rposition(|window| window == b"startxref").unwrap();
        String::from_utf8_lossy(&bytes[start + 9..]).split_whitespace().next().unwrap().parse().unwrap()
    }

    /// Appends a save the way an editor does: a new object, a cross-reference section and a trailer pointing back to the last one.
    fn append_save(bytes: &mut Vec<u8>, object: &str) {
        let document = Document::load_mem(bytes).unwrap();
        let root = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let id = document.max_id + 1;
        let start = bytes.len() + 1;
        bytes.extend(format!("\n{id} 0 obj\n{object}\nendobj\n").as_bytes());
        let xref = bytes.len();
        let previous = last_startxref(bytes);
        bytes.extend(format!("xref\n0 1\n0000000000 65535 f \n{id} 1\n{start:010} 00000 n \ntrailer\n<< /Size {} /Root {} {} R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n", id + 1, root.0, root.1).as_bytes());
    }

    fn with_incremental_updates(mut bytes: Vec<u8>, count: usize) -> Vec<u8> {
        for revision in 0..count {
            append_save(&mut bytes, &format!("(revision {revision})"));
        }
        bytes
    }

    fn expected_hidden_content(incremental_updates: usize) -> HiddenContent {
        HiddenContent {
            incremental_updates: Some(incremental_updates),
            embedded_files: vec![
                EmbeddedFile { name: "rent-schedule.xlsx".into(), size: Some(49152), mime_type: Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into()), page: None },
                EmbeddedFile { name: "photo.jpg".into(), size: Some(8), mime_type: None, page: Some(1) },
            ],
            actions: vec![
                AutomaticAction { trigger: ActionTrigger::DocumentScript, page: None, name: Some("init".into()), kinds: vec!["JavaScript".into()] },
                AutomaticAction { trigger: ActionTrigger::BeforePrint, page: None, name: None, kinds: vec!["JavaScript".into(), "Launch".into()] },
                AutomaticAction { trigger: ActionTrigger::PageOpen, page: Some(1), name: None, kinds: vec!["URI".into()] },
                AutomaticAction { trigger: ActionTrigger::Annotation, page: Some(1), name: None, kinds: vec!["JavaScript".into()] },
            ],
            annotations: vec![
                AnnotationGroup { subtype: "FileAttachment".into(), count: 2, authors: vec!["Asha Patel".into()] },
                AnnotationGroup { subtype: "Highlight".into(), count: 1, authors: vec!["Asha Patel".into()] },
                AnnotationGroup { subtype: "Square".into(), count: 1, authors: vec![] },
                AnnotationGroup { subtype: "Text".into(), count: 2, authors: vec!["Asha Patel".into(), "J. Morgan".into()] },
            ],
            filled_form_fields: vec!["tenant.name".into(), "pets".into()],
            optional_content: OptionalContent {
                layers: vec![Layer { name: "Draft notes".into(), visible: false }, Layer { name: "Final".into(), visible: true }],
                hidden: vec![
                    HiddenItem { page: 1, subtype: "Text".into(), reason: HiddenReason::Hidden },
                    HiddenItem { page: 1, subtype: "Square".into(), reason: HiddenReason::OffPage },
                ],
            },
            links: vec![LinkHost { host: "elmrow-lettings.example".into(), count: 1 }, LinkHost { host: "gov.uk".into(), count: 2 }],
            signatures: vec![Signature {
                field: "approver".into(),
                signer: Some("Acme Ltd".into()),
                signed_at: Some(PdfDate { year: 2026, month: 2, day: 11, hour: 9, minute: 30, second: 0, offset_minutes: Some(60) }),
            }],
            leftovers: Leftovers { thumbnail_pages: vec![1], page_pieces: vec![PagePiece { page: 1, applications: vec!["Illustrator".into()] }] },
        }
    }

    fn assert_none_found(report: &PrivacyReport) {
        assert_eq!(report.document_information, PdfProperties::default(), "document information: none found");
        assert_eq!(report.xmp, XmpReport::default(), "XMP: none found");
        assert_eq!(report.encryption, None, "encryption: none found");
        assert_eq!(report.identifiers, IdentifierReport::default(), "identifiers: none found");
        assert_eq!(report.hidden, HiddenContent { incremental_updates: Some(0), ..HiddenContent::default() }, "hidden content: none found");
        assert!(report.unsupported.is_empty(), "unsupported: none found");
    }

    #[test]
    fn a_clean_pdf_reports_none_found_in_every_category() {
        let sandbox = Sandbox::new("clean");
        let report = sandbox.inspect(&pdf(CLEAN), None).unwrap();
        assert_eq!(report.page_count, 1);
        assert_none_found(&report);
    }

    #[test]
    fn reports_exactly_the_planted_information_entries_identifier_and_producer_version() {
        let sandbox = Sandbox::new("information");
        let report = sandbox.inspect(&pdf(information()), None).unwrap();

        assert_eq!(report.document_information, PdfProperties {
            title: Some("NDA draft v7".into()),
            author: Some("Legal Ops, Acme Ltd".into()),
            producer: Some("Acme PDF Writer 4.2.1 (Windows)".into()),
            creation_date: Some(PdfDate { year: 2024, month: 1, day: 2, hour: 3, minute: 4, second: 5, offset_minutes: Some(0) }),
            ..PdfProperties::default()
        });
        assert_eq!(report.identifiers, IdentifierReport {
            document_id: Some(DocumentId { permanent: "9f3ca71e".into(), changing: "9f3ca71e".into(), unchanged: true }),
            producer: Some(ProducerSoftware { name: "Acme PDF Writer 4.2.1 (Windows)".into(), version: Some("4.2.1".into()) }),
        });
        assert_eq!(report.xmp, XmpReport::default());
        assert_eq!(report.encryption, None);
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn reports_the_xmp_packet_and_exactly_the_value_that_conflicts() {
        let sandbox = Sandbox::new("xmp");
        let report = sandbox.inspect(&pdf(conflicting_xmp()), None).unwrap();

        let summary = report.xmp.summary.as_ref().unwrap();
        assert_eq!(summary.values.author.as_deref(), Some("A. Patel (draft)"));
        assert_eq!(summary.properties, ["dc:creator"]);
        assert_eq!(report.xmp.conflicts, [PropertyConflict { property: DocumentProperty::Author, info: "A. Patel".into(), xmp: "A. Patel (draft)".into() }]);
        assert_eq!(report.document_information, PdfProperties { author: Some("A. Patel".into()), ..PdfProperties::default() });
        assert_eq!(report.encryption, None);
        assert_eq!(report.identifiers, IdentifierReport::default());
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn reports_restricted_permissions_of_a_pdf_that_opens_without_a_password() {
        let sandbox = Sandbox::new("restricted");
        let report = sandbox.inspect(&pdf(restricted()), None).unwrap();

        let encryption = report.encryption.as_ref().unwrap();
        assert!(!encryption.open_password, "anyone can open it");
        assert_eq!((encryption.version, encryption.revision, encryption.key_bits), (2, 3, Some(128)));
        assert_eq!(encryption.allowed, [Permission::Print, Permission::CopyForAccessibility]);
        assert_eq!(encryption.denied, [Permission::PrintHighQuality, Permission::Modify, Permission::Copy, Permission::Annotate, Permission::FillForms, Permission::Assemble]);
        assert_eq!(report.identifiers.document_id, Some(DocumentId { permanent: hex(b"first"), changing: hex(b"later"), unchanged: false }));
        assert_eq!(report.document_information, PdfProperties::default());
        assert_eq!(report.xmp, XmpReport::default());
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn a_pdf_that_needs_an_open_password_reports_it_once_opened() {
        let sandbox = Sandbox::new("open-password");
        let bytes = pdf(Fixture { encryption: Some(("owner", crate::kit::TEST_PASSWORD, Permissions::all())), ..restricted() });

        assert!(matches!(sandbox.inspect(&bytes, None).unwrap_err().kind, ErrorKind::InvalidInput));
        let encryption = sandbox.inspect(&bytes, Some(crate::kit::TEST_PASSWORD)).unwrap().encryption.unwrap();
        assert!(encryption.open_password);
        assert!(encryption.denied.is_empty());
    }

    #[test]
    fn an_unreadable_document_id_is_listed_as_unsupported() {
        let sandbox = Sandbox::new("bad-id");
        let mut document = Document::load_mem(&pdf(CLEAN)).unwrap();
        document.trailer.set("ID", Object::Integer(7));
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();

        let report = sandbox.inspect(&bytes, None).unwrap();
        assert_eq!(report.identifiers.document_id, None);
        assert_eq!(report.unsupported, ["The document ID could not be read."]);
    }

    #[test]
    fn reports_exactly_the_planted_hidden_content_and_lists_xfa_as_unsupported() {
        let sandbox = Sandbox::new("hidden");
        let report = sandbox.inspect(&pdf(hidden_content()), None).unwrap();

        assert_eq!(report.hidden, expected_hidden_content(0));
        assert_eq!(report.unsupported, ["XFA form data could not be checked."]);
        assert_eq!(report.document_information, PdfProperties::default());
        assert_eq!(report.xmp, XmpReport::default());
        assert_eq!(report.encryption, None);
    }

    #[test]
    fn counts_each_incremental_update_and_nothing_else_changes() {
        let sandbox = Sandbox::new("incremental");
        for count in [1, 2] {
            let report = sandbox.inspect(&with_incremental_updates(pdf(CLEAN), count), None).unwrap();
            assert_eq!(report.hidden, HiddenContent { incremental_updates: Some(count), ..HiddenContent::default() }, "{count} updates");
            assert!(report.unsupported.is_empty());
        }
        let report = sandbox.inspect(&with_incremental_updates(pdf(hidden_content()), 2), None).unwrap();
        assert_eq!(report.hidden, expected_hidden_content(2));
    }

    #[test]
    fn a_linearized_first_save_is_not_an_incremental_update() {
        let sandbox = Sandbox::new("linearized");
        let mut bytes = pdf(CLEAN);
        append_save(&mut bytes, "<< /Linearized 1 >>");

        assert_eq!(sandbox.inspect(&bytes, None).unwrap().hidden.incremental_updates, Some(0));
        assert_eq!(sandbox.inspect(&with_incremental_updates(bytes, 1), None).unwrap().hidden.incremental_updates, Some(1));
    }

    #[test]
    fn an_unreadable_save_history_is_listed_as_unsupported() {
        let sandbox = Sandbox::new("bad-history");
        let mut bytes = with_incremental_updates(pdf(CLEAN), 1);
        let own_section = last_startxref(&bytes);
        bytes.truncate(bytes.windows(5).rposition(|window| window == b"/Prev").unwrap());
        bytes.extend(format!("/Prev {own_section} >>\nstartxref\n{own_section}\n%%EOF\n").as_bytes());

        let report = sandbox.inspect(&bytes, None).unwrap();
        assert_eq!(report.hidden, HiddenContent::default());
        assert_eq!(report.unsupported, ["The file's save history could not be read."]);
    }

    #[test]
    fn hidden_content_of_an_encrypted_pdf_is_read_once_it_opens() {
        let sandbox = Sandbox::new("encrypted-hidden");
        let bytes = pdf(Fixture { id: Some((b"first", b"later")), encryption: Some(("owner", crate::kit::TEST_PASSWORD, Permissions::all())), ..hidden_content() });

        let report = sandbox.inspect(&bytes, Some(crate::kit::TEST_PASSWORD)).unwrap();
        assert_eq!(report.hidden, expected_hidden_content(0));
    }

    #[test]
    fn link_hosts_are_the_domain_or_the_scheme() {
        for (uri, expected) in [
            ("https://Example.COM/path?q=1", "example.com"),
            ("http://user:pass@sub.example.org:8080", "sub.example.org"),
            ("https://[2001:db8::1]:443/x", "2001:db8::1"),
            ("mailto:Asha@Example.com?subject=hi", "example.com"),
            ("ftp://files.example.net./", "files.example.net"),
            ("www.example.com/page", "www.example.com"),
            ("file:///C:/secret.docx", "file:"),
            ("tel:+15551234", "tel:"),
            ("", "No host"),
        ] {
            assert_eq!(hidden_content::host(uri), expected, "{uri}");
        }
    }

    #[test]
    fn producer_versions_are_the_first_dotted_number() {
        for (producer, version) in [
            ("Acrobat Distiller 9.0.1 (Windows)", Some("9.0.1")),
            ("LibreOffice 7.6", Some("7.6")),
            ("macOS Version 14.1 (Build 23B74) Quartz PDFContext", Some("14.1")),
            ("pdfTeX-1.40.25", None),
            ("Skia/PDF m120", None),
            ("Microsoft Word for Microsoft 365", None),
            ("Writer v2.3.", Some("2.3")),
        ] {
            assert_eq!(software_version(producer).as_deref(), version, "{producer}");
        }
    }

    #[test]
    fn a_batch_reports_each_file_writes_nothing_and_isolates_a_damaged_file() {
        let sandbox = Sandbox::new("batch");
        let fixtures = [
            ("information.pdf", pdf(information())),
            ("damaged.pdf", b"%PDF-1.7\nnot a pdf".to_vec()),
            ("xmp.pdf", pdf(conflicting_xmp())),
            ("restricted.pdf", pdf(restricted())),
            ("clean.pdf", pdf(CLEAN)),
            ("hidden.pdf", with_incremental_updates(pdf(hidden_content()), 1)),
        ];
        let paths = fixtures.iter().map(|(name, bytes)| sandbox.file(name, bytes)).collect::<Vec<_>>();
        let before = sandbox.names();

        let request = Arc::new(PrivacyRequest { paths: paths.clone(), password: None });
        let outcomes = BatchRunner::inspect(&RunEvents::silent(), "inspect_pdf_privacy", paths.clone(), &request, inspect_job);

        assert_eq!(sandbox.names(), before, "inspection must create no files");
        for ((_, bytes), path) in fixtures.iter().zip(&paths) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes, "{} must stay byte-identical", path.display());
        }
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        assert!(outcomes.iter().all(|outcome| outcome.output_paths().is_empty()), "reports name no output paths");
        assert!(matches!(outcomes[1].error().map(|error| &error.kind), Some(ErrorKind::Processing)), "{:?}", outcomes[1]);
        for index in [0, 2, 3, 4, 5] {
            assert!(matches!(outcomes[index].state, OutcomeState::Succeeded { .. }), "{:?}", outcomes[index]);
        }
        let reports = outcomes.iter().map(|outcome| serde_json::from_str::<serde_json::Value>(&outcome.detail).ok()).collect::<Vec<_>>();
        assert_eq!(reports[0].as_ref().unwrap()["documentInformation"]["author"], "Legal Ops, Acme Ltd");
        assert_eq!(reports[1], None);
        assert_eq!(reports[2].as_ref().unwrap()["xmp"]["conflicts"][0]["property"], "author");
        assert_eq!(reports[3].as_ref().unwrap()["encryption"]["openPassword"], false);
        assert_eq!(reports[4].as_ref().unwrap(), &serde_json::json!({
            "pageCount": 1,
            "documentInformation": {
                "title": null, "author": null, "subject": null, "keywords": null,
                "creator": null, "producer": null, "creationDate": null, "modificationDate": null,
            },
            "xmp": { "summary": null, "conflicts": [] },
            "encryption": null,
            "identifiers": { "documentId": null, "producer": null },
            "incrementalUpdates": 0,
            "embeddedFiles": [],
            "actions": [],
            "annotations": [],
            "filledFormFields": [],
            "optionalContent": { "layers": [], "hidden": [] },
            "links": [],
            "signatures": [],
            "leftovers": { "thumbnailPages": [], "pagePieces": [] },
            "unsupported": [],
        }));
        let hidden = reports[5].as_ref().unwrap();
        assert_eq!(hidden["incrementalUpdates"], 1);
        assert_eq!(hidden["embeddedFiles"][0], serde_json::json!({ "name": "rent-schedule.xlsx", "size": 49152, "mimeType": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "page": null }));
        assert_eq!(hidden["actions"][1], serde_json::json!({ "trigger": "beforePrint", "page": null, "name": null, "kinds": ["JavaScript", "Launch"] }));
        assert_eq!(hidden["optionalContent"]["hidden"][1], serde_json::json!({ "page": 1, "subtype": "Square", "reason": "offPage" }));
        assert_eq!(hidden["signatures"][0], serde_json::json!({ "field": "approver", "signer": "Acme Ltd", "signedAt": "2026-02-11T09:30:00+01:00" }));
        assert_eq!(hidden["leftovers"], serde_json::json!({ "thumbnailPages": [1], "pagePieces": [{ "page": 1, "applications": ["Illustrator"] }] }));
    }
}
