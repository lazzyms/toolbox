use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::{Aes128, Aes192, Aes256};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use cfb::CompoundFile;
use hmac::{Hmac, KeyInit, Mac};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader as XmlReader, XmlVersion};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use std::io::{Cursor, Read, Write};

const BLOCK_SIZE: usize = 16;
const KEY_DATA_SALT_SIZE: usize = 16;
const PACKAGE_KEY_SIZE: usize = 32;
const SEGMENT_SIZE: usize = 4096;
const SPIN_COUNT: u32 = 100_000;
/// ECMA-376 caps the Agile password spin count at ten million.
const MAX_SPIN_COUNT: u32 = 10_000_000;
const AGILE_ENCRYPTION_INFO_VERSION: [u8; 4] = [0x04, 0x00, 0x04, 0x00];
const AGILE_ENCRYPTION_INFO_FLAGS: u32 = 0x40;

const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const CFB_HEADER_DIFAT_ENTRIES: usize = 109;
const CFB_FREE_SECTOR: u32 = 0xFFFF_FFFF;
const CFB_END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const CFB_MAX_REGULAR_SECTOR: u32 = 0xFFFF_FFFA;
const CFB_MINI_SECTOR_LEN: u64 = 64;

const VERIFIER_HASH_INPUT_BLOCK_KEY: [u8; 8] = [
    0xFE, 0xA7, 0xD2, 0x76, 0x3B, 0x4B, 0x9E, 0x79,
];
const VERIFIER_HASH_VALUE_BLOCK_KEY: [u8; 8] = [
    0xD7, 0xAA, 0x0F, 0x6D, 0x30, 0x61, 0x34, 0x4E,
];
const ENCRYPTED_KEY_VALUE_BLOCK_KEY: [u8; 8] = [
    0x14, 0x6E, 0x0B, 0xE7, 0xAB, 0xAC, 0xD0, 0xD6,
];
const INTEGRITY_KEY_BLOCK_KEY: [u8; 8] = [
    0x5F, 0xB2, 0xAD, 0x01, 0x0C, 0xB9, 0xE1, 0xF6,
];
const INTEGRITY_VALUE_BLOCK_KEY: [u8; 8] = [
    0xA0, 0x67, 0x7F, 0x02, 0xB2, 0x2C, 0x84, 0x33,
];

/// The hash algorithms an Agile `keyData` or `encryptedKey` element can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgileHash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl AgileHash {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "SHA1" | "SHA-1" => Ok(Self::Sha1),
            "SHA256" | "SHA-256" => Ok(Self::Sha256),
            "SHA384" | "SHA-384" => Ok(Self::Sha384),
            "SHA512" | "SHA-512" => Ok(Self::Sha512),
            other => Err(format!("The Agile hash algorithm {other} is unsupported.")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha384 => "SHA384",
            Self::Sha512 => "SHA512",
        }
    }

    fn size(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut hasher = D::new();
            for part in parts {
                hasher.update(part);
            }
            hasher.finalize().to_vec()
        }
        match self {
            Self::Sha1 => run::<Sha1>(parts),
            Self::Sha256 => run::<Sha256>(parts),
            Self::Sha384 => run::<Sha384>(parts),
            Self::Sha512 => run::<Sha512>(parts),
        }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        fn run<M: Mac + KeyInit>(key: &[u8], data: &[u8]) -> Vec<u8> {
            let mut mac = <M as KeyInit>::new_from_slice(key).expect("HMAC accepts keys of any length");
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }
        match self {
            Self::Sha1 => run::<Hmac<Sha1>>(key, data),
            Self::Sha256 => run::<Hmac<Sha256>>(key, data),
            Self::Sha384 => run::<Hmac<Sha384>>(key, data),
            Self::Sha512 => run::<Hmac<Sha512>>(key, data),
        }
    }
}

/// The cipher settings of one Agile `keyData` or `encryptedKey` element. Agile
/// always uses AES-CBC with 16-byte blocks; the hash and key size vary by writer.
struct AgileCipher {
    hash: AgileHash,
    key_len: usize,
    salt: Vec<u8>,
}

impl AgileCipher {
    fn toolbox(salt: &[u8]) -> Self {
        Self { hash: AgileHash::Sha512, key_len: PACKAGE_KEY_SIZE, salt: salt.to_vec() }
    }

    /// The password hash after `spin_count` rounds of H(iterator ‖ previous hash).
    fn password_hash(&self, password: &str, spin_count: u32) -> Vec<u8> {
        let password_bytes: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut hash = self.hash.digest(&[&self.salt, &password_bytes]);
        for iterator in 0..spin_count {
            hash = self.hash.digest(&[&iterator.to_le_bytes(), &hash]);
        }
        hash
    }

    fn derive_key(&self, password_hash: &[u8], block_key: &[u8; 8]) -> Vec<u8> {
        fit(self.hash.digest(&[password_hash, block_key]), self.key_len)
    }

    fn xml_attributes(&self) -> String {
        format!(
            "saltSize=\"{}\" blockSize=\"{BLOCK_SIZE}\" keyBits=\"{}\" hashSize=\"{}\" cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"{}\" saltValue=\"{}\"",
            self.salt.len(),
            self.key_len * 8,
            self.hash.size(),
            self.hash.name(),
            BASE64.encode(&self.salt),
        )
    }

    fn salt_iv(&self) -> Vec<u8> {
        fit(self.salt.clone(), BLOCK_SIZE)
    }

    fn derive_iv(&self, block_key: &[u8]) -> Vec<u8> {
        fit(self.hash.digest(&[&self.salt, block_key]), BLOCK_SIZE)
    }
}

/// Truncate, or pad with 0x36, to the length the spec asks for.
fn fit(mut bytes: Vec<u8>, length: usize) -> Vec<u8> {
    bytes.resize(length, 0x36);
    bytes
}

/// Open a Compound File from bytes, tolerating one writer quirk the cfb crate
/// rejects: FAT entries past the end of the file, and MiniFAT entries past the
/// end of the mini stream, marked end-of-chain instead of free. No sector exists there, so freeing them changes no stream. The caller's
/// bytes are left as they are.
pub(crate) fn open_compound_file(raw: &[u8]) -> std::io::Result<CompoundFile<Cursor<Vec<u8>>>> {
    let mut bytes = raw.to_vec();
    free_fat_entries_past_end(&mut bytes);
    CompoundFile::open(Cursor::new(bytes))
}

