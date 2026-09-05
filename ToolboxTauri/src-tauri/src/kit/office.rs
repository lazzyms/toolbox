use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use cfb::CompoundFile;
use md5::{Digest, Md5};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use sha1::Sha1;
use zip::ZipArchive;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

const OFFICE_EXTENSIONS: [&str; 6] = ["doc", "docx", "xls", "xlsx", "ppt", "pptx"];
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub struct OfficeProcessor;

impl OfficeProcessor {
    pub fn supports_extension(extension: &str) -> bool {
        OFFICE_EXTENSIONS
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(extension))
    }

    pub fn remove_password(
        input_path: PathBuf,
        password: &str,
        output_location: &OutputLocation,
    ) -> JobOutcome {
        if password.is_empty() {
            return JobOutcome::failure(
                input_path,
                ToolError::invalid_input("Enter the document password."),
            );
        }

        let Some(extension) = input_path.extension().and_then(|extension| extension.to_str()) else {
            return JobOutcome::failure(
                input_path,
                ToolError::invalid_input(
                    "Only Word, Excel, and PowerPoint files can be unlocked.",
                ),
            );
        };
        if !Self::supports_extension(extension) {
            return JobOutcome::failure(
                input_path,
                ToolError::invalid_input(
                    "Only Word, Excel, and PowerPoint files can be unlocked.",
                ),
            );
        }
        if !input_path.is_file() {
            return JobOutcome::failure(
                input_path,
                ToolError::invalid_input("Input file does not exist."),
            );
        }

        let raw = match std::fs::read(&input_path) {
            Ok(raw) => raw,
            Err(error) => {
                return JobOutcome::failure(
                    input_path,
                    ToolError::processing(format!("Could not read the Office document: {error}")),
                )
            }
        };
        let output_path = OutputNaming::get_destination(
            &input_path,
            output_location,
            "-unlocked",
            extension,
        );

        let mut output_created = false;
        match decrypt_office_bytes(&raw, extension, password)
            .and_then(|bytes| verify_unlocked_bytes(bytes, extension))
            .and_then(|bytes| {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output_path)
                    .map_err(|error| {
                        OfficeError::Processing(format!("Could not create the unlocked document: {error}"))
                    })?;
                output_created = true;
                output.write_all(&bytes).map_err(|error| {
                    OfficeError::Processing(format!("Could not write the unlocked document: {error}"))
                })
            }) {
            Ok(()) => JobOutcome {
                input_path,
                output_paths: vec![output_path],
                detail: "Office document unlocked and verified".to_string(),
                failure: None,
            },
            Err(error) => {
                if output_created {
                    let _ = std::fs::remove_file(&output_path);
                }
                JobOutcome::failure(input_path, error.into_tool_error())
            }
        }
    }
}

#[derive(Debug)]
enum OfficeError {
    InvalidInput(String),
    WrongPassword(String),
    Unsupported(String),
    Processing(String),
}

impl OfficeError {
    fn into_tool_error(self) -> ToolError {
        match self {
            Self::InvalidInput(message) | Self::WrongPassword(message) => {
                ToolError::invalid_input(message)
            }
            Self::Unsupported(message) => ToolError::unavailable(message),
            Self::Processing(message) => ToolError::processing(message),
        }
    }
}

fn invalid(message: impl Into<String>) -> OfficeError {
    OfficeError::InvalidInput(message.into())
}

fn decrypt_office_bytes(raw: &[u8], extension: &str, password: &str) -> Result<Vec<u8>, OfficeError> {
    let extension = extension.to_ascii_lowercase();
    if matches!(extension.as_str(), "docx" | "xlsx" | "pptx") && is_ooxml_package(raw, &extension) {
        return Ok(raw.to_vec());
    }
    if !raw.starts_with(&CFB_MAGIC) {
        return Err(invalid("The file is not a supported Office document."));
    }

    match extension.as_str() {
        "docx" | "xlsx" | "pptx" => office_crypto::decrypt_from_bytes(raw.to_vec(), password)
            .map_err(map_modern_decrypt_error),
        "doc" => decrypt_doc(raw, password),
        "xls" => decrypt_xls(raw, password),
        "ppt" => decrypt_ppt(raw, password),
        _ => Err(invalid("Only Word, Excel, and PowerPoint files can be unlocked.")),
    }
}

fn verify_unlocked_bytes(bytes: Vec<u8>, extension: &str) -> Result<Vec<u8>, OfficeError> {
    let extension = extension.to_ascii_lowercase();
    let valid = if matches!(extension.as_str(), "docx" | "xlsx" | "pptx") {
        is_ooxml_package(&bytes, &extension)
    } else {
        bytes.starts_with(&CFB_MAGIC)
    };
    if valid {
        Ok(bytes)
    } else {
        Err(OfficeError::Processing(
            "The unlocked Office document could not be verified.".to_string(),
        ))
    }
}

fn is_ooxml_package(data: &[u8], extension: &str) -> bool {
    let (main_part, main_part_root) = match extension {
        "docx" => ("word/document.xml", b"document".as_slice()),
        "xlsx" => ("xl/workbook.xml", b"workbook".as_slice()),
        "pptx" => ("ppt/presentation.xml", b"presentation".as_slice()),
        _ => return false,
    };
    let Ok(mut archive) = ZipArchive::new(Cursor::new(data)) else {
        return false;
    };
    let Some(content_types) = read_zip_entry(&mut archive, "[Content_Types].xml") else {
        return false;
    };
    let Some(main_part) = read_zip_entry(&mut archive, main_part) else {
        return false;
    };
    has_xml_root(&content_types, b"Types") && has_xml_root(&main_part, main_part_root)
}

fn read_zip_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut contents = Vec::new();
    entry.read_to_end(&mut contents).ok()?;
    Some(contents)
}

fn has_xml_root(data: &[u8], expected: &[u8]) -> bool {
    let mut reader = XmlReader::from_reader(Cursor::new(data));
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                if depth == 0 {
                    if root_seen || element.local_name().as_ref() != expected {
                        return false;
                    }
                    root_seen = true;
                }
                depth += 1;
            }
            Ok(Event::Empty(element)) => {
                if depth == 0 {
                    if root_seen || element.local_name().as_ref() != expected {
                        return false;
                    }
                    root_seen = true;
                }
            }
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Ok(Event::Eof) => return root_seen && depth == 0,
            Ok(_) => {}
            Err(_) => return false,
        }
        buffer.clear();
    }
}

fn map_modern_decrypt_error(error: office_crypto::DecryptError) -> OfficeError {
    match error {
        office_crypto::DecryptError::Unimplemented(message) => OfficeError::Unsupported(format!(
            "This Office encryption variant is not supported by the native adapter: {message}"
        )),
        office_crypto::DecryptError::InvalidHeader
        | office_crypto::DecryptError::InvalidStructure
        | office_crypto::DecryptError::NotEncrypted => OfficeError::WrongPassword(
            "Wrong password, malformed Office file, or unsupported Office encryption.".to_string(),
        ),
        office_crypto::DecryptError::IoError(error) => {
            OfficeError::Processing(format!("Could not decrypt the Office document: {error}"))
        }
        office_crypto::DecryptError::Unknown => {
            OfficeError::Processing("Could not decrypt the Office document.".to_string())
        }
    }
}

