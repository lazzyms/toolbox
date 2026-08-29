# PDF Encryption Fidelity Research: Rust vs. PDFKit

This research investigates the capability of Rust libraries to handle PDF encryption and password management compared to Apple's native `PDFKit`, specifically for "Protect PDF" (adding passwords) and "Remove Password PDF" (removing passwords) tools.

## Executive Summary

For the Tauri Toolbox project, **`lopdf`** is the only viable Rust crate that supports both adding and removing passwords with modern encryption standards (AES-256). Other libraries like `pdf-rs` lack encryption support entirely or focus primarily on decryption. While `lopdf` provides the necessary primitives to match `PDFKit`'s functionality, it requires more manual implementation of the PDF specification (e.g., managing the File Encryption Key).

## Library Comparison

### 1. `lopdf` (Recommended)
The most feature-complete crate for PDF manipulation.

| Feature | Support | Implementation Detail |
| :--- | :--- | :--- |
| **AES-256** | ✅ Yes | Supported via `EncryptionVersion::V5` (PDF 2.0). |
| **AES-128** | ✅ Yes | Supported via `EncryptionVersion::V4` (PDF 1.5). |
| **Legacy RC4** | ✅ Yes | Supported for older PDF versions. |
| **Remove Password** | ✅ Yes | Decrypt document using `load_with_password`, then `save()` without re-encrypting. |
| **Add Password** | ✅ Yes | Create `EncryptionVersion` $\rightarrow$ `EncryptionState` $\rightarrow$ `document.encrypt(&state)`. |

### 2. `pdf-rs`
Not suitable for these tools.
- **Encryption Support:** ❌ None.
- **Status:** Currently lists "Add encrypted PDF support" as a future plan in its roadmap.

### 3. Other Rust Alternatives
- **`spectre_pdf`**: Strong focus on decryption across all revisions since 1996.
- **`oxideav-pdf`**: Supports standard security handlers including AES-256 and RC4.
- **`pdf_oxide`**: Provides password authentication and decryption.

## Comparison with Apple's `PDFKit`

### Capabilities
Both `PDFKit` and `lopdf` can achieve the same end-goals: protecting a PDF with AES-256 and removing existing passwords.

### Key Differences & Limitations

| Aspect | Apple `PDFKit` | Rust `lopdf` |
| :--- | :--- | :--- |
| **Abstraction Level** | High. Simple API calls to set passwords. | Low. Requires manual definition of encryption versions and states. |
| **Key Management** | Automatic. Handles FEK (File Encryption Key) generation internally. | Manual. The developer must generate a cryptographically secure random 32-byte FEK for AES-256. |
| **Compatibility** | Extremely high. Integrated with OS-level PDF standards. | High, but relies on the crate's implementation of the PDF spec. |
| **Performance** | Optimized native implementation. | Document is typically loaded into memory, which may be a bottleneck for very large files. |

## Conclusion

To implement the "Protect PDF" and "Remove Password PDF" tools in Rust:
1. Use **`lopdf`**.
2. For **Removing Passwords**: Use `Document::load_with_password` followed by `Document::save`.
3. For **Adding Passwords**: Use `EncryptionVersion::V5` (for AES-256), generate a random 32-byte FEK, and call `document.encrypt()`.

## Sources
- [`lopdf` Encryption Documentation](https://docs.rs/lopdf/latest/lopdf/encryption/index.html)
- [`lopdf` Source Code (encryption.rs)](https://docs.rs/lopdf/latest/src/lopdf/encryption.rs.html)
- [`pdf-rs` Documentation](https://docs.rs/crate/pdf-rs/latest)
- [`spectre_pdf` README](https://docs.rs/crate/spectre_pdf/latest/source/README.md)
- [`oxideav-pdf` Lib.rs](https://lib.rs/crates/oxideav-pdf)
