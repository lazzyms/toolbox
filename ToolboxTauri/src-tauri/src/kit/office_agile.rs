use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use cfb::CompoundFile;
use hmac::{Hmac, KeyInit, Mac};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader as XmlReader, XmlVersion};
use sha2::{Digest, Sha512};
use std::io::{Cursor, Read, Write};

const BLOCK_SIZE: usize = 16;
const HASH_SIZE: usize = 64;
const KEY_DATA_SALT_SIZE: usize = 16;
const PACKAGE_KEY_SIZE: usize = 32;
const SEGMENT_SIZE: usize = 4096;
const SPIN_COUNT: u32 = 100_000;
const AGILE_ENCRYPTION_INFO_VERSION: [u8; 4] = [0x04, 0x00, 0x04, 0x00];
const AGILE_ENCRYPTION_INFO_FLAGS: u32 = 0x40;

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

type Aes256CbcEncryptor = cbc::Encryptor<Aes256>;
type Aes256CbcDecryptor = cbc::Decryptor<Aes256>;

/// Encrypt an OOXML package using ECMA-376 Agile encryption.
///
/// The function only operates on byte slices. It does not read or write files,
/// and the returned bytes are a Compound File containing the streams and
/// DataSpaces hierarchy used by encrypted OOXML documents.
pub(crate) fn encrypt_ooxml(package: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let package_length = u32::try_from(package.len())
        .map_err(|_| "The OOXML package is too large to encrypt.".to_string())?;
    let password_salt = random_bytes::<KEY_DATA_SALT_SIZE>()?;
    let key_data_salt = random_bytes::<KEY_DATA_SALT_SIZE>()?;
    let package_key = random_bytes::<PACKAGE_KEY_SIZE>()?;
    let verifier_hash_input = random_bytes::<BLOCK_SIZE>()?;
    let integrity_salt = random_bytes::<KEY_DATA_SALT_SIZE>()?;

    let password_hash = iterated_password_hash(&password_salt, password);
    let encrypted_verifier_hash_input = encrypt_aes_cbc(
        &derive_key(&password_hash, &VERIFIER_HASH_INPUT_BLOCK_KEY),
        &password_salt,
        &verifier_hash_input,
    )?;
    let verifier_hash = Sha512::digest(verifier_hash_input);
    let encrypted_verifier_hash_value = encrypt_aes_cbc(
        &derive_key(&password_hash, &VERIFIER_HASH_VALUE_BLOCK_KEY),
        &password_salt,
        &verifier_hash,
    )?;
    let encrypted_key_value = encrypt_aes_cbc(
        &derive_key(&password_hash, &ENCRYPTED_KEY_VALUE_BLOCK_KEY),
        &password_salt,
        &package_key,
    )?;

    let encrypted_package = encrypt_package(package, package_length, &package_key, &key_data_salt)?;
    let integrity_value = hmac_sha512(&integrity_salt, &encrypted_package)?;
    let encrypted_integrity_key = encrypt_aes_cbc(
        &package_key,
        &derive_iv(&key_data_salt, &INTEGRITY_KEY_BLOCK_KEY),
        &integrity_salt,
    )?;
    let encrypted_integrity_value = encrypt_aes_cbc(
        &package_key,
        &derive_iv(&key_data_salt, &INTEGRITY_VALUE_BLOCK_KEY),
        &integrity_value,
    )?;

    let encryption_info = encryption_info_xml(
        &key_data_salt,
        &encrypted_integrity_key,
        &encrypted_integrity_value,
        &password_salt,
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

pub(crate) fn is_agile_ooxml(encrypted: &[u8]) -> Result<bool, String> {
    let mut compound = CompoundFile::open(Cursor::new(encrypted.to_vec()))
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

pub(crate) fn decrypt_ooxml(encrypted: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut compound = CompoundFile::open(Cursor::new(encrypted.to_vec()))
        .map_err(|error| format!("Could not open the encrypted Office container: {error}"))?;
    let encryption_info = read_compound_stream(&mut compound, "/EncryptionInfo")?;
    let encrypted_package = read_compound_stream(&mut compound, "/EncryptedPackage")?;
    let parameters = parse_encryption_info(&encryption_info)?;

    let password_hash = iterated_password_hash(&parameters.password_salt, password);
    let package_key: [u8; PACKAGE_KEY_SIZE] = decrypt_aes_cbc(
        &derive_key(&password_hash, &ENCRYPTED_KEY_VALUE_BLOCK_KEY),
        &parameters.password_salt,
        &parameters.encrypted_key_value,
    )?
    .try_into()
    .map_err(|_| "The Agile package key has an invalid length.".to_string())?;

    let verifier_hash_input = decrypt_aes_cbc(
        &derive_key(&password_hash, &VERIFIER_HASH_INPUT_BLOCK_KEY),
        &parameters.password_salt,
        &parameters.encrypted_verifier_hash_input,
    )?;
    let verifier_hash_value = decrypt_aes_cbc(
        &derive_key(&password_hash, &VERIFIER_HASH_VALUE_BLOCK_KEY),
        &parameters.password_salt,
        &parameters.encrypted_verifier_hash_value,
    )?;
    let expected_verifier_hash: [u8; HASH_SIZE] = Sha512::digest(&verifier_hash_input).into();
    if verifier_hash_value != expected_verifier_hash {
        return Err("The Agile password verifier did not match.".to_string());
    }

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

        let mut iv_input = Vec::with_capacity(KEY_DATA_SALT_SIZE + std::mem::size_of::<u32>());
        iv_input.extend_from_slice(&parameters.key_data_salt);
        iv_input.extend_from_slice(&segment_index.to_le_bytes());
        let segment_iv: [u8; BLOCK_SIZE] = Sha512::digest(iv_input)[..BLOCK_SIZE]
            .try_into()
            .map_err(|_| "The Agile segment IV has an invalid length.".to_string())?;
        let decrypted_segment = decrypt_aes_cbc(&package_key, &segment_iv, ciphertext)?;
        plaintext.extend_from_slice(&decrypted_segment[..segment_length]);

        encrypted_offset = ciphertext_end;
        remaining -= segment_length;
        segment_index = segment_index
            .checked_add(1)
            .ok_or_else(|| "The OOXML package has too many encryption segments.".to_string())?;
    }

    let integrity_key: [u8; KEY_DATA_SALT_SIZE] = decrypt_aes_cbc(
        &package_key,
        &derive_iv(&parameters.key_data_salt, &INTEGRITY_KEY_BLOCK_KEY),
        &parameters.encrypted_hmac_key,
    )?
    .try_into()
    .map_err(|_| "The Agile integrity key has an invalid length.".to_string())?;
    let integrity_value = decrypt_aes_cbc(
        &package_key,
        &derive_iv(&parameters.key_data_salt, &INTEGRITY_VALUE_BLOCK_KEY),
        &parameters.encrypted_hmac_value,
    )?;
    let computed_integrity = hmac_sha512(&integrity_key, &encrypted_package)?;
    if integrity_value != computed_integrity {
        return Err("The Agile package integrity HMAC did not match.".to_string());
    }

    Ok(plaintext)
}

pub(crate) fn verify_ooxml(
    encrypted: &[u8],
    password: &str,
    expected_package: &[u8],
) -> Result<(), String> {
    let plaintext = decrypt_ooxml(encrypted, password)?;

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

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|error| format!("Could not generate encryption randomness: {error}"))?;
    Ok(bytes)
}

fn iterated_password_hash(password_salt: &[u8; KEY_DATA_SALT_SIZE], password: &str) -> [u8; HASH_SIZE] {
    let mut password_bytes = Vec::with_capacity(password.encode_utf16().count() * 2);
    for code_unit in password.encode_utf16() {
        password_bytes.extend_from_slice(&code_unit.to_le_bytes());
    }

    let mut initial = Vec::with_capacity(password_salt.len() + password_bytes.len());
    initial.extend_from_slice(password_salt);
    initial.extend_from_slice(&password_bytes);
    let mut hash = sha512(&initial);

    for iterator in 0..SPIN_COUNT {
        let mut input = Vec::with_capacity(std::mem::size_of::<u32>() + hash.len());
        input.extend_from_slice(&iterator.to_le_bytes());
        input.extend_from_slice(&hash);
        hash = sha512(&input);
    }

    hash
}

fn sha512(input: &[u8]) -> [u8; HASH_SIZE] {
    Sha512::digest(input).into()
}

fn derive_key(hash: &[u8; HASH_SIZE], block_key: &[u8; 8]) -> [u8; PACKAGE_KEY_SIZE] {
    let mut input = Vec::with_capacity(hash.len() + block_key.len());
    input.extend_from_slice(hash);
    input.extend_from_slice(block_key);
    let digest = Sha512::digest(input);
    digest[..PACKAGE_KEY_SIZE].try_into().expect("SHA-512 has at least 32 bytes")
}

fn derive_iv(salt: &[u8; KEY_DATA_SALT_SIZE], block_key: &[u8; 8]) -> [u8; BLOCK_SIZE] {
    let mut input = Vec::with_capacity(salt.len() + block_key.len());
    input.extend_from_slice(salt);
    input.extend_from_slice(block_key);
    Sha512::digest(input)[..BLOCK_SIZE]
        .try_into()
        .expect("SHA-512 has at least 16 bytes")
}

fn encrypt_aes_cbc(
    key: &[u8; PACKAGE_KEY_SIZE],
    iv: &[u8; BLOCK_SIZE],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    if plaintext.len() % BLOCK_SIZE != 0 {
        return Err("Agile AES plaintext must be block aligned.".to_string());
    }

    let mut ciphertext = vec![0_u8; plaintext.len()];
    let encrypted = Aes256CbcEncryptor::new(key.into(), iv.into())
        .encrypt_padded_b2b_mut::<NoPadding>(plaintext, &mut ciphertext)
        .map_err(|_| "Could not encrypt an Agile OOXML value.".to_string())?;
    Ok(encrypted.to_vec())
}

fn decrypt_aes_cbc(
    key: &[u8; PACKAGE_KEY_SIZE],
    iv: &[u8; BLOCK_SIZE],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    if ciphertext.len() % BLOCK_SIZE != 0 {
        return Err("Agile AES ciphertext must be block aligned.".to_string());
    }

    let mut plaintext = vec![0_u8; ciphertext.len()];
    let decrypted = Aes256CbcDecryptor::new(key.into(), iv.into())
        .decrypt_padded_b2b_mut::<NoPadding>(ciphertext, &mut plaintext)
        .map_err(|_| "Could not decrypt an Agile OOXML value.".to_string())?;
    Ok(decrypted.to_vec())
}

fn encrypt_package(
    package: &[u8],
    package_length: u32,
    package_key: &[u8; PACKAGE_KEY_SIZE],
    key_data_salt: &[u8; KEY_DATA_SALT_SIZE],
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
        let mut iv_input = Vec::with_capacity(key_data_salt.len() + std::mem::size_of::<u32>());
        iv_input.extend_from_slice(key_data_salt);
        iv_input.extend_from_slice(&segment_index.to_le_bytes());
        let iv: [u8; BLOCK_SIZE] = Sha512::digest(iv_input)[..BLOCK_SIZE]
            .try_into()
            .expect("SHA-512 has at least 16 bytes");
        encrypted_package.extend_from_slice(&encrypt_aes_cbc(
            package_key,
            &iv,
            &padded_segment,
        )?);
    }

    Ok(encrypted_package)
}