type CfbFile = CompoundFile<Cursor<Vec<u8>>>;

fn open_cfb(raw: &[u8]) -> Result<CfbFile, OfficeError> {
    CompoundFile::open(Cursor::new(raw.to_vec()))
        .map_err(|error| invalid(format!("The Office compound file is malformed: {error}")))
}

fn read_cfb_stream(file: &mut CfbFile, name: &str) -> Result<Vec<u8>, OfficeError> {
    let mut stream = file
        .open_stream(name)
        .map_err(|error| invalid(format!("The Office document is missing {name}: {error}")))?;
    let mut data = Vec::new();
    stream
        .read_to_end(&mut data)
        .map_err(|error| OfficeError::Processing(format!("Could not read Office stream {name}: {error}")))?;
    Ok(data)
}

fn read_optional_cfb_stream(file: &mut CfbFile, name: &str) -> Result<Option<Vec<u8>>, OfficeError> {
    match file.open_stream(name) {
        Ok(mut stream) => {
            let mut data = Vec::new();
            stream
                .read_to_end(&mut data)
                .map_err(|error| OfficeError::Processing(format!("Could not read Office stream {name}: {error}")))?;
            Ok(Some(data))
        }
        Err(_) => Ok(None),
    }
}

fn write_cfb_stream(file: &mut CfbFile, name: &str, data: &[u8]) -> Result<(), OfficeError> {
    let mut stream = file
        .open_stream(name)
        .map_err(|error| OfficeError::Processing(format!("Could not open Office stream {name}: {error}")))?;
    let old_len = stream
        .seek(SeekFrom::End(0))
        .map_err(|error| OfficeError::Processing(format!("Could not inspect Office stream {name}: {error}")))?;
    if old_len != data.len() as u64 {
        return Err(OfficeError::Processing(format!(
            "The native adapter changed the size of Office stream {name}."
        )));
    }
    stream
        .seek(SeekFrom::Start(0))
        .and_then(|_| stream.write_all(data))
        .map_err(|error| OfficeError::Processing(format!("Could not write Office stream {name}: {error}")))?;
    Ok(())
}

fn write_cfb_stream_resized(file: &mut CfbFile, name: &str, data: &[u8]) -> Result<(), OfficeError> {
    let mut stream = file
        .open_stream(name)
        .map_err(|error| OfficeError::Processing(format!("Could not open Office stream {name}: {error}")))?;
    stream
        .set_len(data.len() as u64)
        .and_then(|_| stream.seek(SeekFrom::Start(0)))
        .and_then(|_| stream.write_all(data))
        .map_err(|error| OfficeError::Processing(format!("Could not resize or write Office stream {name}: {error}")))?;
    Ok(())
}

fn finish_cfb(mut file: CfbFile) -> Result<Vec<u8>, OfficeError> {
    file.flush()
        .map_err(|error| OfficeError::Processing(format!("Could not finalize the Office document: {error}")))?;
    Ok(file.into_inner().into_inner())
}

#[derive(Clone, Copy)]
enum LegacyCipher {
    Rc4 { salt: [u8; 16] },
    CryptoApi { salt: [u8; 16], key_size: u32 },
}

impl LegacyCipher {
    fn key(self, password: &str, block: u32) -> Vec<u8> {
        match self {
            Self::Rc4 { salt } => make_rc4_key(password, &salt, block),
            Self::CryptoApi { salt, key_size } => make_cryptoapi_key(password, &salt, key_size, block),
        }
    }

    fn verify(self, password: &str, encrypted_verifier: &[u8], encrypted_hash: &[u8]) -> bool {
        let mut cipher = Rc4::new(&self.key(password, 0));
        let mut verifier = encrypted_verifier.to_vec();
        let mut hash = encrypted_hash.to_vec();
        cipher.apply(&mut verifier);
        cipher.apply(&mut hash);
        match self {
            Self::Rc4 { .. } => md5_digest(&verifier).as_slice() == hash,
            Self::CryptoApi { .. } => sha1_digest(&verifier).as_slice() == hash,
        }
    }
}

fn decrypt_rekeyed_with_password(
    data: &[u8],
    password: &str,
    cipher: LegacyCipher,
    block_size: usize,
    block_start: u32,
) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    for (index, chunk) in data.chunks(block_size.max(1)).enumerate() {
        let mut decrypted = chunk.to_vec();
        let mut rc4 = Rc4::new(&cipher.key(password, block_start + index as u32));
        rc4.apply(&mut decrypted);
        output.extend_from_slice(&decrypted);
    }
    output
}

fn utf16le_password(password: &str) -> Vec<u8> {
    password
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

fn md5_digest(data: &[u8]) -> [u8; 16] {
    let digest = Md5::digest(data);
    let mut output = [0u8; 16];
    output.copy_from_slice(&digest);
    output
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let digest = Sha1::digest(data);
    let mut output = [0u8; 20];
    output.copy_from_slice(&digest);
    output
}

fn make_rc4_key(password: &str, salt: &[u8; 16], block: u32) -> Vec<u8> {
    let password_bytes = utf16le_password(password);
    let h0 = md5_digest(&password_bytes);
    let mut intermediate = Vec::with_capacity(16 * 21);
    for _ in 0..16 {
        intermediate.extend_from_slice(&h0[..5]);
        intermediate.extend_from_slice(salt);
    }
    let h1 = md5_digest(&intermediate);
    let mut final_input = Vec::with_capacity(9);
    final_input.extend_from_slice(&h1[..5]);
    final_input.extend_from_slice(&block.to_le_bytes());
    md5_digest(&final_input).to_vec()
}

fn make_cryptoapi_key(password: &str, salt: &[u8; 16], key_size: u32, block: u32) -> Vec<u8> {
    let password_bytes = utf16le_password(password);
    let mut h0_input = Vec::with_capacity(salt.len() + password_bytes.len());
    h0_input.extend_from_slice(salt);
    h0_input.extend_from_slice(&password_bytes);
    let h0 = sha1_digest(&h0_input);
    let mut final_input = Vec::with_capacity(h0.len() + 4);
    final_input.extend_from_slice(&h0);
    final_input.extend_from_slice(&block.to_le_bytes());
    let hfinal = sha1_digest(&final_input);
    if key_size == 40 {
        let mut key = hfinal[..5].to_vec();
        key.resize(16, 0);
        key
    } else {
        hfinal[..((key_size as usize / 8).clamp(1, 16))].to_vec()
    }
}

struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    fn new(key: &[u8]) -> Self {
        let mut state = [0u8; 256];
        for (index, value) in state.iter_mut().enumerate() {
            *value = index as u8;
        }
        let mut j = 0u8;
        for index in 0..256 {
            j = j
                .wrapping_add(state[index])
                .wrapping_add(key[index % key.len().max(1)]);
            state.swap(index, j as usize);
        }
        Self { state, i: 0, j: 0 }
    }

    fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.state[self.i as usize]);
            self.state.swap(self.i as usize, self.j as usize);
            let index = self.state[self.i as usize].wrapping_add(self.state[self.j as usize]);
            *byte ^= self.state[index as usize];
        }
    }
}