fn free_fat_entries_past_end(bytes: &mut [u8]) {
    fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
    }
    if bytes.len() < 512 || bytes[..8] != CFB_MAGIC {
        return;
    }
    let sector_len = match u16::from_le_bytes([bytes[30], bytes[31]]) {
        9 => 512usize,
        12 => 4096usize,
        _ => return,
    };
    if bytes.len() < sector_len {
        return;
    }
    let num_sectors = (bytes.len() + sector_len - 1) / sector_len - 1;
    let entries_per_sector = sector_len / 4;
    let sector_offset = |sector: u32| (sector as usize + 1).checked_mul(sector_len);
    let Some(num_fat_sectors) = read_u32(bytes, 44) else { return };

    let mut fat_sectors = Vec::new();
    for index in 0..CFB_HEADER_DIFAT_ENTRIES {
        match read_u32(bytes, 76 + index * 4) {
            Some(sector) if sector <= CFB_MAX_REGULAR_SECTOR => fat_sectors.push(sector),
            _ => break,
        }
    }
    let mut difat_sector = read_u32(bytes, 68).unwrap_or(CFB_END_OF_CHAIN);
    let mut difat_sectors_left = read_u32(bytes, 72).unwrap_or(0) as usize;
    while difat_sectors_left > 0 && (difat_sector as usize) < num_sectors {
        let Some(base) = sector_offset(difat_sector) else { return };
        for index in 0..entries_per_sector - 1 {
            match read_u32(bytes, base + index * 4) {
                Some(sector) if sector <= CFB_MAX_REGULAR_SECTOR => fat_sectors.push(sector),
                _ => break,
            }
        }
        difat_sector = read_u32(bytes, base + (entries_per_sector - 1) * 4).unwrap_or(CFB_END_OF_CHAIN);
        difat_sectors_left -= 1;
    }
    fat_sectors.truncate(num_fat_sectors as usize);

    for (position, &fat_sector) in fat_sectors.iter().enumerate() {
        let first_entry = position * entries_per_sector;
        if first_entry + entries_per_sector <= num_sectors {
            continue;
        }
        let Some(base) = sector_offset(fat_sector) else { continue };
        if (fat_sector as usize) >= num_sectors {
            continue;
        }
        free_end_of_chain_entries(bytes, base, num_sectors.saturating_sub(first_entry), entries_per_sector);
    }

    // The same writers pad the MiniFAT the same way, past the last mini sector
    // the root entry's mini stream holds.
    let fat_entry = |bytes: &[u8], sector: u32| -> Option<u32> {
        let fat_sector = *fat_sectors.get(sector as usize / entries_per_sector)?;
        read_u32(bytes, sector_offset(fat_sector)? + (sector as usize % entries_per_sector) * 4)
    };
    let Some(root_entry) = read_u32(bytes, 48).and_then(sector_offset) else { return };
    let Some(root_stream_len) = bytes
        .get(root_entry + 120..root_entry + 128)
        .map(|length| u64::from_le_bytes(length.try_into().expect("eight bytes")))
    else {
        return;
    };
    let num_mini_sectors = usize::try_from(root_stream_len.div_ceil(CFB_MINI_SECTOR_LEN)).unwrap_or(usize::MAX);
    let mut minifat_sector = read_u32(bytes, 60).unwrap_or(CFB_END_OF_CHAIN);
    let mut position = 0usize;
    while minifat_sector <= CFB_MAX_REGULAR_SECTOR && (minifat_sector as usize) < num_sectors && position < num_sectors {
        let first_entry = position * entries_per_sector;
        if first_entry + entries_per_sector > num_mini_sectors {
            let Some(base) = sector_offset(minifat_sector) else { return };
            free_end_of_chain_entries(bytes, base, num_mini_sectors.saturating_sub(first_entry), entries_per_sector);
        }
        minifat_sector = fat_entry(bytes, minifat_sector).unwrap_or(CFB_END_OF_CHAIN);
        position += 1;
    }
}

/// Mark end-of-chain entries `from..to` of the allocation table sector at `base` free.
fn free_end_of_chain_entries(bytes: &mut [u8], base: usize, from: usize, to: usize) {
    for index in from..to {
        let offset = base + index * 4;
        if bytes.get(offset..offset + 4) == Some(CFB_END_OF_CHAIN.to_le_bytes().as_slice()) {
            bytes[offset..offset + 4].copy_from_slice(&CFB_FREE_SECTOR.to_le_bytes());
        }
    }
}

/// Encrypt an OOXML package using ECMA-376 Agile encryption.
///
/// The function only operates on byte slices. It does not read or write files,
/// and the returned bytes are a Compound File containing the streams and
/// DataSpaces hierarchy used by encrypted OOXML documents.
pub(crate) fn encrypt_ooxml(package: &[u8], password: &str) -> Result<Vec<u8>, String> {
    encrypt_ooxml_with(
        package,
        password,
        &AgileCipher::toolbox(&random_bytes(KEY_DATA_SALT_SIZE)?),
        &AgileCipher::toolbox(&random_bytes(KEY_DATA_SALT_SIZE)?),
        SPIN_COUNT,
    )
}

/// Encrypt with explicit cipher settings. Toolbox always locks with SHA-512 and
/// AES-256; tests use other settings to stand in for files other writers made.
fn encrypt_ooxml_with(
    package: &[u8],
    password: &str,
    password_cipher: &AgileCipher,
    key_data: &AgileCipher,
    spin_count: u32,
) -> Result<Vec<u8>, String> {
    let package_length = u32::try_from(package.len())
        .map_err(|_| "The OOXML package is too large to encrypt.".to_string())?;
    let package_key = random_bytes(key_data.key_len)?;
    let verifier_hash_input = random_bytes(password_cipher.salt.len())?;
    let integrity_salt = random_bytes(KEY_DATA_SALT_SIZE)?;

    let password_hash = password_cipher.password_hash(password, spin_count);
    let password_iv = password_cipher.salt_iv();
    let encrypted_verifier_hash_input = aes_cbc_encrypt(
        &password_cipher.derive_key(&password_hash, &VERIFIER_HASH_INPUT_BLOCK_KEY),
        &password_iv,
        &pad_to_block(verifier_hash_input.clone()),
    )?;
    let encrypted_verifier_hash_value = aes_cbc_encrypt(
        &password_cipher.derive_key(&password_hash, &VERIFIER_HASH_VALUE_BLOCK_KEY),
        &password_iv,
        &pad_to_block(password_cipher.hash.digest(&[&verifier_hash_input])),
    )?;
    let encrypted_key_value = aes_cbc_encrypt(
        &password_cipher.derive_key(&password_hash, &ENCRYPTED_KEY_VALUE_BLOCK_KEY),
        &password_iv,
        &pad_to_block(package_key.clone()),
    )?;

    let encrypted_package = encrypt_package(package, package_length, &package_key, key_data)?;
    let integrity_value = key_data.hash.hmac(&integrity_salt, &encrypted_package);
    let encrypted_integrity_key = aes_cbc_encrypt(
        &package_key,
        &key_data.derive_iv(&INTEGRITY_KEY_BLOCK_KEY),
        &integrity_salt,
    )?;
    let encrypted_integrity_value = aes_cbc_encrypt(
        &package_key,
        &key_data.derive_iv(&INTEGRITY_VALUE_BLOCK_KEY),
        &pad_to_block(integrity_value),
    )?;
    let encryption_info = encryption_info_xml(
        key_data,
        &encrypted_integrity_key,
        &encrypted_integrity_value,
        password_cipher,
        spin_count,
        &encrypted_verifier_hash_input,
        &encrypted_verifier_hash_value,
        &encrypted_key_value,
    );
    let mut encryption_info_stream = Vec::with_capacity(8 + encryption_info.len());
    encryption_info_stream.extend_from_slice(&AGILE_ENCRYPTION_INFO_VERSION);
    encryption_info_stream.extend_from_slice(&AGILE_ENCRYPTION_INFO_FLAGS.to_le_bytes());
    encryption_info_stream.extend_from_slice(encryption_info.as_bytes());

    let mut compound = CompoundFile::create(Cursor::new(Vec::new()))
        .map_err(|error| format!("Could not create the encrypted Office container: {error}"))?;
    compound
        .create_storage_all("/\x06DataSpaces")
        .map_err(|error| format!("Could not create the Office DataSpaces storage: {error}"))?;
    write_compound_stream(
        &mut compound,
        "/\x06DataSpaces/Version",
        &data_space_version_info(),
    )?;
    write_compound_stream(
        &mut compound,
        "/\x06DataSpaces/DataSpaceMap",
        &data_space_map(),
    )?;
    compound
        .create_storage_all("/\x06DataSpaces/DataSpaceInfo")
        .map_err(|error| format!("Could not create the Office DataSpaceInfo storage: {error}"))?;
    write_compound_stream(
        &mut compound,
        "/\x06DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
        &data_space_definition(),
    )?;
    compound
        .create_storage_all("/\x06DataSpaces/TransformInfo/StrongEncryptionTransform")
        .map_err(|error| format!("Could not create the Office TransformInfo storage: {error}"))?;
    write_compound_stream(
        &mut compound,
        "/\x06DataSpaces/TransformInfo/StrongEncryptionTransform/\x06Primary",
        &transform_info(),
    )?;
    write_compound_stream(&mut compound, "/EncryptionInfo", &encryption_info_stream)?;
    write_compound_stream(&mut compound, "/EncryptedPackage", &encrypted_package)?;
    compound
        .flush()
        .map_err(|error| format!("Could not finalize the encrypted Office container: {error}"))?;

    Ok(compound.into_inner().into_inner())
}