fn hmac_sha512(key: &[u8; KEY_DATA_SALT_SIZE], data: &[u8]) -> Result<[u8; HASH_SIZE], String> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key)
        .map_err(|_| "Could not initialize the Agile integrity HMAC.".to_string())?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().into())
}

fn encryption_info_xml(
    key_data_salt: &[u8; KEY_DATA_SALT_SIZE],
    encrypted_integrity_key: &[u8],
    encrypted_integrity_value: &[u8],
    password_salt: &[u8; KEY_DATA_SALT_SIZE],
    encrypted_verifier_hash_input: &[u8],
    encrypted_verifier_hash_value: &[u8],
    encrypted_key_value: &[u8],
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<encryption xmlns=\"http://schemas.microsoft.com/office/2006/encryption\" xmlns:p=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\n  <keyData saltSize=\"16\" blockSize=\"16\" keyBits=\"256\" hashSize=\"64\" cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"SHA512\" saltValue=\"{}\"/>\n  <dataIntegrity encryptedHmacKey=\"{}\" encryptedHmacValue=\"{}\"/>\n  <keyEncryptors>\n    <keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\n      <p:encryptedKey spinCount=\"{}\" saltSize=\"16\" blockSize=\"16\" keyBits=\"256\" hashSize=\"64\" cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"SHA512\" saltValue=\"{}\" encryptedVerifierHashInput=\"{}\" encryptedVerifierHashValue=\"{}\" encryptedKeyValue=\"{}\"/>\n    </keyEncryptor>\n  </keyEncryptors>\n</encryption>",
        BASE64.encode(key_data_salt),
        BASE64.encode(encrypted_integrity_key),
        BASE64.encode(encrypted_integrity_value),
        SPIN_COUNT,
        BASE64.encode(password_salt),
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
    key_data_salt: Option<[u8; KEY_DATA_SALT_SIZE]>,
    encrypted_hmac_key: Option<[u8; KEY_DATA_SALT_SIZE]>,
    encrypted_hmac_value: Option<[u8; HASH_SIZE]>,
    spin_count: Option<u32>,
    password_salt: Option<[u8; KEY_DATA_SALT_SIZE]>,
    encrypted_verifier_hash_input: Option<[u8; BLOCK_SIZE]>,
    encrypted_verifier_hash_value: Option<[u8; HASH_SIZE]>,
    encrypted_key_value: Option<[u8; PACKAGE_KEY_SIZE]>,
}