struct CryptoApiInfo {
    salt: [u8; 16],
    key_size: u32,
    encrypted_verifier: [u8; 16],
    encrypted_hash: [u8; 20],
}

fn parse_cryptoapi_info(data: &[u8], offset: usize) -> Result<CryptoApiInfo, OfficeError> {
    let version = read_u32(data, offset)?;
    let major = (version & 0xffff) as u16;
    let minor = (version >> 16) as u16;
    if !matches!(major, 2 | 3 | 4) || minor != 2 {
        return Err(OfficeError::Unsupported(format!(
            "Office CryptoAPI version {major}.{minor}"
        )));
    }
    let header_size = read_u32(data, offset + 8)? as usize;
    let header_start = offset
        .checked_add(12)
        .ok_or_else(|| invalid("Invalid Office encryption header."))?;
    let header_end = header_start
        .checked_add(header_size)
        .ok_or_else(|| invalid("Invalid Office encryption header."))?;
    if header_end > data.len() || header_size < 20 {
        return Err(invalid("The Office encryption header is malformed."));
    }
    let key_size = read_u32(data, header_start + 16)?.max(40);
    let verifier = header_end;
    if verifier.checked_add(60).is_none_or(|end| end > data.len()) {
        return Err(invalid("The Office encryption verifier is malformed."));
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&data[verifier + 4..verifier + 20]);
    let mut encrypted_verifier = [0u8; 16];
    encrypted_verifier.copy_from_slice(&data[verifier + 20..verifier + 36]);
    let mut encrypted_hash = [0u8; 20];
    encrypted_hash.copy_from_slice(&data[verifier + 40..verifier + 60]);
    Ok(CryptoApiInfo { salt, key_size, encrypted_verifier, encrypted_hash })
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, OfficeError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("The Office file is truncated."))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, OfficeError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("The Office file is truncated."))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn decrypt_doc(raw: &[u8], password: &str) -> Result<Vec<u8>, OfficeError> {
    let mut file = open_cfb(raw)?;
    let word = read_cfb_stream(&mut file, "/WordDocument")?;
    if word.len() < 32 {
        return Err(invalid("The Word document is truncated."));
    }
    let flags = read_u16(&word, 10)?;
    if flags & 0x0100 == 0 {
        return Ok(raw.to_vec());
    }

    let table_name = if flags & 0x0200 != 0 { "/1Table" } else { "/0Table" };
    let mut table = read_cfb_stream(&mut file, table_name)?;
    let mut data_stream = read_optional_cfb_stream(&mut file, "/Data")?;
    let i_key = read_u32(&word, 14)?;

    if flags & 0x8000 != 0 {
        let chars = legacy_password_chars(password)?;
        if password_verifier_method2(&chars) != i_key {
            return Err(OfficeError::WrongPassword("The Word document password is incorrect.".to_string()));
        }
        decrypt_xor_method2(&mut table, &chars, 0);
        if let Some(data) = data_stream.as_mut() {
            decrypt_xor_method2(data, &chars, 0);
        }
        let mut word_decrypted = word.clone();
        if word_decrypted.len() > 68 {
            decrypt_xor_method2(&mut word_decrypted[68..], &chars, 68);
        }
        clear_word_fib(&mut word_decrypted);
        write_cfb_stream(&mut file, "/WordDocument", &word_decrypted)?;
        write_cfb_stream(&mut file, table_name, &table)?;
        if let Some(data) = data_stream {
            write_cfb_stream(&mut file, "/Data", &data)?;
        }
        return finish_cfb(file);
    }

    if word.len() < 68 || table.len() < 4 {
        return Err(invalid("The Word encryption metadata is malformed."));
    }
    let version = read_u32(&table, 0)?;
    let cipher = if version == 0x0001_0001 {
        if table.len() < 52 {
            return Err(invalid("The Word RC4 header is malformed."));
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&table[4..20]);
        let verifier = &table[20..36];
        let hash = &table[36..52];
        let cipher = LegacyCipher::Rc4 { salt };
        if !cipher.verify(password, verifier, hash) {
            return Err(OfficeError::WrongPassword("The Word document password is incorrect.".to_string()));
        }
        cipher
    } else {
        let info = parse_cryptoapi_info(&table, 0)?;
        let cipher = LegacyCipher::CryptoApi { salt: info.salt, key_size: info.key_size };
        if !cipher.verify(password, &info.encrypted_verifier, &info.encrypted_hash) {
            return Err(OfficeError::WrongPassword("The Word document password is incorrect.".to_string()));
        }
        cipher
    };

    let decrypted_word = decrypt_rekeyed_with_password(&word, password, cipher, 0x200, 0);
    table = decrypt_rekeyed_with_password(&table, password, cipher, 0x200, 0);
    if let Some(data) = data_stream.as_mut() {
        *data = decrypt_rekeyed_with_password(data, password, cipher, 0x200, 0);
    }
    let mut word_decrypted = word[..68].to_vec();
    word_decrypted.extend_from_slice(&decrypted_word[68..]);
    clear_word_fib(&mut word_decrypted);
    write_cfb_stream(&mut file, "/WordDocument", &word_decrypted)?;
    write_cfb_stream(&mut file, table_name, &table)?;
    if let Some(data) = data_stream {
        write_cfb_stream(&mut file, "/Data", &data)?;
    }
    finish_cfb(file)
}

fn clear_word_fib(word: &mut [u8]) {
    if word.len() >= 18 {
        let flags = u16::from_le_bytes([word[10], word[11]]) & !0x8100;
        word[10..12].copy_from_slice(&flags.to_le_bytes());
        word[14..18].fill(0);
    }
}

const PAD_ARRAY: [u8; 15] = [
    0xBB, 0xFF, 0xFF, 0xBA, 0xFF, 0xFF, 0xB9, 0x80, 0x00, 0xBE, 0x0F, 0x00, 0xBF, 0x0F,
    0x00,
];
const INITIAL_CODE: [u16; 15] = [
    0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F,
    0x84F9, 0x280C, 0xA96A, 0x4EC3,
];
const XOR_MATRIX: [u16; 105] = [
    0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09, 0x7B61, 0xF6C2, 0xFDA5, 0xEB6B,
    0xC6F7, 0x9DCF, 0x2BBF, 0x4563, 0x8AC6, 0x05AD, 0x0B5A, 0x16B4, 0x2D68, 0x5AD0, 0x0375,
    0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40, 0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D,
    0xAA7A, 0x44D5, 0x6F45, 0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A, 0xEB23, 0xC667,
    0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9, 0x47D3, 0x8FA6, 0x0F6D, 0x1EDA, 0x3DB4, 0x7B68,
    0xF6D0, 0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC, 0x45A0, 0x8B40, 0x06A1,
    0x0D42, 0x1A84, 0x3508, 0x6A10, 0xAA51, 0x4483, 0x8906, 0x022D, 0x045A, 0x08B4, 0x1168,
    0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C, 0x3730, 0x6E60, 0xDCC0, 0xA9A1,
    0x4363, 0x86C6, 0x1DAD, 0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC, 0x1021,
    0x2042, 0x4084, 0x8108, 0x1231, 0x2462, 0x48C4,
];