/// A package locked the way some other writers lock: SHA-1 and AES-128, a 32-byte
/// password salt, and FAT and MiniFAT entries past the end marked end-of-chain.
#[cfg(test)]
pub(crate) fn test_encrypt_like_other_writers(package: &[u8], password: &str) -> Vec<u8> {
    let password_cipher = AgileCipher { hash: AgileHash::Sha1, key_len: 16, salt: random_bytes(32).unwrap() };
    let key_data = AgileCipher { hash: AgileHash::Sha1, key_len: 16, salt: random_bytes(16).unwrap() };
    let mut encrypted = encrypt_ooxml_with(package, password, &password_cipher, &key_data, SPIN_COUNT).unwrap();
    let sector_len = 1usize << u16::from_le_bytes([encrypted[30], encrypted[31]]);
    let num_sectors = encrypted.len().div_ceil(sector_len) - 1;
    let first_fat_sector = u32::from_le_bytes(encrypted[76..80].try_into().unwrap()) as usize;
    let base = (first_fat_sector + 1) * sector_len;
    let mark = |encrypted: &mut [u8], base: usize, from: usize| {
        assert!(from < sector_len / 4, "the fixture must leave table entries past the end");
        for index in from..sector_len / 4 {
            let entry = &mut encrypted[base + index * 4..base + index * 4 + 4];
            if *entry == CFB_FREE_SECTOR.to_le_bytes() {
                entry.copy_from_slice(&CFB_END_OF_CHAIN.to_le_bytes());
            }
        }
    };
    mark(&mut encrypted, base, num_sectors);
    let root_entry = (u32::from_le_bytes(encrypted[48..52].try_into().unwrap()) as usize + 1) * sector_len;
    let root_stream_len = u64::from_le_bytes(encrypted[root_entry + 120..root_entry + 128].try_into().unwrap());
    let minifat_base = (u32::from_le_bytes(encrypted[60..64].try_into().unwrap()) as usize + 1) * sector_len;
    mark(&mut encrypted, minifat_base, root_stream_len.div_ceil(CFB_MINI_SECTOR_LEN) as usize);
    encrypted
}

pub(crate) fn is_agile_ooxml(encrypted: &[u8]) -> Result<bool, String> {
    let mut compound = open_compound_file(encrypted)
        .map_err(|error| format!("Could not open the encrypted Office container: {error}"))?;
    if !compound.is_stream("/EncryptionInfo") {
        return Ok(false);
    }

    let encryption_info = read_compound_stream(&mut compound, "/EncryptionInfo")?;
    if encryption_info.len() < 8 {
        return Err("The EncryptionInfo stream is incomplete.".to_string());
    }
    Ok(is_agile_encryption_info(&encryption_info))
}

#[derive(Debug)]
pub(crate) enum AgileDecryptError {
    WrongPassword,
    Unsupported(String),
    Damaged(String),
}

impl std::fmt::Display for AgileDecryptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => formatter.write_str("The Agile password verifier did not match."),
            Self::Unsupported(message) | Self::Damaged(message) => formatter.write_str(message),
        }
    }
}

pub(crate) fn decrypt_ooxml(encrypted: &[u8], password: &str) -> Result<Vec<u8>, AgileDecryptError> {
    let mut compound = open_compound_file(encrypted).map_err(|error| {
        AgileDecryptError::Damaged(format!("Could not open the encrypted Office container: {error}"))
    })?;
    let encryption_info =
        read_compound_stream(&mut compound, "/EncryptionInfo").map_err(AgileDecryptError::Damaged)?;
    let encrypted_package =
        read_compound_stream(&mut compound, "/EncryptedPackage").map_err(AgileDecryptError::Damaged)?;
    let parameters = parse_encryption_info(&encryption_info).map_err(AgileDecryptError::Unsupported)?;
    let package_key = verified_package_key(&parameters, password)
        .map_err(AgileDecryptError::Damaged)?
        .ok_or(AgileDecryptError::WrongPassword)?;
    decrypt_package(&parameters, &package_key, &encrypted_package).map_err(AgileDecryptError::Damaged)
}

/// Check the password against the verifier before touching the key, so a wrong
/// password is always reported as one, then return the package key.
fn verified_package_key(
    parameters: &ParsedAgileParameters,
    password: &str,
) -> Result<Option<Vec<u8>>, String> {
    let cipher = &parameters.password;
    let password_hash = cipher.password_hash(password, parameters.spin_count);
    let iv = cipher.salt_iv();

    let verifier_hash_input = aes_cbc_decrypt(
        &cipher.derive_key(&password_hash, &VERIFIER_HASH_INPUT_BLOCK_KEY),
        &iv,
        &parameters.encrypted_verifier_hash_input,
    )?;
    let verifier_hash_value = aes_cbc_decrypt(
        &cipher.derive_key(&password_hash, &VERIFIER_HASH_VALUE_BLOCK_KEY),
        &iv,
        &parameters.encrypted_verifier_hash_value,
    )?;
    let verifier_hash_input = verifier_hash_input
        .get(..cipher.salt.len())
        .ok_or_else(|| "The Agile verifier input is shorter than its salt.".to_string())?;
    let expected_verifier_hash = cipher.hash.digest(&[verifier_hash_input]);
    if verifier_hash_value.get(..expected_verifier_hash.len()) != Some(expected_verifier_hash.as_slice()) {
        return Ok(None);
    }

    let package_key = aes_cbc_decrypt(
        &cipher.derive_key(&password_hash, &ENCRYPTED_KEY_VALUE_BLOCK_KEY),
        &iv,
        &parameters.encrypted_key_value,
    )?;
    package_key
        .get(..parameters.key_data.key_len)
        .map(|key| Some(key.to_vec()))
        .ok_or_else(|| "The Agile package key has an invalid length.".to_string())
}

