#![allow(clippy::doc_markdown)]

use std::fs;
use std::path::Path;

use goblin::pe::PE;
use sha2::{Digest, Sha256};

use crate::models::environment::{AntiCheat, DirectXVersion, GameEngine, PeArchitecture};
use crate::models::errors::PlayError;

/// Static analysis result from a PE binary — no process execution involved.
#[derive(Debug, Clone)]
pub struct BinaryAnalysis {
    pub hash: String,
    pub dx_version: DirectXVersion,
    pub pe_arch: PeArchitecture,
    pub anti_cheat: Vec<AntiCheat>,
    pub engine_hint: Option<GameEngine>,
    /// RADGame Tools / Bink = likely WMF cutscenes
    pub has_bink_video: bool,
}

/// Perform static analysis on a PE binary.
///
/// Computes the SHA256 hash first (so we have it even if parsing fails),
/// then parses the PE import table to determine DX version, anti-cheat
/// presence, engine hints, and video codec signals.
///
/// # Errors
///
/// Returns `PlayError::BinaryAnalysis` if the file cannot be read or
/// is not a valid PE binary.
pub fn analyze_binary(path: &Path) -> Result<BinaryAnalysis, PlayError> {
    let bytes = fs::read(path).map_err(|e| PlayError::BinaryAnalysis {
        path: path.into(),
        reason: format!("failed to read file: {e}"),
    })?;

    // Hash before parsing — we need it regardless of parse success.
    // Raw hex only — no "sha256:" prefix. The field is already named exe_hash.
    let hash = format!("{:x}", Sha256::digest(&bytes));

    let pe = PE::parse(&bytes).map_err(|e| PlayError::BinaryAnalysis {
        path: path.into(),
        reason: format!("PE parse failed: {e}"),
    })?;

    let imports: Vec<String> = pe.libraries.iter().map(|s| s.to_lowercase()).collect();

    let dx_version = detect_dx_version(&imports);
    let anti_cheat = detect_anti_cheat(&imports);
    let engine_hint = detect_engine(&imports);
    let has_bink_video = imports.iter().any(|i| i.contains("bink") || i.contains("rad"));

    let pe_arch = if pe.is_64 { PeArchitecture::X86_64 } else { PeArchitecture::X86 };

    Ok(BinaryAnalysis { hash, dx_version, pe_arch, anti_cheat, engine_hint, has_bink_video })
}

/// Determine DX version from PE import table.
/// Checks most specific (D3D12) first, falls through to Unknown.
fn detect_dx_version(imports: &[String]) -> DirectXVersion {
    if imports.iter().any(|i| i == "d3d12.dll") {
        DirectXVersion::D3D12
    } else if imports.iter().any(|i| i == "d3d11.dll") {
        DirectXVersion::D3D11
    } else if imports.iter().any(|i| i == "d3d10.dll") {
        DirectXVersion::D3D10
    } else if imports.iter().any(|i| i == "d3d9.dll") {
        DirectXVersion::D3D9
    } else if imports.iter().any(|i| i == "d3d8.dll") {
        DirectXVersion::D3D8
    } else if imports.iter().any(|i| i == "vulkan-1.dll") {
        DirectXVersion::Vulkan
    } else if imports.iter().any(|i| i == "opengl32.dll") {
        DirectXVersion::OpenGL
    } else {
        DirectXVersion::Unknown
    }
}

/// Detect anti-cheat DLLs from PE import table.
fn detect_anti_cheat(imports: &[String]) -> Vec<AntiCheat> {
    let mut result = vec![];

    if imports.iter().any(|i| i.contains("easyanticheat")) {
        // Default to unsupported; the database can override with actual status
        result.push(AntiCheat::EasyAntiCheat { linux_supported: false });
    }
    if imports.iter().any(|i| i.contains("battleye")) {
        result.push(AntiCheat::BattlEye { linux_supported: false });
    }
    if imports.iter().any(|i| i.contains("denuvo")) {
        result.push(AntiCheat::Denuvo);
    }
    if imports.iter().any(|i| i.contains("vmprotect")) {
        result.push(AntiCheat::VMProtect);
    }
    if imports.iter().any(|i| i.contains("gameguard") || i.contains("nprotect")) {
        result.push(AntiCheat::GameGuard);
    }

    result
}