fn legacy_password_chars(password: &str) -> Result<Vec<u8>, OfficeError> {
    let chars: Vec<u8> = password.chars().map(|character| character as u32 as u8).collect();
    if chars.is_empty() || chars.len() > 15 {
        return Err(OfficeError::Unsupported(
            "Legacy Office XOR encryption supports passwords up to 15 characters.".to_string(),
        ));
    }
    Ok(chars)
}

fn password_verifier_method1(password: &[u8]) -> u16 {
    let mut verifier = 0u16;
    let mut bytes = Vec::with_capacity(password.len() + 1);
    bytes.push(password.len() as u8);
    bytes.extend_from_slice(password);
    for byte in bytes.into_iter().rev() {
        let intermediate1 = u16::from((verifier & 0x4000) != 0);
        let intermediate2 = (verifier << 1) & 0x7fff;
        verifier = (intermediate1 ^ intermediate2) ^ byte as u16;
    }
    verifier ^ 0xCE4B
}

fn xor_key_method1(password: &[u8]) -> u16 {
    let mut key = INITIAL_CODE[password.len() - 1];
    let mut element = 0x68usize;
    for &value in password.iter().rev() {
        let mut value = value;
        for _ in 0..7 {
            if value & 0x40 != 0 {
                key ^= XOR_MATRIX[element];
            }
            value <<= 1;
            element -= 1;
        }
    }
    key
}

fn ror8(value: u8, count: u32) -> u8 {
    value.rotate_right(count)
}

fn xor_ror(left: u8, right: u8) -> u8 {
    ror8(left ^ right, 1)
}

fn xor_array_method1(password: &[u8]) -> [u8; 16] {
    let key = xor_key_method1(password);
    let mut output = [0u8; 16];
    let mut index = password.len();
    if index % 2 == 1 {
        output[index] = xor_ror(PAD_ARRAY[0], (key >> 8) as u8);
        index -= 1;
        output[index] = xor_ror(password[password.len() - 1], key as u8);
    }
    while index > 0 {
        index -= 1;
        output[index] = xor_ror(password[index], (key >> 8) as u8);
        index -= 1;
        output[index] = xor_ror(password[index], key as u8);
    }
    index = 15;
    let mut pad_index = 15 - password.len();
    while pad_index > 0 {
        pad_index -= 1;
        output[index] = xor_ror(PAD_ARRAY[pad_index], (key >> 8) as u8);
        index -= 1;
        pad_index -= 1;
        output[index] = xor_ror(PAD_ARRAY[pad_index], key as u8);
        index -= 1;
    }
    output
}

fn password_verifier_method2(password: &[u8]) -> u32 {
    ((xor_key_method1(password) as u32) << 16) | password_verifier_method1(password) as u32
}

fn xor_array_method2(password: &[u8]) -> [u8; 16] {
    let key = xor_key_method1(password);
    let mut padded = [0u8; 16];
    for (index, value) in padded.iter_mut().enumerate() {
        *value = if index < password.len() {
            password[index]
        } else {
            PAD_ARRAY[(index - password.len()) % PAD_ARRAY.len()]
        };
    }
    for (index, value) in padded.iter_mut().enumerate() {
        let key_byte = if index % 2 == 0 { key as u8 } else { (key >> 8) as u8 };
        *value = xor_ror(*value, key_byte);
    }
    padded
}

fn decrypt_xor_method2(data: &mut [u8], password: &[u8], stream_offset: usize) {
    let array = xor_array_method2(password);
    for (index, value) in data.iter_mut().enumerate() {
        let transformed = *value ^ array[(stream_offset + index) % 16];
        if *value != 0 && transformed != 0 {
            *value = transformed;
        }
    }
}

#[derive(Clone, Copy)]
struct BiffRecord {
    id: u16,
    start: usize,
    size: usize,
}

impl BiffRecord {
    fn encoded_len(self) -> usize {
        4 + self.size
    }

    fn end(self) -> usize {
        self.start + self.encoded_len()
    }
}

fn parse_biff_records(data: &[u8]) -> Result<Vec<BiffRecord>, OfficeError> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        if data.len() - offset < 4 {
            return Err(invalid("The Excel workbook contains a truncated BIFF record."));
        }
        let id = read_u16(data, offset)?;
        let size = read_u16(data, offset + 2)? as usize;
        let end = offset
            .checked_add(4 + size)
            .ok_or_else(|| invalid("The Excel workbook contains an invalid BIFF record."))?;
        if end > data.len() {
            return Err(invalid("The Excel workbook contains a truncated BIFF record."));
        }
        records.push(BiffRecord { id, start: offset, size });
        offset = end;
    }
    Ok(records)
}