fn decrypt_package(
    parameters: &ParsedAgileParameters,
    package_key: &[u8],
    encrypted_package: &[u8],
) -> Result<Vec<u8>, String> {
    if encrypted_package.len() < 8 {
        return Err("The EncryptedPackage stream is missing its length header.".to_string());
    }
    let package_length = u32::from_le_bytes(
        encrypted_package[..4]
            .try_into()
            .map_err(|_| "The EncryptedPackage length header is invalid.".to_string())?,
    );
    if encrypted_package[4..8] != [0_u8; 4] {
        return Err("The EncryptedPackage stream has an invalid reserved header.".to_string());
    }
    let package_length = usize::try_from(package_length)
        .map_err(|_| "The OOXML package length is not supported on this platform.".to_string())?;
    let expected_encrypted_length = encrypted_package_length(package_length)?;
    if encrypted_package.len() != expected_encrypted_length {
        return Err("The EncryptedPackage stream has an invalid ciphertext length.".to_string());
    }

    let key_data = &parameters.key_data;
    let mut plaintext = Vec::with_capacity(package_length);
    let mut remaining = package_length;
    let mut encrypted_offset = 8usize;
    let mut segment_index = 0_u32;
    while remaining > 0 {
        let segment_length = remaining.min(SEGMENT_SIZE);
        let ciphertext_length = padded_segment_length(segment_length)?;
        let ciphertext_end = encrypted_offset
            .checked_add(ciphertext_length)
            .ok_or_else(|| "The EncryptedPackage stream length overflowed.".to_string())?;
        let ciphertext = encrypted_package
            .get(encrypted_offset..ciphertext_end)
            .ok_or_else(|| "The EncryptedPackage stream ended before its declared length.".to_string())?;

        let decrypted_segment =
            aes_cbc_decrypt(package_key, &key_data.derive_iv(&segment_index.to_le_bytes()), ciphertext)?;
        plaintext.extend_from_slice(&decrypted_segment[..segment_length]);

        encrypted_offset = ciphertext_end;
        remaining -= segment_length;
        segment_index = segment_index
            .checked_add(1)
            .ok_or_else(|| "The OOXML package has too many encryption segments.".to_string())?;
    }

    let integrity_key = aes_cbc_decrypt(
        package_key,
        &key_data.derive_iv(&INTEGRITY_KEY_BLOCK_KEY),
        &parameters.encrypted_hmac_key,
    )?;
    let integrity_value = aes_cbc_decrypt(
        package_key,
        &key_data.derive_iv(&INTEGRITY_VALUE_BLOCK_KEY),
        &parameters.encrypted_hmac_value,
    )?;
    // The HMAC key is hashSize bytes, padded to the block size when encrypted.
    // Packages Toolbox locked use a 16-byte key, which is kept whole.
    let integrity_key = &integrity_key[..integrity_key.len().min(key_data.hash.size())];
    let computed_integrity = key_data.hash.hmac(integrity_key, encrypted_package);
    if integrity_value.get(..computed_integrity.len()) != Some(computed_integrity.as_slice()) {
        return Err("The Agile package integrity HMAC did not match.".to_string());
    }

    Ok(plaintext)
}

pub(crate) fn verify_ooxml(
    encrypted: &[u8],
    password: &str,
    expected_package: &[u8],
) -> Result<(), String> {
    let plaintext = decrypt_ooxml(encrypted, password).map_err(|error| error.to_string())?;

    if plaintext != expected_package {
        return Err("The decrypted OOXML package did not match the expected package.".to_string());
    }
    Ok(())
}

fn is_agile_encryption_info(encryption_info: &[u8]) -> bool {
    let flags = AGILE_ENCRYPTION_INFO_FLAGS.to_le_bytes();
    encryption_info.get(..4) == Some(AGILE_ENCRYPTION_INFO_VERSION.as_slice())
        && encryption_info.get(4..8) == Some(flags.as_slice())
}

fn encrypted_package_length(package_length: usize) -> Result<usize, String> {
    let mut encrypted_length = 8usize;
    let mut remaining = package_length;
    let mut segment_index = 0_u32;
    while remaining > 0 {
        let segment_length = remaining.min(SEGMENT_SIZE);
        encrypted_length = encrypted_length
            .checked_add(padded_segment_length(segment_length)?)
            .ok_or_else(|| "The EncryptedPackage stream length overflowed.".to_string())?;
        remaining -= segment_length;
        segment_index = segment_index
            .checked_add(1)
            .ok_or_else(|| "The OOXML package has too many encryption segments.".to_string())?;
    }
    Ok(encrypted_length)
}

fn padded_segment_length(segment_length: usize) -> Result<usize, String> {
    let rounded_length = segment_length
        .checked_add(BLOCK_SIZE - 1)
        .ok_or_else(|| "The EncryptedPackage stream length overflowed.".to_string())?
        / BLOCK_SIZE;
    rounded_length
        .checked_mul(BLOCK_SIZE)
        .ok_or_else(|| "The EncryptedPackage stream length overflowed.".to_string())
}

fn random_bytes(length: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes).map_err(|error| format!("Could not generate encryption randomness: {error}"))?;
    Ok(bytes)
}

/// Zero-pad a value to the AES block size before it is encrypted.
fn pad_to_block(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.resize(bytes.len().div_ceil(BLOCK_SIZE) * BLOCK_SIZE, 0);
    bytes
}

fn aes_cbc_encrypt(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    fn run<C: BlockEncryptMut + KeyIvInit>(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut ciphertext = vec![0_u8; plaintext.len()];
        C::new_from_slices(key, iv)
            .map_err(|_| "The Agile AES key or IV has an invalid length.".to_string())?
            .encrypt_padded_b2b_mut::<NoPadding>(plaintext, &mut ciphertext)
            .map_err(|_| "Could not encrypt an Agile OOXML value.".to_string())?;
        Ok(ciphertext)
    }
    if plaintext.len() % BLOCK_SIZE != 0 {
        return Err("Agile AES plaintext must be block aligned.".to_string());
    }
    match key.len() {
        16 => run::<cbc::Encryptor<Aes128>>(key, iv, plaintext),
        24 => run::<cbc::Encryptor<Aes192>>(key, iv, plaintext),
        32 => run::<cbc::Encryptor<Aes256>>(key, iv, plaintext),
        _ => Err("The Agile AES key has an invalid length.".to_string()),
    }
}