fn parse_encryption_info(encryption_info: &[u8]) -> Result<ParsedAgileParameters, String> {
    if encryption_info.len() <= 8 {
        return Err("The EncryptionInfo stream is incomplete.".to_string());
    }
    if encryption_info[..4] != [0x04, 0x00, 0x04, 0x00]
        || u32::from_le_bytes(
            encryption_info[4..8]
                .try_into()
                .expect("the EncryptionInfo header is four bytes"),
        ) != 0x40
    {
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
    parameters
        .spin_count
        .ok_or_else(|| "The EncryptionInfo XML has no password spin count.".to_string())?;

    Ok(ParsedAgileParameters {
        key_data_salt: parameters
            .key_data_salt
            .ok_or_else(|| "The EncryptionInfo XML has no keyData salt.".to_string())?,
        encrypted_hmac_key: parameters
            .encrypted_hmac_key
            .ok_or_else(|| "The EncryptionInfo XML has no encrypted HMAC key.".to_string())?,
        encrypted_hmac_value: parameters
            .encrypted_hmac_value
            .ok_or_else(|| "The EncryptionInfo XML has no encrypted HMAC value.".to_string())?,
        password_salt: parameters
            .password_salt
            .ok_or_else(|| "The EncryptionInfo XML has no password salt.".to_string())?,
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
    key_data_salt: [u8; KEY_DATA_SALT_SIZE],
    encrypted_hmac_key: [u8; KEY_DATA_SALT_SIZE],
    encrypted_hmac_value: [u8; HASH_SIZE],
    password_salt: [u8; KEY_DATA_SALT_SIZE],
    encrypted_verifier_hash_input: [u8; BLOCK_SIZE],
    encrypted_verifier_hash_value: [u8; HASH_SIZE],
    encrypted_key_value: [u8; PACKAGE_KEY_SIZE],
}

fn parse_agile_element(
    element: &BytesStart<'_>,
    parameters: &mut AgileParameters,
) -> Result<(), String> {
    match element.local_name().as_ref() {
        b"keyData" => {
            if parameters.key_data_salt.is_some() {
                return Err("The EncryptionInfo XML has duplicate keyData elements.".to_string());
            }
            parameters.key_data_salt = Some(read_base64_attribute(element, b"saltValue")?);
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
            if spin_count != SPIN_COUNT {
                return Err("The Agile encryption spin count is unsupported.".to_string());
            }
            parameters.spin_count = Some(spin_count);
            parameters.password_salt = Some(read_base64_attribute(element, b"saltValue")?);
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

fn read_base64_attribute<const N: usize>(
    element: &BytesStart<'_>,
    name: &[u8],
) -> Result<[u8; N], String> {
    let encoded = read_xml_attribute(element, name)?;
    let decoded = BASE64
        .decode(encoded.as_bytes())
        .map_err(|error| format!("The Agile {} attribute is not base64: {error}", String::from_utf8_lossy(name)))?;
    decoded.try_into().map_err(|decoded: Vec<u8>| {
        format!(
            "The Agile {} attribute has an invalid length: expected {N}, got {}.",
            String::from_utf8_lossy(name),
            decoded.len()
        )
    })
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use cfb::CompoundFile;
    use super::{encrypt_ooxml, verify_ooxml};
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
}