fn decrypt_xls(raw: &[u8], password: &str) -> Result<Vec<u8>, OfficeError> {
    let mut file = open_cfb(raw)?;
    let workbook_name = if file.open_stream("/Workbook").is_ok() { "/Workbook" } else { "/Book" };
    let workbook = read_cfb_stream(&mut file, workbook_name)?;
    let records = parse_biff_records(&workbook)?;
    let Some(file_pass) = records.iter().copied().find(|record| record.id == 0x002F) else {
        return Ok(raw.to_vec());
    };
    if file_pass.size < 2 {
        return Err(invalid("The Excel FilePass record is malformed."));
    }
    let info = &workbook[file_pass.start + 4..file_pass.start + 4 + file_pass.size];
    let encryption_type = read_u16(info, 0)?;
    let cipher = match encryption_type {
        0 => None,
        1 => {
            let version = read_u32(info, 2)?;
            if version == 0x0001_0001 {
                if info.len() < 54 {
                    return Err(invalid("The Excel RC4 header is malformed."));
                }
                let mut salt = [0u8; 16];
                salt.copy_from_slice(&info[6..22]);
                let candidate = LegacyCipher::Rc4 { salt };
                if !candidate.verify(password, &info[22..38], &info[38..54]) {
                    return Err(OfficeError::WrongPassword("The Excel workbook password is incorrect.".to_string()));
                }
                Some(candidate)
            } else {
                let api = parse_cryptoapi_info(info, 2)?;
                let candidate = LegacyCipher::CryptoApi { salt: api.salt, key_size: api.key_size };
                if !candidate.verify(password, &api.encrypted_verifier, &api.encrypted_hash) {
                    return Err(OfficeError::WrongPassword("The Excel workbook password is incorrect.".to_string()));
                }
                Some(candidate)
            }
        }
        _ => return Err(OfficeError::Unsupported("This Excel encryption type is not supported by the native adapter.".to_string())),
    };

    let mut plain: Vec<i16> = Vec::with_capacity(workbook.len());
    let mut encrypted = Vec::with_capacity(workbook.len());
    for record in records.iter().copied() {
        let header = &workbook[record.start..record.start + 4];
        let payload = &workbook[record.start + 4..record.start + 4 + record.size];
        if record.id == 0x002F {
            plain.extend_from_slice(&[0i16, 0, header[2] as i16, header[3] as i16]);
            plain.resize(plain.len() + record.size, 0);
            encrypted.resize(encrypted.len() + 4 + record.size, 0);
        } else if matches!(record.id, 0x0809 | 404 | 405 | 225 | 406 | 312) {
            plain.extend(header.iter().map(|byte| *byte as i16));
            plain.extend(payload.iter().map(|byte| *byte as i16));
            encrypted.resize(encrypted.len() + 4 + record.size, 0);
        } else if record.id == 0x0085 && record.size >= 4 {
            plain.extend(header.iter().map(|byte| *byte as i16));
            plain.extend(payload[..4].iter().map(|byte| *byte as i16));
            plain.resize(plain.len() + record.size - 4, -2);
            encrypted.extend_from_slice(&[0u8; 4]);
            encrypted.extend_from_slice(&[0u8; 4]);
            encrypted.extend_from_slice(&payload[4..]);
        } else {
            plain.extend(header.iter().map(|byte| *byte as i16));
            plain.resize(plain.len() + record.size, -1);
            encrypted.extend_from_slice(&[0u8; 4]);
            encrypted.extend_from_slice(payload);
        }
    }

    let decrypted = if let Some(cipher) = cipher {
        decrypt_rekeyed_with_password(&encrypted, password, cipher, 1024, 0)
    } else {
        let chars = legacy_password_chars(password)?;
        if password_verifier_method1(&chars) != read_u16(info, 4)? {
            return Err(OfficeError::WrongPassword("The Excel workbook password is incorrect.".to_string()));
        }
        decrypt_xls_xor(&encrypted, &plain, &chars)
    };
    let mut workbook_decrypted = Vec::with_capacity(workbook.len());
    for (index, marker) in plain.into_iter().enumerate() {
        workbook_decrypted.push(if marker >= 0 { marker as u8 } else { decrypted[index] });
    }
    let removed_start = file_pass.start;
    let removed_end = file_pass.end();
    let removed_len = file_pass.encoded_len();
    for record in records.iter().copied().filter(|record| record.id == 0x0085 && record.size >= 4) {
        let offset = read_u32(&workbook_decrypted, record.start + 4)? as usize;
        let relocated = if offset >= removed_end {
            offset.checked_sub(removed_len).ok_or_else(|| {
                invalid("The Excel BoundSheet8 offset cannot be relocated safely.")
            })?
        } else if offset >= removed_start {
            return Err(invalid(
                "The Excel BoundSheet8 offset points inside the removed FilePass record.",
            ));
        } else {
            offset
        };
        workbook_decrypted[record.start + 4..record.start + 8]
            .copy_from_slice(&(relocated as u32).to_le_bytes());
    }
    workbook_decrypted.drain(removed_start..removed_end);
    write_cfb_stream_resized(&mut file, workbook_name, &workbook_decrypted)?;
    finish_cfb(file)
}

fn decrypt_xls_xor(encrypted: &[u8], plain: &[i16], password: &[u8]) -> Vec<u8> {
    let array = xor_array_method1(password);
    let mut output = Vec::with_capacity(encrypted.len());
    let mut index = 0usize;
    while index < plain.len() {
        if plain[index] >= 0 {
            output.push(encrypted[index]);
            index += 1;
            continue;
        }
        let mut count = 1usize;
        while index + count < plain.len() && plain[index + count] < 0 {
            count += 1;
        }
        let array_index = (index + count + usize::from(plain[index] == -2) * 4) % 16;
        for item in 0..count {
            output.push(ror8(encrypted[index + item] ^ array[(array_index + item) % 16], 5));
        }
        index += count;
    }
    output
}

#[derive(Clone, Copy)]
struct PptRecord {
    offset: usize,
    record_type: u16,
    length: usize,
    end: usize,
}

fn ppt_record(data: &[u8], offset: usize) -> Result<PptRecord, OfficeError> {
    let record_type = read_u16(data, offset + 2)?;
    let length = read_u32(data, offset + 4)? as usize;
    let end = offset
        .checked_add(8 + length)
        .ok_or_else(|| invalid("The PowerPoint record is malformed."))?;
    if end > data.len() {
        return Err(invalid("The PowerPoint record is truncated."));
    }
    Ok(PptRecord { offset, record_type, length, end })
}

struct PptDirectory {
    record: PptRecord,
    entries: Vec<(u32, Vec<u32>)>,
}

struct PptEdit {
    offset: usize,
    record: PptRecord,
    last_edit: u32,
    directory_offset: u32,
    encryption_persist_id: Option<u32>,
}

fn parse_ppt_directory(data: &[u8], offset: usize) -> Result<PptDirectory, OfficeError> {
    let record = ppt_record(data, offset)?;
    if record.record_type != 0x1772 {
        return Err(invalid("The PowerPoint persist directory is malformed."));
    }
    let end = record.end;
    let mut cursor = offset + 8;
    let mut entries = Vec::new();
    while cursor < end {
        if end - cursor < 4 {
            return Err(invalid("The PowerPoint persist directory is truncated."));
        }
        let packed = read_u32(data, cursor)?;
        let persist_id = packed & 0x000F_FFFF;
        let count = (packed >> 20) as usize;
        cursor += 4;
        if count > (end - cursor) / 4 {
            return Err(invalid("The PowerPoint persist directory is malformed."));
        }
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            offsets.push(read_u32(data, cursor)?);
            cursor += 4;
        }
        entries.push((persist_id, offsets));
    }
    Ok(PptDirectory { record, entries })
}

fn parse_ppt_edit(data: &[u8], offset: usize) -> Result<PptEdit, OfficeError> {
    let record = ppt_record(data, offset)?;
    if record.record_type != 0x0FF5 || record.length < 0x1C {
        return Err(invalid("The PowerPoint UserEditAtom is malformed."));
    }
    let body = offset + 8;
    Ok(PptEdit {
        offset,
        record,
        last_edit: read_u32(data, body + 8)?,
        directory_offset: read_u32(data, body + 12)?,
        encryption_persist_id: if record.length >= 0x20 {
            Some(read_u32(data, body + 28)?)
        } else {
            None
        },
    })
}