fn aes_cbc_decrypt(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    fn run<C: BlockDecryptMut + KeyIvInit>(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut plaintext = vec![0_u8; ciphertext.len()];
        C::new_from_slices(key, iv)
            .map_err(|_| "The Agile AES key or IV has an invalid length.".to_string())?
            .decrypt_padded_b2b_mut::<NoPadding>(ciphertext, &mut plaintext)
            .map_err(|_| "Could not decrypt an Agile OOXML value.".to_string())?;
        Ok(plaintext)
    }
    if ciphertext.len() % BLOCK_SIZE != 0 {
        return Err("Agile AES ciphertext must be block aligned.".to_string());
    }
    match key.len() {
        16 => run::<cbc::Decryptor<Aes128>>(key, iv, ciphertext),
        24 => run::<cbc::Decryptor<Aes192>>(key, iv, ciphertext),
        32 => run::<cbc::Decryptor<Aes256>>(key, iv, ciphertext),
        _ => Err("The Agile AES key has an invalid length.".to_string()),
    }
}

fn encrypt_package(
    package: &[u8],
    package_length: u32,
    package_key: &[u8],
    key_data: &AgileCipher,
) -> Result<Vec<u8>, String> {
    let mut encrypted_package = Vec::new();
    encrypted_package.extend_from_slice(&package_length.to_le_bytes());
    encrypted_package.extend_from_slice(&0_u32.to_le_bytes());

    for (segment_index, segment) in package.chunks(SEGMENT_SIZE).enumerate() {
        let padded_length = (segment.len() + BLOCK_SIZE - 1) / BLOCK_SIZE * BLOCK_SIZE;
        let mut padded_segment = vec![0_u8; padded_length];
        padded_segment[..segment.len()].copy_from_slice(segment);

        let segment_index = u32::try_from(segment_index)
            .map_err(|_| "The OOXML package has too many encryption segments.".to_string())?;
        encrypted_package.extend_from_slice(&aes_cbc_encrypt(
            package_key,
            &key_data.derive_iv(&segment_index.to_le_bytes()),
            &padded_segment,
        )?);
    }

    Ok(encrypted_package)
}

fn encryption_info_xml(
    key_data: &AgileCipher,
    encrypted_integrity_key: &[u8],
    encrypted_integrity_value: &[u8],
    password_cipher: &AgileCipher,
    spin_count: u32,
    encrypted_verifier_hash_input: &[u8],
    encrypted_verifier_hash_value: &[u8],
    encrypted_key_value: &[u8],
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<encryption xmlns=\"http://schemas.microsoft.com/office/2006/encryption\" xmlns:p=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\n  <keyData {}/>\n  <dataIntegrity encryptedHmacKey=\"{}\" encryptedHmacValue=\"{}\"/>\n  <keyEncryptors>\n    <keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\n      <p:encryptedKey spinCount=\"{}\" {} encryptedVerifierHashInput=\"{}\" encryptedVerifierHashValue=\"{}\" encryptedKeyValue=\"{}\"/>\n    </keyEncryptor>\n  </keyEncryptors>\n</encryption>",
        key_data.xml_attributes(),
        BASE64.encode(encrypted_integrity_key),
        BASE64.encode(encrypted_integrity_value),
        spin_count,
        password_cipher.xml_attributes(),
        BASE64.encode(encrypted_verifier_hash_input),
        BASE64.encode(encrypted_verifier_hash_value),
        BASE64.encode(encrypted_key_value),
    )
}

fn write_unicode_lp_p4(buffer: &mut Vec<u8>, value: &str) {
    let code_units: Vec<u16> = value.encode_utf16().collect();
    let data_length = code_units
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .expect("DataSpaces Unicode string is too long");
    buffer.extend_from_slice(&data_length.to_le_bytes());
    for code_unit in code_units {
        buffer.extend_from_slice(&code_unit.to_le_bytes());
    }
    let padding = (4 - data_length % 4) % 4;
    for _ in 0..padding {
        buffer.push(0);
    }
}

fn write_version(buffer: &mut Vec<u8>) {
    buffer.extend_from_slice(&1_u16.to_le_bytes());
    buffer.extend_from_slice(&0_u16.to_le_bytes());
}

fn data_space_version_info() -> Vec<u8> {
    let mut stream = Vec::new();
    write_unicode_lp_p4(&mut stream, "Microsoft.Container.DataSpaces");
    write_version(&mut stream);
    write_version(&mut stream);
    write_version(&mut stream);
    stream
}

fn data_space_map() -> Vec<u8> {
    let mut entry = Vec::new();
    entry.extend_from_slice(&1_u32.to_le_bytes());
    entry.extend_from_slice(&0_u32.to_le_bytes());
    write_unicode_lp_p4(&mut entry, "EncryptedPackage");
    write_unicode_lp_p4(&mut entry, "StrongEncryptionDataSpace");

    let entry_length = u32::try_from(entry.len() + std::mem::size_of::<u32>())
        .expect("DataSpaceMap entry is too long");
    let mut stream = Vec::new();
    stream.extend_from_slice(&8_u32.to_le_bytes());
    stream.extend_from_slice(&1_u32.to_le_bytes());
    stream.extend_from_slice(&entry_length.to_le_bytes());
    stream.extend_from_slice(&entry);
    stream
}

fn data_space_definition() -> Vec<u8> {
    let mut stream = Vec::new();
    stream.extend_from_slice(&8_u32.to_le_bytes());
    stream.extend_from_slice(&1_u32.to_le_bytes());
    write_unicode_lp_p4(&mut stream, "StrongEncryptionTransform");
    stream
}

fn transform_info() -> Vec<u8> {
    let mut transform_id = Vec::new();
    write_unicode_lp_p4(
        &mut transform_id,
        "{FF9A3F03-56EF-4613-BDD5-5A41C1D07246}",
    );

    let transform_length = u32::try_from(
        std::mem::size_of::<u32>()
            + std::mem::size_of::<u32>()
            + transform_id.len(),
    )
    .expect("TransformInfoHeader is too long");
    let mut stream = Vec::new();
    stream.extend_from_slice(&transform_length.to_le_bytes());
    stream.extend_from_slice(&1_u32.to_le_bytes());
    stream.extend_from_slice(&transform_id);
    write_unicode_lp_p4(&mut stream, "Microsoft.Container.EncryptionTransform");
    write_version(&mut stream);
    write_version(&mut stream);
    write_version(&mut stream);

    // Agile encryption is described by EncryptionInfo; the transform name is
    // therefore the required null string, while these fields identify AES.
    stream.extend_from_slice(&0_u32.to_le_bytes());
    stream.extend_from_slice(&16_u32.to_le_bytes());
    stream.extend_from_slice(&0_u32.to_le_bytes());
    stream.extend_from_slice(&4_u32.to_le_bytes());
    stream
}

fn write_compound_stream(
    compound: &mut CompoundFile<Cursor<Vec<u8>>>,
    path: &str,
    contents: &[u8],
) -> Result<(), String> {
    let mut stream = compound
        .create_stream(path)
        .map_err(|error| format!("Could not create the {path} stream: {error}"))?;
    stream
        .write_all(contents)
        .map_err(|error| format!("Could not write the {path} stream: {error}"))
}

fn read_compound_stream(
    compound: &mut CompoundFile<Cursor<Vec<u8>>>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let mut stream = compound
        .open_stream(path)
        .map_err(|error| format!("Could not open the {path} stream: {error}"))?;
    let mut contents = Vec::new();
    stream
        .read_to_end(&mut contents)
        .map_err(|error| format!("Could not read the {path} stream: {error}"))?;
    Ok(contents)
}