/// Detect game engine from known DLL import signatures.
fn detect_engine(imports: &[String]) -> Option<GameEngine> {
    // Unreal Engine 5: UE5Game.dll or UE5-Win64-Shipping.dll patterns
    if imports.iter().any(|i| i.contains("ue5")) {
        return Some(GameEngine::UnrealEngine5);
    }
    // Unreal Engine 4: ships PhysX/Phonon audio DLLs
    if imports.iter().any(|i| i.contains("phonon")) {
        return Some(GameEngine::UnrealEngine4);
    }
    // RE Engine (Capcom): mt_framework signature
    if imports.iter().any(|i| i.contains("mt_framework")) {
        return Some(GameEngine::REEngine);
    }
    // Unity: mono runtime or UnityPlayer
    if imports.iter().any(|i| i.contains("unityplayer") || i.contains("mono")) {
        return Some(GameEngine::Unity);
    }
    // Source engine: tier0/vstdlib
    if imports.iter().any(|i| i == "tier0.dll" || i == "vstdlib.dll") {
        return Some(GameEngine::Source);
    }
    // Source 2: tier0 + scenesystem
    if imports.iter().any(|i| i.contains("scenesystem")) {
        return Some(GameEngine::Source2);
    }
    // id Tech: idlib signature
    if imports.iter().any(|i| i.contains("idlib")) {
        return Some(GameEngine::IDAEngine);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal valid 32-bit PE binary with the given import DLLs.
    /// This constructs the absolute minimum PE structure goblin can parse.
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::similar_names)]
    fn build_minimal_pe32(import_dlls: &[&str]) -> Vec<u8> {
        let mut buf = build_dos_and_pe_headers();
        let (section_rva, import_dir_rva_offset, import_dir_size_offset) =
            add_section_header(&mut buf);
        let section_data_start = finalize_headers(&mut buf);

        let section_data = build_import_section_data(import_dlls, section_rva);
        buf.extend_from_slice(&section_data);

        fixup_header_values(
            &mut buf,
            section_rva,
            section_data_start,
            import_dir_rva_offset,
            import_dir_size_offset,
            section_data.len(),
            import_dlls.len(),
        );

        buf
    }

    /// Build DOS header and PE/COFF headers.
    fn build_dos_and_pe_headers() -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // --- DOS Header (64 bytes) ---
        buf.extend_from_slice(b"MZ");
        buf.resize(0x3C, 0);
        buf.extend_from_slice(&0x40_u32.to_le_bytes());
        buf.resize(0x40, 0);

        // --- PE Signature ---
        buf.extend_from_slice(b"PE\0\0");

        // --- COFF Header (20 bytes) ---
        buf.extend_from_slice(&0x014C_u16.to_le_bytes()); // Machine
        buf.extend_from_slice(&1_u16.to_le_bytes()); // NumberOfSections
        buf.extend_from_slice(&0_u32.to_le_bytes()); // TimeDateStamp
        buf.extend_from_slice(&0_u32.to_le_bytes()); // PointerToSymbolTable
        buf.extend_from_slice(&0_u32.to_le_bytes()); // NumberOfSymbols
        let num_data_dirs: u32 = 2;
        let optional_header_size: u16 = 96 + (num_data_dirs as u16) * 8;
        buf.extend_from_slice(&optional_header_size.to_le_bytes());
        buf.extend_from_slice(&0x0102_u16.to_le_bytes()); // Characteristics

        // --- Optional Header (PE32) ---
        buf.extend_from_slice(&0x010B_u16.to_le_bytes()); // Magic
        buf.extend_from_slice(&[0u8; 2]); // MajorLinkerVersion, MinorLinkerVersion
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfCode
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfInitializedData
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfUninitializedData
        buf.extend_from_slice(&0x1000_u32.to_le_bytes()); // AddressOfEntryPoint
        buf.extend_from_slice(&0x1000_u32.to_le_bytes()); // BaseOfCode
        buf.extend_from_slice(&0x2000_u32.to_le_bytes()); // BaseOfData
        buf.extend_from_slice(&0x0040_0000_u32.to_le_bytes()); // ImageBase
        buf.extend_from_slice(&0x1000_u32.to_le_bytes()); // SectionAlignment
        buf.extend_from_slice(&0x200_u32.to_le_bytes()); // FileAlignment
        buf.extend_from_slice(&4_u16.to_le_bytes()); // MajorOperatingSystemVersion
        buf.extend_from_slice(&0_u16.to_le_bytes()); // MinorOperatingSystemVersion
        buf.extend_from_slice(&0_u16.to_le_bytes()); // MajorImageVersion
        buf.extend_from_slice(&0_u16.to_le_bytes()); // MinorImageVersion
        buf.extend_from_slice(&4_u16.to_le_bytes()); // MajorSubsystemVersion
        buf.extend_from_slice(&0_u16.to_le_bytes()); // MinorSubsystemVersion
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Win32VersionValue
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfImage (placeholder)
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfHeaders (placeholder)
        buf.extend_from_slice(&0_u32.to_le_bytes()); // CheckSum
        buf.extend_from_slice(&3_u16.to_le_bytes()); // Subsystem
        buf.extend_from_slice(&0_u16.to_le_bytes()); // DllCharacteristics
        buf.extend_from_slice(&0x0010_0000_u32.to_le_bytes()); // SizeOfStackReserve
        buf.extend_from_slice(&0x1000_u32.to_le_bytes()); // SizeOfStackCommit
        buf.extend_from_slice(&0x0010_0000_u32.to_le_bytes()); // SizeOfHeapReserve
        buf.extend_from_slice(&0x1000_u32.to_le_bytes()); // SizeOfHeapCommit
        buf.extend_from_slice(&0_u32.to_le_bytes()); // LoaderFlags
        buf.extend_from_slice(&2_u32.to_le_bytes()); // NumberOfRvaAndSizes

        // --- Data Directories ---
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Export Table RVA
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Export Table Size
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Import Table RVA (placeholder)
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Import Table Size (placeholder)

        buf
    }

    /// Add section header and return important offsets.
    fn add_section_header(buf: &mut Vec<u8>) -> (u32, usize, usize) {
        // --- Section Header: .idata (40 bytes) ---
        buf.extend_from_slice(b".idata\0\0");
        buf.extend_from_slice(&0_u32.to_le_bytes()); // VirtualSize (placeholder)
        let section_rva: u32 = 0x2000;
        buf.extend_from_slice(&section_rva.to_le_bytes()); // VirtualAddress
        buf.extend_from_slice(&0_u32.to_le_bytes()); // SizeOfRawData (placeholder)
        buf.extend_from_slice(&0_u32.to_le_bytes()); // PointerToRawData (placeholder)
        buf.extend_from_slice(&0_u32.to_le_bytes()); // PointerToRelocations
        buf.extend_from_slice(&0_u32.to_le_bytes()); // PointerToLinenumbers
        buf.extend_from_slice(&0_u16.to_le_bytes()); // NumberOfRelocations
        buf.extend_from_slice(&0_u16.to_le_bytes()); // NumberOfLinenumbers
        buf.extend_from_slice(&0xC000_0040_u32.to_le_bytes()); // Characteristics

        // Align to file alignment (0x200)
        let headers_end = buf.len();
        let aligned_headers = (headers_end + 0x1FF) & !0x1FF;
        buf.resize(aligned_headers, 0);

        let import_dir_rva_offset = 0xC0; // Import Table RVA in optional header data directories
        let import_dir_size_offset = 0xC4; // Import Table Size in optional header data directories

        (section_rva, import_dir_rva_offset, import_dir_size_offset)
    }

    /// Finalize headers and return section data start position.
    fn finalize_headers(buf: &mut Vec<u8>) -> usize {
        let size_of_headers_offset = 0x94; // SizeOfHeaders in optional header
        let size_of_headers = buf.len() as u32;
        buf[size_of_headers_offset..size_of_headers_offset + 4]
            .copy_from_slice(&size_of_headers.to_le_bytes());
        buf.len()
    }

    /// Build the import section data.
    fn build_import_section_data(import_dlls: &[&str], section_rva: u32) -> Vec<u8> {
        let n = import_dlls.len();
        let idt_size = (n + 1) * 20;

        // Build names blob
        let mut names_blob: Vec<u8> = Vec::new();
        for dll in import_dlls {
            names_blob.extend_from_slice(dll.as_bytes());
            names_blob.push(0);
        }
        let names_size = names_blob.len();
        let import_lookup_table_size = n * 4;

        // Calculate offsets
        let names_start = idt_size;
        let import_lookup_table_start = names_start + names_size;
        let import_address_table_start = import_lookup_table_start + import_lookup_table_size;

        // Collect name RVAs
        let mut name_rvas: Vec<u32> = Vec::new();
        let mut offset = 0usize;
        for dll in import_dlls {
            name_rvas.push(section_rva + (names_start as u32) + (offset as u32));
            offset += dll.len() + 1;
        }

        let mut section_data: Vec<u8> = Vec::new();

        // Write IDT entries
        for (i, &name_rva) in name_rvas.iter().enumerate().take(n) {
            let import_lookup_table_rva =
                section_rva + (import_lookup_table_start as u32) + (i as u32) * 4;
            let import_address_table_rva =
                section_rva + (import_address_table_start as u32) + (i as u32) * 4;
            section_data.extend_from_slice(&import_lookup_table_rva.to_le_bytes());
            section_data.extend_from_slice(&0_u32.to_le_bytes()); // TimeDateStamp
            section_data.extend_from_slice(&0_u32.to_le_bytes()); // ForwarderChain
            section_data.extend_from_slice(&name_rva.to_le_bytes()); // Name RVA
            section_data.extend_from_slice(&import_address_table_rva.to_le_bytes());
            // FirstThunk
        }
        section_data.extend_from_slice(&[0u8; 20]); // Null terminator

        // Write DLL names
        section_data.extend_from_slice(&names_blob);

        // Write ILT and IAT entries
        for _ in 0..n {
            section_data.extend_from_slice(&0_u32.to_le_bytes());
        }
        for _ in 0..n {
            section_data.extend_from_slice(&0_u32.to_le_bytes());
        }

        section_data
    }

    /// Fix up header values with final section information.
    fn fixup_header_values(
        buf: &mut Vec<u8>,
        section_rva: u32,
        section_data_start: usize,
        import_dir_rva_offset: usize,
        import_dir_size_offset: usize,
        section_data_len: usize,
        num_dlls: usize,
    ) {
        let size_of_image_offset = 0x90; // SizeOfImage in optional header
        let virtual_size_offset = 0xD0; // Section VirtualSize
        let raw_size_offset = 0xD8; // Section SizeOfRawData
        let raw_ptr_offset = 0xDC; // Section PointerToRawData

        let section_raw_size = section_data_len as u32;
        let section_virtual_size = section_raw_size;
        let section_file_offset = section_data_start as u32;

        // Fix up section header
        buf[virtual_size_offset..virtual_size_offset + 4]
            .copy_from_slice(&section_virtual_size.to_le_bytes());
        buf[raw_size_offset..raw_size_offset + 4].copy_from_slice(&section_raw_size.to_le_bytes());
        buf[raw_ptr_offset..raw_ptr_offset + 4].copy_from_slice(&section_file_offset.to_le_bytes());

        // Fix up import directory
        buf[import_dir_rva_offset..import_dir_rva_offset + 4]
            .copy_from_slice(&section_rva.to_le_bytes());
        // Import directory size: (num_dlls + 1) * 20, where +1 is the null terminator entry
        // We receive num_dlls as a parameter instead of reverse-computing from section_data_len
        let import_dir_size = ((num_dlls + 1) * 20) as u32;
        buf[import_dir_size_offset..import_dir_size_offset + 4]
            .copy_from_slice(&import_dir_size.to_le_bytes());

        // Fix up SizeOfImage
        let size_of_image = section_rva + ((section_virtual_size + 0xFFF) & !0xFFF);
        buf[size_of_image_offset..size_of_image_offset + 4]
            .copy_from_slice(&size_of_image.to_le_bytes());
    }

    /// Helper: write PE bytes to a temp file and run analyze_binary on it.
    fn analyze_pe_bytes(bytes: &[u8]) -> Result<BinaryAnalysis, PlayError> {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let pe_path = dir.path().join("test.exe");
        let mut f = fs::File::create(&pe_path).expect("failed to create temp file");
        f.write_all(bytes).expect("failed to write temp file");
        drop(f);
        analyze_binary(&pe_path)
    }

    #[test]
    fn test_d3d12_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "d3d12.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::D3D12);
        assert_eq!(result.pe_arch, PeArchitecture::X86);
        assert!(!result.hash.contains(':'), "hash should be raw hex without prefix");
    }

    #[test]
    fn test_d3d11_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "d3d11.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::D3D11);
    }

    #[test]
    fn test_d3d9_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "d3d9.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::D3D9);
    }

    #[test]
    fn test_vulkan_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "vulkan-1.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::Vulkan);
    }

    #[test]
    fn test_opengl_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "opengl32.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::OpenGL);
    }

    #[test]
    fn test_unknown_dx_when_no_graphics_dll() {
        let pe = build_minimal_pe32(&["KERNEL32.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::Unknown);
    }

    #[test]
    fn test_d3d12_takes_priority_over_d3d11() {
        let pe = build_minimal_pe32(&["d3d11.dll", "d3d12.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.dx_version, DirectXVersion::D3D12);
    }

    #[test]
    fn test_eac_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "EasyAntiCheat.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.anti_cheat, vec![AntiCheat::EasyAntiCheat { linux_supported: false }]);
    }

    #[test]
    fn test_battleye_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "BattlEye.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.anti_cheat, vec![AntiCheat::BattlEye { linux_supported: false }]);
    }

    #[test]
    fn test_no_anti_cheat_when_clean() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "d3d11.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert!(result.anti_cheat.is_empty());
    }

    #[test]
    fn test_unity_engine_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "UnityPlayer.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.engine_hint, Some(GameEngine::Unity));
    }

    #[test]
    fn test_source_engine_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "tier0.dll", "vstdlib.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.engine_hint, Some(GameEngine::Source));
    }

    #[test]
    fn test_ue5_engine_detected_from_ue5game_dll() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "UE5Game.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.engine_hint, Some(GameEngine::UnrealEngine5));
    }

    #[test]
    fn test_ue5_engine_detected_from_ue5_win64_shipping() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "UE5-Win64-Shipping.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.engine_hint, Some(GameEngine::UnrealEngine5));
    }

    #[test]
    fn test_ue5_takes_precedence_over_ue4() {
        // UE5 game that still imports phonon (UE4 audio DLL)
        let pe = build_minimal_pe32(&["KERNEL32.dll", "phonon.dll", "UE5Game.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(result.engine_hint, Some(GameEngine::UnrealEngine5));
    }

    #[test]
    fn test_bink_video_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "bink2w64.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert!(result.has_bink_video);
    }

    #[test]
    fn test_no_bink_video_when_absent() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "d3d11.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert!(!result.has_bink_video);
    }

    #[test]
    fn test_hash_is_deterministic() {
        let pe = build_minimal_pe32(&["KERNEL32.dll"]);
        let r1 = analyze_pe_bytes(&pe).expect("first analysis");
        let r2 = analyze_pe_bytes(&pe).expect("second analysis");
        assert_eq!(r1.hash, r2.hash);
    }

    #[test]
    fn test_nonexistent_file_returns_error() {
        let result = analyze_binary(Path::new("/tmp/nonexistent_game_9f8a7b.exe"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Binary analysis failed"));
        assert!(msg.contains("nonexistent_game_9f8a7b.exe"));
    }

    #[test]
    fn test_invalid_pe_returns_error() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("garbage.exe");
        fs::write(&path, b"this is not a PE file").expect("write");
        let result = analyze_binary(&path);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("PE parse failed"));
    }
}