fn decrypt_ppt(raw: &[u8], password: &str) -> Result<Vec<u8>, OfficeError> {
    let mut file = open_cfb(raw)?;
    let current_user = read_cfb_stream(&mut file, "/Current User")?;
    let mut document = read_cfb_stream(&mut file, "/PowerPoint Document")?;
    if current_user.len() < 20 || read_u16(&current_user, 2)? != 0x0FF6 {
        return Err(invalid("The PowerPoint Current User stream is malformed."));
    }
    let current_edit_offset = read_u32(&current_user, 16)? as usize;
    let current_edit = parse_ppt_edit(&document, current_edit_offset)?;
    if current_edit.encryption_persist_id.is_none() {
        return Ok(raw.to_vec());
    }

    let mut edits = Vec::new();
    let mut directories = Vec::new();
    let mut visited = HashSet::new();
    let mut edit_offset = current_edit_offset;
    while edit_offset != 0 && visited.insert(edit_offset) {
        let edit = parse_ppt_edit(&document, edit_offset)?;
        let directory = parse_ppt_directory(&document, edit.directory_offset as usize)?;
        directories.push(directory);
        edits.push(edit);
        edit_offset = edits.last().map(|edit| edit.last_edit).unwrap_or(0) as usize;
    }
    if edits.is_empty() {
        return Err(invalid("The PowerPoint edit chain is malformed."));
    }

    let mut persist_objects = HashMap::new();
    for directory in directories.iter().rev() {
        for (persist_id, offsets) in &directory.entries {
            for (index, offset) in offsets.iter().enumerate() {
                persist_objects.insert(persist_id + index as u32, *offset as usize);
            }
        }
    }
    let crypt_persist_id = current_edit.encryption_persist_id.unwrap();
    let crypt_offset = *persist_objects
        .get(&crypt_persist_id)
        .ok_or_else(|| invalid("The PowerPoint encryption session is missing."))?;
    let crypt_record = ppt_record(&document, crypt_offset)?;
    if crypt_record.record_type != 0x2F14 {
        return Err(invalid("The PowerPoint encryption session is malformed."));
    }
    let info = parse_cryptoapi_info(
        &document[crypt_offset + 8..crypt_offset + 8 + crypt_record.length],
        0,
    )?;
    let cipher = LegacyCipher::CryptoApi { salt: info.salt, key_size: info.key_size };
    if !cipher.verify(password, &info.encrypted_verifier, &info.encrypted_hash) {
        return Err(OfficeError::WrongPassword("The PowerPoint presentation password is incorrect.".to_string()));
    }
    if file.open_stream("/Pictures").is_ok() {
        return Err(OfficeError::Unsupported(
            "Encrypted PowerPoint presentations with a /Pictures stream are not supported by the native adapter.".to_string(),
        ));
    }

    let mut current_user_decrypted = current_user.clone();
    current_user_decrypted[12..16].copy_from_slice(&0xE391C05Fu32.to_le_bytes());
    if current_edit.record.length >= 0x20 {
        document[current_edit.offset + 4..current_edit.offset + 8].copy_from_slice(&0x1Cu32.to_le_bytes());
        document[current_edit.offset + 8 + 28..current_edit.offset + 8 + 32].fill(0);
    }
    remove_ppt_persist_entry(&mut document, &directories[0], crypt_persist_id, crypt_offset)?;
    let crypt_end = crypt_offset + 8 + crypt_record.length;
    document[crypt_offset..crypt_end].fill(0);

    let mut ordered: Vec<(u32, usize)> = persist_objects.into_iter().collect();
    ordered.sort_by_key(|(_, offset)| *offset);
    for (persist_id, offset) in &ordered {
        let record = ppt_record(&document, *offset);
        let Ok(record) = record else { continue };
        if record.record_type == 0x0FF5 || record.record_type == 0x1772 || *offset == crypt_offset {
            continue;
        }
        let end = record.end;
        if end <= *offset {
            continue;
        }
        let key_size = (info.key_size as usize).max(1);
        let block_size = key_size * ((end - *offset) / key_size + 1);
        let decrypted = decrypt_rekeyed_with_password(
            &document[*offset..end],
            password,
            cipher,
            block_size,
            *persist_id,
        );
        document[*offset..end].copy_from_slice(&decrypted);
    }
    write_cfb_stream(&mut file, "/Current User", &current_user_decrypted)?;
    write_cfb_stream(&mut file, "/PowerPoint Document", &document)?;
    finish_cfb(file)
}