#[derive(Default)]
struct AgileParameters {
    key_data: Option<AgileCipher>,
    encrypted_hmac_key: Option<Vec<u8>>,
    encrypted_hmac_value: Option<Vec<u8>>,
    spin_count: Option<u32>,
    password: Option<AgileCipher>,
    encrypted_verifier_hash_input: Option<Vec<u8>>,
    encrypted_verifier_hash_value: Option<Vec<u8>>,
    encrypted_key_value: Option<Vec<u8>>,
}

fn parse_encryption_info(encryption_info: &[u8]) -> Result<ParsedAgileParameters, String> {
    if encryption_info.len() <= 8 {
        return Err("The EncryptionInfo stream is incomplete.".to_string());
    }
    if !is_agile_encryption_info(encryption_info) {
        return Err("The EncryptionInfo stream is not an Agile encryption header.".to_string());
    }

    let xml = std::str::from_utf8(&encryption_info[8..])
        .map_err(|error| format!("The EncryptionInfo XML is not UTF-8: {error}"))?;
    let mut reader = XmlReader::from_reader(Cursor::new(xml.as_bytes()));
    reader.config_mut().check_end_names = true;
    let mut parameters = AgileParameters::default();
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut root_seen = false;
    let mut root_closed = false;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                if depth == 0 {
                    if root_seen || root_closed || element.local_name().as_ref() != b"encryption" {
                        return Err("The EncryptionInfo XML has an invalid root element.".to_string());
                    }
                    root_seen = true;
                }
                parse_agile_element(&element, &mut parameters)?;
                depth += 1;
            }
            Ok(Event::Empty(element)) => {
                if depth == 0 {
                    if root_seen || root_closed || element.local_name().as_ref() != b"encryption" {
                        return Err("The EncryptionInfo XML has an invalid root element.".to_string());
                    }
                    root_seen = true;
                    root_closed = true;
                } else {
                    parse_agile_element(&element, &mut parameters)?;
                }
            }
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return Err("The EncryptionInfo XML has an unexpected closing element.".to_string());
                }
                depth -= 1;
                if depth == 0 {
                    root_closed = true;
                }
            }
            Ok(Event::Text(text)) => {
                if text.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    return Err("The EncryptionInfo XML has unexpected text.".to_string());
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("The EncryptionInfo XML is malformed: {error}")),
        }
        buffer.clear();
    }

    if !root_seen || !root_closed || depth != 0 {
        return Err("The EncryptionInfo XML is incomplete.".to_string());
    }

    Ok(ParsedAgileParameters {
        key_data: parameters
            .key_data
            .ok_or_else(|| "The EncryptionInfo XML has no keyData element.".to_string())?,
        encrypted_hmac_key: parameters
            .encrypted_hmac_key
            .ok_or_else(|| "The EncryptionInfo XML has no encrypted HMAC key.".to_string())?,
        encrypted_hmac_value: parameters
            .encrypted_hmac_value
            .ok_or_else(|| "The EncryptionInfo XML has no encrypted HMAC value.".to_string())?,
        spin_count: parameters
            .spin_count
            .ok_or_else(|| "The EncryptionInfo XML has no password spin count.".to_string())?,
        password: parameters
            .password
            .ok_or_else(|| "The EncryptionInfo XML has no password key encryptor.".to_string())?,
        encrypted_verifier_hash_input: parameters.encrypted_verifier_hash_input.ok_or_else(|| {
            "The EncryptionInfo XML has no encrypted verifier input.".to_string()
        })?,
        encrypted_verifier_hash_value: parameters.encrypted_verifier_hash_value.ok_or_else(|| {
            "The EncryptionInfo XML has no encrypted verifier hash.".to_string()
        })?,
        encrypted_key_value: parameters
            .encrypted_key_value
            .ok_or_else(|| "The EncryptionInfo XML has no encrypted package key.".to_string())?,
    })
}

struct ParsedAgileParameters {
    key_data: AgileCipher,
    encrypted_hmac_key: Vec<u8>,
    encrypted_hmac_value: Vec<u8>,
    spin_count: u32,
    password: AgileCipher,
    encrypted_verifier_hash_input: Vec<u8>,
    encrypted_verifier_hash_value: Vec<u8>,
    encrypted_key_value: Vec<u8>,
}

fn parse_agile_element(
    element: &BytesStart<'_>,
    parameters: &mut AgileParameters,
) -> Result<(), String> {
    match element.local_name().as_ref() {
        b"keyData" => {
            if parameters.key_data.is_some() {
                return Err("The EncryptionInfo XML has duplicate keyData elements.".to_string());
            }
            parameters.key_data = Some(read_agile_cipher(element)?);
        }
        b"dataIntegrity" => {
            if parameters.encrypted_hmac_key.is_some() || parameters.encrypted_hmac_value.is_some() {
                return Err("The EncryptionInfo XML has duplicate dataIntegrity elements.".to_string());
            }
            parameters.encrypted_hmac_key = Some(read_base64_attribute(element, b"encryptedHmacKey")?);
            parameters.encrypted_hmac_value = Some(read_base64_attribute(element, b"encryptedHmacValue")?);
        }
        b"encryptedKey" => {
            if parameters.spin_count.is_some() {
                return Err("The EncryptionInfo XML has duplicate encryptedKey elements.".to_string());
            }
            let spin_count = read_xml_attribute(element, b"spinCount")?
                .parse::<u32>()
                .map_err(|error| format!("The Agile spin count is invalid: {error}"))?;
            if spin_count > MAX_SPIN_COUNT {
                return Err("The Agile encryption spin count is unsupported.".to_string());
            }
            parameters.spin_count = Some(spin_count);
            parameters.password = Some(read_agile_cipher(element)?);
            parameters.encrypted_verifier_hash_input =
                Some(read_base64_attribute(element, b"encryptedVerifierHashInput")?);
            parameters.encrypted_verifier_hash_value =
                Some(read_base64_attribute(element, b"encryptedVerifierHashValue")?);
            parameters.encrypted_key_value =
                Some(read_base64_attribute(element, b"encryptedKeyValue")?);
        }
        _ => {}
    }
    Ok(())
}

fn read_agile_cipher(element: &BytesStart<'_>) -> Result<AgileCipher, String> {
    if read_xml_attribute(element, b"cipherAlgorithm")? != "AES" {
        return Err("Only AES Agile encryption is supported.".to_string());
    }
    if read_xml_attribute(element, b"cipherChaining")? != "ChainingModeCBC" {
        return Err("Only CBC Agile encryption is supported.".to_string());
    }
    if read_xml_attribute(element, b"blockSize")? != BLOCK_SIZE.to_string() {
        return Err("The Agile block size is unsupported.".to_string());
    }
    let key_len = match read_xml_attribute(element, b"keyBits")?.as_str() {
        "128" => 16,
        "192" => 24,
        "256" => 32,
        _ => return Err("The Agile key size is unsupported.".to_string()),
    };
    let hash = AgileHash::parse(&read_xml_attribute(element, b"hashAlgorithm")?)?;
    let salt = read_base64_attribute(element, b"saltValue")?;
    if salt.is_empty() {
        return Err("The Agile salt is empty.".to_string());
    }
    Ok(AgileCipher { hash, key_len, salt })
}