fn remove_ppt_persist_entry(
    document: &mut [u8],
    directory: &PptDirectory,
    crypt_id: u32,
    crypt_offset: usize,
) -> Result<(), OfficeError> {
    let mut pairs = Vec::new();
    for (persist_id, offsets) in &directory.entries {
        for (index, offset) in offsets.iter().enumerate() {
            let id = persist_id + index as u32;
            if id != crypt_id || *offset as usize != crypt_offset {
                pairs.push((id, *offset));
            }
        }
    }
    pairs.sort_by_key(|(id, _)| *id);
    let mut body = Vec::new();
    let mut index = 0usize;
    while index < pairs.len() {
        let start_id = pairs[index].0;
        let mut offsets = vec![pairs[index].1];
        let mut next_id = start_id + 1;
        index += 1;
        while index < pairs.len() && pairs[index].0 == next_id {
            offsets.push(pairs[index].1);
            next_id += 1;
            index += 1;
        }
        body.extend_from_slice(&((start_id & 0x000F_FFFF) | ((offsets.len() as u32) << 20)).to_le_bytes());
        for offset in offsets {
            body.extend_from_slice(&offset.to_le_bytes());
        }
    }
    if body.len() > directory.record.length {
        return Err(OfficeError::Unsupported(
            "The PowerPoint persist directory cannot be compacted safely.".to_string(),
        ));
    }
    document[directory.record.offset + 4..directory.record.offset + 8]
        .copy_from_slice(&(body.len() as u32).to_le_bytes());
    let body_start = directory.record.offset + 8;
    document[body_start..body_start + directory.record.length].fill(0);
    document[body_start..body_start + body.len()].copy_from_slice(&body);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        decrypt_doc, decrypt_office_bytes, decrypt_ppt, decrypt_xls, make_cryptoapi_key,
        make_rc4_key, password_verifier_method1, password_verifier_method2, open_cfb,
        read_cfb_stream, xor_array_method1, xor_array_method2, OfficeError, OfficeProcessor,
        Rc4,
    };
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::ErrorKind;
    use cfb::CompoundFile;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn cfb_with_streams(streams: &[(&str, &[u8])]) -> Vec<u8> {
        let mut file = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        for (name, data) in streams {
            file.create_stream(*name).unwrap().write_all(data).unwrap();
        }
        file.flush().unwrap();
        file.into_inner().into_inner()
    }

    fn biff_record(id: u16, payload: &[u8]) -> Vec<u8> {
        let mut record = Vec::with_capacity(4 + payload.len());
        record.extend_from_slice(&id.to_le_bytes());
        record.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    fn minimal_ooxml_package(extension: &str) -> Vec<u8> {
        let (main_part, content_types, main_xml, relationship_type):
            (&str, &[u8], &[u8], &str) = match extension {
            "docx" => (
                "word/document.xml",
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
            ),
            "xlsx" => (
                "xl/workbook.xml",
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
                br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/></workbook>"#,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
            ),
            "pptx" => (
                "ppt/presentation.xml",
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#,
                br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
            ),
            _ => panic!("unsupported OOXML extension"),
        };
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("[Content_Types].xml", SimpleFileOptions::default()).unwrap();
        archive.write_all(content_types).unwrap();
        archive.start_file("_rels/.rels", SimpleFileOptions::default()).unwrap();
        write!(
            archive,
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{relationship_type}\" Target=\"/{main_part}\"/></Relationships>"
        )
        .unwrap();
        archive.start_file(main_part, SimpleFileOptions::default()).unwrap();
        archive.write_all(main_xml).unwrap();
        archive.finish().unwrap().into_inner()
    }

    fn malformed_ooxml_package() -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in [
            ("[Content_Types].xml", b"<Types>".as_slice()),
            ("word/document.xml", b"<w:document>".as_slice()),
        ] {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(contents).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    fn ppt_record(record_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut record = Vec::with_capacity(8 + payload.len());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&record_type.to_le_bytes());
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    fn cryptoapi_info(password: &str, salt: [u8; 16], key_size: u32) -> Vec<u8> {
        let mut info = vec![0u8; 12 + 20 + 60];
        info[0..4].copy_from_slice(&0x0002_0004u32.to_le_bytes());
        info[8..12].copy_from_slice(&20u32.to_le_bytes());
        info[28..32].copy_from_slice(&key_size.to_le_bytes());
        let verifier_offset = 32;
        info[verifier_offset + 4..verifier_offset + 20].copy_from_slice(&salt);
        let verifier = [0x21u8; 16];
        let hash = super::sha1_digest(&verifier);
        let mut encrypted_verifier = verifier;
        let mut encrypted_hash = hash;
        let key = make_cryptoapi_key(password, &salt, key_size, 0);
        let mut rc4 = Rc4::new(&key);
        rc4.apply(&mut encrypted_verifier);
        rc4.apply(&mut encrypted_hash);
        info[verifier_offset + 20..verifier_offset + 36].copy_from_slice(&encrypted_verifier);
        info[verifier_offset + 40..verifier_offset + 60].copy_from_slice(&encrypted_hash);
        info
    }

    fn ppt_fixture(with_content: bool, with_pictures: bool) -> Vec<u8> {
        let password = "secret";
        let salt = [0x31u8; 16];
        let info = cryptoapi_info(password, salt, 128);
        let crypt = ppt_record(0x2F14, &info);
        assert_eq!(crypt.len(), 100);

        let content_offset = 300usize;
        let content_plain = ppt_record(0x03E8, &[0x41, 0x42, 0x43, 0x44]);
        let mut document = vec![0u8; 320];
        document[..crypt.len()].copy_from_slice(&crypt);
        if with_content {
            let mut content_encrypted = content_plain.clone();
            let key = make_cryptoapi_key(password, &salt, 128, 2);
            Rc4::new(&key).apply(&mut content_encrypted[8..]);
            document[content_offset..content_offset + content_encrypted.len()]
                .copy_from_slice(&content_encrypted);
            document[content_offset + content_encrypted.len()..]
                .copy_from_slice(&[0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7]);
        }

        let mut directory_body = Vec::new();
        let entry_count = u32::from(with_content) + 1;
        directory_body.extend_from_slice(&(1u32 | (entry_count << 20)).to_le_bytes());
        directory_body.extend_from_slice(&0u32.to_le_bytes());
        if with_content {
            directory_body.extend_from_slice(&(content_offset as u32).to_le_bytes());
        }
        document[100..100 + 8 + directory_body.len()]
            .copy_from_slice(&ppt_record(0x1772, &directory_body));

        let mut edit_body = vec![0u8; 0x20];
        edit_body[12..16].copy_from_slice(&100u32.to_le_bytes());
        edit_body[28..32].copy_from_slice(&1u32.to_le_bytes());
        document[200..240].copy_from_slice(&ppt_record(0x0FF5, &edit_body));

        let mut current_user = vec![0u8; 20];
        current_user[2..4].copy_from_slice(&0x0FF6u16.to_le_bytes());
        current_user[16..20].copy_from_slice(&200u32.to_le_bytes());

        if with_pictures {
            cfb_with_streams(&[
                ("/Current User", &current_user),
                ("/PowerPoint Document", &document),
                ("/Pictures", &[0xAA, 0xBB, 0xCC]),
            ])
        } else {
            cfb_with_streams(&[
                ("/Current User", &current_user),
                ("/PowerPoint Document", &document),
            ])
        }
    }

    #[test]
    fn recognizes_modern_and_legacy_microsoft_office_extensions() {
        for extension in ["doc", "docx", "xls", "xlsx", "ppt", "pptx"] {
            assert!(OfficeProcessor::supports_extension(extension), "{extension} should be supported");
        }
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert!(OfficeProcessor::supports_extension("DOCX"));
        assert!(OfficeProcessor::supports_extension("XlS"));
        assert!(OfficeProcessor::supports_extension("PpTx"));
    }

    #[test]
    fn unsupported_input_is_rejected_without_creating_output() {
        let input = PathBuf::from("presentation.odp");
        let outcome = OfficeProcessor::remove_password(input.clone(), "secret", &OutputLocation::AlongsideInput);

        assert!(matches!(outcome.failure.as_ref().map(|error| &error.kind), Some(ErrorKind::InvalidInput)));
        assert!(outcome.output_paths.is_empty());
        assert_eq!(outcome.input_path, input);
    }

    #[test]
    fn empty_password_is_rejected_before_processing() {
        let input = PathBuf::from("report.docx");
        let outcome = OfficeProcessor::remove_password(input, "", &OutputLocation::AlongsideInput);

        assert!(matches!(outcome.failure.as_ref().map(|error| &error.kind), Some(ErrorKind::InvalidInput)));
        assert!(outcome.output_paths.is_empty());
    }

    #[test]
    fn plain_modern_office_package_is_handled_without_an_external_runtime() {
        for extension in ["docx", "xlsx", "pptx"] {
            let package = minimal_ooxml_package(extension);
            let decrypted = decrypt_office_bytes(&package, extension, "secret").unwrap();
            assert_eq!(decrypted, package);
        }
    }

    #[test]
    fn malformed_modern_office_package_is_not_verified() {
        let malformed = b"PK\x03\x04garbage";
        let error = decrypt_office_bytes(malformed, "docx", "secret").unwrap_err();
        assert!(matches!(error, OfficeError::InvalidInput(_)));
        let error = super::verify_unlocked_bytes(malformed.to_vec(), "docx").unwrap_err();
        assert!(matches!(error, OfficeError::Processing(_)));
        let malformed_zip = malformed_ooxml_package();
        let error = decrypt_office_bytes(&malformed_zip, "docx", "secret").unwrap_err();
        assert!(matches!(error, OfficeError::InvalidInput(_)));
    }

    #[test]
    fn word_xor_decryption_uses_the_word_document_offset() {
        let password = b"abcde";
        let mut word = vec![0u8; 100];
        word[10..12].copy_from_slice(&0x8100u16.to_le_bytes());
        word[14..18].copy_from_slice(&password_verifier_method2(password).to_le_bytes());
        let array = xor_array_method2(password);
        for (index, byte) in word[68..].iter_mut().enumerate() {
            let plain = 0xA0u8.wrapping_add(index as u8);
            *byte = plain ^ array[(68 + index) % 16];
        }
        let raw = cfb_with_streams(&[
            ("/WordDocument", &word),
            ("/0Table", &[0x11; 16]),
        ]);
        let mut raw_file = open_cfb(&raw).unwrap();
        let raw_word = read_cfb_stream(&mut raw_file, "/WordDocument").unwrap();
        assert_eq!(u32::from_le_bytes(raw_word[14..18].try_into().unwrap()), password_verifier_method2(password));
        assert_eq!(password_verifier_method2(&super::legacy_password_chars("abcde").unwrap()), password_verifier_method2(password));

        let unlocked = decrypt_doc(&raw, "abcde").unwrap();
        let mut file = open_cfb(&unlocked).unwrap();
        let word = read_cfb_stream(&mut file, "/WordDocument").unwrap();
        assert_eq!(word[68..], (0..32).map(|index| 0xA0u8.wrapping_add(index)).collect::<Vec<_>>());
    }

    #[test]
    fn excel_unlock_removes_the_file_pass_record_and_resizes_the_stream() {
        let password = b"abcde";
        let mut file_pass_payload = vec![0u8; 6];
        file_pass_payload[4..6].copy_from_slice(&password_verifier_method1(password).to_le_bytes());
        let bof = biff_record(0x0809, &[1, 2, 3, 4]);
        let mut workbook = biff_record(0x002F, &file_pass_payload);
        workbook.extend_from_slice(&bof);
        let padding = biff_record(0x0809, &[0x5A; 4555]);
        workbook.extend_from_slice(&padding);
        let mut bound_sheet = biff_record(0x0085, &[0, 0, 0, 0]);
        let sheet_bof_offset = workbook.len() + bound_sheet.len();
        bound_sheet[4..8].copy_from_slice(&(sheet_bof_offset as u32).to_le_bytes());
        let bound_sheet_len = bound_sheet.len();
        workbook.extend_from_slice(&bound_sheet);
        let sheet_bof = biff_record(0x0809, &[5, 6, 7, 8]);
        let sheet_bof_len = sheet_bof.len();
        workbook.extend_from_slice(&sheet_bof);
        let data = biff_record(0x0203, &[0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);
        let data_start = workbook.len() + 4;
        workbook.extend_from_slice(&data);
        workbook.extend_from_slice(&biff_record(0x000A, &[]));
        let array = xor_array_method1(password);
        let payload_len = 8usize;
        for index in 0..payload_len {
            let plain = workbook[data_start + index];
            let array_index = (data_start + payload_len + index) % 16;
            workbook[data_start + index] = plain.rotate_left(5) ^ array[array_index];
        }
        let raw = cfb_with_streams(&[("/Workbook", &workbook)]);

        let unlocked = decrypt_xls(&raw, "abcde").unwrap();
        let mut file = open_cfb(&unlocked).unwrap();
        let workbook = read_cfb_stream(&mut file, "/Workbook").unwrap();
        let records = super::parse_biff_records(&workbook).unwrap();
        assert!(records.iter().all(|record| record.id != 0x002F));
        assert_eq!(workbook.len(), bof.len() + padding.len() + bound_sheet_len + sheet_bof_len + data.len() + 4);
        let bound_sheet = records.iter().find(|record| record.id == 0x0085).unwrap();
        let relocated_offset = u32::from_le_bytes(workbook[bound_sheet.start + 4..bound_sheet.start + 8].try_into().unwrap()) as usize;
        assert_eq!(relocated_offset, sheet_bof_offset - (4 + file_pass_payload.len()));
        assert!(records.iter().any(|record| record.id == 0x0809 && record.start == relocated_offset));
        let data = records.iter().find(|record| record.id == 0x0203).unwrap();
        assert_eq!(&workbook[data.start + 4..data.end()], &[0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);
    }

    #[test]
    fn powerpoint_final_persist_object_does_not_decrypt_the_tail() {
        let raw = ppt_fixture(true, false);
        let unlocked = decrypt_ppt(&raw, "secret").unwrap();
        let mut file = open_cfb(&unlocked).unwrap();
        let document = read_cfb_stream(&mut file, "/PowerPoint Document").unwrap();
        assert_eq!(&document[312..320], &[0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7]);
    }

    #[test]
    fn powerpoint_with_pictures_is_rejected_until_picture_decryption_is_supported() {
        let error = decrypt_ppt(&ppt_fixture(false, true), "secret").unwrap_err();
        assert!(matches!(error, OfficeError::Unsupported(_)));
    }

    #[test]
    fn powerpoint_picture_rejection_preserves_input_and_creates_no_output() {
        let directory = std::env::temp_dir().join(format!(
            "toolbox-office-ppt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("presentation.ppt");
        std::fs::write(&input, ppt_fixture(false, true)).unwrap();
        let original = std::fs::read(&input).unwrap();

        let outcome = OfficeProcessor::remove_password(
            input.clone(),
            "secret",
            &OutputLocation::AlongsideInput,
        );

        assert!(matches!(
            outcome.failure.as_ref().map(|error| &error.kind),
            Some(ErrorKind::Unavailable)
        ));
        assert!(outcome.output_paths.is_empty());
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(!directory.join("presentation-unlocked.ppt").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsupported_encryption_is_reported_by_the_native_adapter() {
        let error = decrypt_office_bytes(b"not-an-office-file", "xlsx", "secret").unwrap_err();
        assert!(matches!(error, OfficeError::InvalidInput(_)));
    }

    #[test]
    fn native_rc4_key_matches_the_office_binary_vector() {
        let salt = [
            0xe8, 0x77, 0x2c, 0x1d, 0x91, 0xc5, 0x6a, 0x37, 0x96, 0x47, 0x61, 0xb2, 0x80, 0x18,
            0x32, 0x17,
        ];
        assert_eq!(
            make_rc4_key("password1", &salt, 0),
            vec![0x20, 0xbf, 0x32, 0xdd, 0xf5, 0x40, 0x85, 0x8c, 0x51, 0x37, 0x44, 0xaf, 0x0f, 0x24, 0xe0, 0x3c]
        );
    }

    #[test]
    fn native_xor_password_verifier_matches_the_office_binary_vector() {
        assert_eq!(password_verifier_method1(b"VelvetSweatshop"), 0x9a0a);
    }
}