fn read_xml_attribute(element: &BytesStart<'_>, name: &[u8]) -> Result<String, String> {
    let mut value = None;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| format!("The Agile XML attribute is invalid: {error}"))?;
        if attribute.key.as_ref() == name {
            if value.is_some() {
                return Err(format!(
                    "The Agile XML has duplicate {} attributes.",
                    String::from_utf8_lossy(name)
                ));
            }
            value = Some(
                attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|error| format!("The Agile XML attribute is invalid: {error}"))?
                    .into_owned(),
            );
        }
    }
    value.ok_or_else(|| {
        format!(
            "The Agile XML is missing its {} attribute.",
            String::from_utf8_lossy(name)
        )
    })
}

fn read_base64_attribute(element: &BytesStart<'_>, name: &[u8]) -> Result<Vec<u8>, String> {
    let encoded = read_xml_attribute(element, name)?;
    BASE64
        .decode(encoded.as_bytes())
        .map_err(|error| format!("The Agile {} attribute is not base64: {error}", String::from_utf8_lossy(name)))
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use cfb::CompoundFile;
    use super::{
        decrypt_ooxml, encrypt_ooxml, encrypt_ooxml_with, is_agile_ooxml, random_bytes,
        test_encrypt_like_other_writers, verify_ooxml, AgileCipher, AgileDecryptError, AgileHash,
    };
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn minimal_ooxml_package(kind: &str, include_large_entry: bool) -> Vec<u8> {
        let (main_part, content_type, main_xml): (&str, &str, &[u8]) = match kind {
            "docx" => (
                "word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#,
            ),
            "xlsx" => (
                "xl/workbook.xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/></workbook>"#,
            ),
            _ => panic!("unsupported OOXML fixture kind"),
        };

        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("[Content_Types].xml", SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/{main_part}" ContentType="{content_type}"/></Types>"#
        )
        .unwrap();
        archive
            .start_file("_rels/.rels", SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="/{main_part}"/></Relationships>"#
        )
        .unwrap();
        archive
            .start_file(main_part, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(main_xml).unwrap();

        if include_large_entry {
            let mut large_entry = vec![0_u8; 12_000];
            let mut state = 0x9E37_79B9_u32;
            for byte in &mut large_entry {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            archive
                .start_file("word/large.bin", SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&large_entry).unwrap();
        }

        archive.finish().unwrap().into_inner()
    }

    fn decode_encryption_info_attribute(encrypted: Vec<u8>, name: &str) -> Vec<u8> {
        let mut compound = CompoundFile::open(Cursor::new(encrypted)).unwrap();
        let mut stream = compound.open_stream("/EncryptionInfo").unwrap();
        let mut encryption_info = Vec::new();
        stream.read_to_end(&mut encryption_info).unwrap();
        assert!(encryption_info.len() > 8);

        let xml = std::str::from_utf8(&encryption_info[8..]).unwrap();
        let marker = format!("{name}=\"");
        let value = xml
            .split_once(&marker)
            .and_then(|(_, remainder)| remainder.split_once('\"'))
            .map(|(value, _)| value)
            .unwrap();
        BASE64.decode(value).unwrap()
    }

    fn tamper_encrypted_package(encrypted: Vec<u8>) -> Vec<u8> {
        let mut compound = CompoundFile::open(Cursor::new(encrypted)).unwrap();
        {
            let mut stream = compound.open_stream("/EncryptedPackage").unwrap();
            stream.seek(SeekFrom::Start(8)).unwrap();
            let mut byte = [0_u8; 1];
            stream.read_exact(&mut byte).unwrap();
            byte[0] ^= 1;
            stream.seek(SeekFrom::Start(8)).unwrap();
            stream.write_all(&byte).unwrap();
        }
        compound.flush().unwrap();
        compound.into_inner().into_inner()
    }

    fn read_u32(bytes: &[u8], offset: &mut usize) -> u32 {
        let end = *offset + 4;
        let value = u32::from_le_bytes(bytes[*offset..end].try_into().unwrap());
        *offset = end;
        value
    }

    fn read_unicode_lp_p4(bytes: &[u8], offset: &mut usize) -> String {
        let byte_length = read_u32(bytes, offset) as usize;
        assert_eq!(byte_length % 2, 0);
        let data_end = *offset + byte_length;
        let code_units = bytes[*offset..data_end]
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]));
        let value = String::from_utf16(code_units.collect::<Vec<_>>().as_slice()).unwrap();
        *offset = data_end;
        while *offset % 4 != 0 {
            assert_eq!(bytes[*offset], 0);
            *offset += 1;
        }
        value
    }

    fn read_test_stream(
        compound: &mut CompoundFile<Cursor<Vec<u8>>>,
        path: &str,
    ) -> Vec<u8> {
        let mut stream = compound.open_stream(path).unwrap();
        let mut contents = Vec::new();
        stream.read_to_end(&mut contents).unwrap();
        contents
    }

    #[test]
    fn writes_agile_data_spaces_hierarchy() {
        let encrypted = encrypt_ooxml(&minimal_ooxml_package("docx", false), "password").unwrap();
        let mut compound = CompoundFile::open(Cursor::new(encrypted)).unwrap();

        for path in [
            "/\x06DataSpaces",
            "/\x06DataSpaces/DataSpaceInfo",
            "/\x06DataSpaces/TransformInfo",
            "/\x06DataSpaces/TransformInfo/StrongEncryptionTransform",
        ] {
            assert!(compound.is_storage(path), "missing storage {path:?}");
        }
        for path in [
            "/\x06DataSpaces/Version",
            "/\x06DataSpaces/DataSpaceMap",
            "/\x06DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
            "/\x06DataSpaces/TransformInfo/StrongEncryptionTransform/\x06Primary",
        ] {
            assert!(compound.is_stream(path), "missing stream {path:?}");
        }

        let version = read_test_stream(&mut compound, "/\x06DataSpaces/Version");
        assert_eq!(version.len(), 76);
        let mut offset = 0;
        assert_eq!(
            read_unicode_lp_p4(&version, &mut offset),
            "Microsoft.Container.DataSpaces"
        );
        for _ in 0..3 {
            assert_eq!(read_u32(&version, &mut offset), 1);
        }
        assert_eq!(offset, version.len());

        let map = read_test_stream(&mut compound, "/\x06DataSpaces/DataSpaceMap");
        assert_eq!(map.len(), 112);
        let mut offset = 0;
        assert_eq!(read_u32(&map, &mut offset), 8);
        assert_eq!(read_u32(&map, &mut offset), 1);
        let entry_start = offset;
        let entry_length = read_u32(&map, &mut offset) as usize;
        assert_eq!(entry_length, 104);
        assert_eq!(read_u32(&map, &mut offset), 1);
        assert_eq!(read_u32(&map, &mut offset), 0);
        assert_eq!(read_unicode_lp_p4(&map, &mut offset), "EncryptedPackage");
        assert_eq!(
            read_unicode_lp_p4(&map, &mut offset),
            "StrongEncryptionDataSpace"
        );
        assert_eq!(offset, entry_start + entry_length);
        assert_eq!(offset, map.len());

        let definition = read_test_stream(
            &mut compound,
            "/\x06DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
        );
        assert_eq!(definition.len(), 64);
        let mut offset = 0;
        assert_eq!(read_u32(&definition, &mut offset), 8);
        assert_eq!(read_u32(&definition, &mut offset), 1);
        assert_eq!(
            read_unicode_lp_p4(&definition, &mut offset),
            "StrongEncryptionTransform"
        );
        assert_eq!(offset, definition.len());

        let primary = read_test_stream(
            &mut compound,
            "/\x06DataSpaces/TransformInfo/StrongEncryptionTransform/\x06Primary",
        );
        assert_eq!(primary.len(), 200);
        let mut offset = 0;
        assert_eq!(read_u32(&primary, &mut offset), 88);
        assert_eq!(read_u32(&primary, &mut offset), 1);
        assert_eq!(
            read_unicode_lp_p4(&primary, &mut offset),
            "{FF9A3F03-56EF-4613-BDD5-5A41C1D07246}"
        );
        assert_eq!(
            read_unicode_lp_p4(&primary, &mut offset),
            "Microsoft.Container.EncryptionTransform"
        );
        for _ in 0..3 {
            assert_eq!(read_u32(&primary, &mut offset), 1);
        }
        assert_eq!(read_u32(&primary, &mut offset), 0);
        assert_eq!(read_u32(&primary, &mut offset), 16);
        assert_eq!(read_u32(&primary, &mut offset), 0);
        assert_eq!(read_u32(&primary, &mut offset), 4);
        assert_eq!(offset, primary.len());
    }

    #[test]
    fn verifies_small_ooxml_package() {
        let package = minimal_ooxml_package("docx", false);
        assert!(package.len() < 4096);
        let encrypted = encrypt_ooxml(&package, "password").unwrap();

        assert_eq!(verify_ooxml(&encrypted, "password", &package), Ok(()));
    }

    #[test]
    fn verifies_package_with_exactly_one_segment() {
        let mut package = minimal_ooxml_package("docx", false);
        package.resize(4096, 0xA5);
        assert_eq!(package.len(), 4096);
        let encrypted = encrypt_ooxml(&package, "password").unwrap();

        assert_eq!(verify_ooxml(&encrypted, "password", &package), Ok(()));
    }

    #[test]
    fn verifies_package_over_one_segment() {
        let package = minimal_ooxml_package("xlsx", true);
        assert!(package.len() > 4096);
        let encrypted = encrypt_ooxml(&package, "password").unwrap();

        assert_eq!(verify_ooxml(&encrypted, "password", &package), Ok(()));
    }

    #[test]
    fn verifies_unicode_password() {
        let package = minimal_ooxml_package("docx", false);
        let encrypted = encrypt_ooxml(&package, "päss🔐文").unwrap();

        assert_eq!(verify_ooxml(&encrypted, "päss🔐文", &package), Ok(()));
    }

    #[test]
    fn rejects_wrong_password() {
        let package = minimal_ooxml_package("docx", false);
        let encrypted = encrypt_ooxml(&package, "correct password").unwrap();

        assert!(verify_ooxml(&encrypted, "wrong password", &package).is_err());
    }

    #[test]
    fn rejects_one_byte_ciphertext_tampering() {
        let package = minimal_ooxml_package("docx", false);
        let encrypted = encrypt_ooxml(&package, "password").unwrap();
        let tampered = tamper_encrypted_package(encrypted);

        assert!(verify_ooxml(&tampered, "password", &package).is_err());
    }

    #[test]
    fn writes_agile_integrity_fields_with_declared_sizes() {
        let encrypted = encrypt_ooxml(&minimal_ooxml_package("docx", false), "password").unwrap();

        let encrypted_hmac_key = decode_encryption_info_attribute(encrypted.clone(), "encryptedHmacKey");
        let encrypted_hmac_value = decode_encryption_info_attribute(encrypted, "encryptedHmacValue");

        assert_eq!(encrypted_hmac_key.len(), 16);
        assert_eq!(encrypted_hmac_value.len(), 64);
    }

    #[test]
    fn encrypts_minimal_docx_and_xlsx_packages() {
        for kind in ["docx", "xlsx"] {
            let package = minimal_ooxml_package(kind, true);
            let encrypted = encrypt_ooxml(&package, "correct horse battery staple").unwrap();
            let decrypted = office_crypto::decrypt_from_bytes(
                encrypted,
                "correct horse battery staple",
            )
            .unwrap();
            assert_eq!(decrypted, package, "{kind} package did not round-trip");
        }
    }

    #[test]
    fn wrong_password_never_matches_the_source_package() {
        let package = minimal_ooxml_package("docx", true);
        let encrypted = encrypt_ooxml(&package, "correct password").unwrap();
        let result = office_crypto::decrypt_from_bytes(encrypted, "wrong password");
        assert!(result.map_or(true, |decrypted| decrypted != package));
    }

    #[test]
    fn encrypts_large_package_with_unicode_password() {
        let package = minimal_ooxml_package("xlsx", true);
        assert!(package.len() > 4096);
        let encrypted = encrypt_ooxml(&package, "päss🔐文").unwrap();
        let decrypted = office_crypto::decrypt_from_bytes(encrypted, "päss🔐文").unwrap();
        assert_eq!(decrypted, package);
    }

    #[test]
    fn unlocks_packages_locked_with_other_hashes_key_sizes_and_salt_sizes() {
        let package = minimal_ooxml_package("xlsx", false);
        for (hash, key_len, salt_len) in [
            (AgileHash::Sha1, 16, 16),
            (AgileHash::Sha1, 16, 32),
            (AgileHash::Sha256, 24, 16),
            (AgileHash::Sha384, 32, 16),
            (AgileHash::Sha512, 32, 32),
        ] {
            let cipher = || AgileCipher { hash, key_len, salt: random_bytes(salt_len).unwrap() };
            let encrypted = encrypt_ooxml_with(&package, "secret", &cipher(), &cipher(), 1_000).unwrap();
            assert_eq!(decrypt_ooxml(&encrypted, "secret").unwrap(), package, "{hash:?} {key_len} {salt_len}");
            assert!(
                matches!(decrypt_ooxml(&encrypted, "wrong"), Err(AgileDecryptError::WrongPassword)),
                "{hash:?} {key_len} {salt_len}"
            );
        }
    }

    #[test]
    fn opens_compound_files_whose_fat_marks_entries_past_the_end_as_end_of_chain() {
        let package = minimal_ooxml_package("xlsx", false);
        let encrypted = test_encrypt_like_other_writers(&package, "secret");
        let before = encrypted.clone();

        assert!(CompoundFile::open(Cursor::new(encrypted.clone())).is_err());
        assert_eq!(is_agile_ooxml(&encrypted), Ok(true));
        assert_eq!(decrypt_ooxml(&encrypted, "secret").unwrap(), package);
        assert!(matches!(decrypt_ooxml(&encrypted, "wrong"), Err(AgileDecryptError::WrongPassword)));
        assert_eq!(encrypted, before);
    }
}
