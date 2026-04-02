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

    // Hash before parsing — we need it regardless of parse success
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));

    let pe = PE::parse(&bytes).map_err(|e| PlayError::BinaryAnalysis {
        path: path.into(),
        reason: format!("PE parse failed: {e}"),
    })?;

    let imports: Vec<String> = pe.libraries.iter().map(|s| s.to_lowercase()).collect();

    let dx_version = detect_dx_version(&imports);
    let anti_cheat = detect_anti_cheat(&imports);
    let engine_hint = detect_engine(&imports);
    let has_bink_video = imports
        .iter()
        .any(|i| i.contains("bink") || i.contains("rad"));

    let pe_arch = if pe.is_64 {
        PeArchitecture::X86_64
    } else {
        PeArchitecture::X86
    };

    Ok(BinaryAnalysis {
        hash,
        dx_version,
        pe_arch,
        anti_cheat,
        engine_hint,
        has_bink_video,
    })
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
        result.push(AntiCheat::EasyAntiCheat {
            linux_supported: false,
        });
    }
    if imports.iter().any(|i| i.contains("battleye")) {
        result.push(AntiCheat::BattlEye {
            linux_supported: false,
        });
    }
    if imports.iter().any(|i| i.contains("denuvo")) {
        result.push(AntiCheat::Denuvo);
    }
    if imports.iter().any(|i| i.contains("vmprotect")) {
        result.push(AntiCheat::VMProtect);
    }
    if imports
        .iter()
        .any(|i| i.contains("gameguard") || i.contains("nprotect"))
    {
        result.push(AntiCheat::GameGuard);
    }

    result
}

/// Detect game engine from known DLL import signatures.
fn detect_engine(imports: &[String]) -> Option<GameEngine> {
    // Unreal Engine 4: ships PhysX/Phonon audio DLLs
    if imports.iter().any(|i| i.contains("phonon")) {
        return Some(GameEngine::UnrealEngine4);
    }
    // RE Engine (Capcom): mt_framework signature
    if imports.iter().any(|i| i.contains("mt_framework")) {
        return Some(GameEngine::REEngine);
    }
    // Unity: mono runtime or UnityPlayer
    if imports
        .iter()
        .any(|i| i.contains("unityplayer") || i.contains("mono"))
    {
        return Some(GameEngine::Unity);
    }
    // Source engine: tier0/vstdlib
    if imports
        .iter()
        .any(|i| i == "tier0.dll" || i == "vstdlib.dll")
    {
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
    fn build_minimal_pe32(import_dlls: &[&str]) -> Vec<u8> {
        // We'll build a PE by hand. This is the simplest possible PE:
        // - DOS header (64 bytes)
        // - PE signature (4 bytes)
        // - COFF header (20 bytes)
        // - Optional header (PE32) with import directory RVA
        // - Section header (.idata)
        // - Import Directory Table
        // - DLL name strings
        //
        // All values are little-endian.

        let mut buf: Vec<u8> = Vec::new();

        // --- DOS Header (64 bytes) ---
        // e_magic = "MZ"
        buf.extend_from_slice(b"MZ");
        // Pad to offset 0x3C
        buf.resize(0x3C, 0);
        // e_lfanew = offset to PE signature (we'll put it at 0x40)
        buf.extend_from_slice(&0x40_u32.to_le_bytes());

        // Pad to 0x40
        buf.resize(0x40, 0);

        // --- PE Signature ---
        buf.extend_from_slice(b"PE\0\0");

        // --- COFF Header (20 bytes) ---
        let _coff_offset = buf.len();
        // Machine: IMAGE_FILE_MACHINE_I386 = 0x014C
        buf.extend_from_slice(&0x014C_u16.to_le_bytes());
        // NumberOfSections: 1
        buf.extend_from_slice(&1_u16.to_le_bytes());
        // TimeDateStamp
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // PointerToSymbolTable
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // NumberOfSymbols
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // SizeOfOptionalHeader: PE32 optional header = 96 + data dirs
        // We need at least 2 data directory entries (export + import)
        let num_data_dirs: u32 = 2;
        let optional_header_size: u16 = 96 + (num_data_dirs as u16) * 8;
        buf.extend_from_slice(&optional_header_size.to_le_bytes());
        // Characteristics: EXECUTABLE_IMAGE | 32BIT_MACHINE
        buf.extend_from_slice(&0x0102_u16.to_le_bytes());

        // --- Optional Header (PE32) ---
        let _opt_offset = buf.len();
        // Magic: PE32 = 0x010B
        buf.extend_from_slice(&0x010B_u16.to_le_bytes());
        // MajorLinkerVersion, MinorLinkerVersion
        buf.extend_from_slice(&[0u8; 2]);
        // SizeOfCode
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // SizeOfInitializedData
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // SizeOfUninitializedData
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // AddressOfEntryPoint
        buf.extend_from_slice(&0x1000_u32.to_le_bytes());
        // BaseOfCode
        buf.extend_from_slice(&0x1000_u32.to_le_bytes());
        // BaseOfData (PE32 only)
        buf.extend_from_slice(&0x2000_u32.to_le_bytes());
        // ImageBase
        buf.extend_from_slice(&0x0040_0000_u32.to_le_bytes());
        // SectionAlignment
        buf.extend_from_slice(&0x1000_u32.to_le_bytes());
        // FileAlignment
        buf.extend_from_slice(&0x200_u32.to_le_bytes());
        // MajorOperatingSystemVersion
        buf.extend_from_slice(&4_u16.to_le_bytes());
        // MinorOperatingSystemVersion
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // MajorImageVersion
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // MinorImageVersion
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // MajorSubsystemVersion
        buf.extend_from_slice(&4_u16.to_le_bytes());
        // MinorSubsystemVersion
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // Win32VersionValue
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // SizeOfImage (will fix up later)
        let size_of_image_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // SizeOfHeaders (will fix up later)
        let size_of_headers_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // CheckSum
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // Subsystem: IMAGE_SUBSYSTEM_WINDOWS_CUI = 3
        buf.extend_from_slice(&3_u16.to_le_bytes());
        // DllCharacteristics
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // SizeOfStackReserve
        buf.extend_from_slice(&0x0010_0000_u32.to_le_bytes());
        // SizeOfStackCommit
        buf.extend_from_slice(&0x1000_u32.to_le_bytes());
        // SizeOfHeapReserve
        buf.extend_from_slice(&0x0010_0000_u32.to_le_bytes());
        // SizeOfHeapCommit
        buf.extend_from_slice(&0x1000_u32.to_le_bytes());
        // LoaderFlags
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // NumberOfRvaAndSizes
        buf.extend_from_slice(&num_data_dirs.to_le_bytes());

        // --- Data Directories ---
        // [0] Export Table: RVA=0, Size=0
        buf.extend_from_slice(&0_u32.to_le_bytes());
        buf.extend_from_slice(&0_u32.to_le_bytes());

        // [1] Import Table: RVA and Size will be fixed up
        let import_dir_rva_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes()); // RVA placeholder
        let import_dir_size_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes()); // Size placeholder

        // --- Section Header: .idata (40 bytes) ---
        let _section_header_offset = buf.len();
        // Name: ".idata\0\0"
        buf.extend_from_slice(b".idata\0\0");
        // VirtualSize (placeholder)
        let virtual_size_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // VirtualAddress: 0x2000
        let section_rva: u32 = 0x2000;
        buf.extend_from_slice(&section_rva.to_le_bytes());
        // SizeOfRawData (placeholder)
        let raw_size_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // PointerToRawData (placeholder)
        let raw_ptr_offset = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // PointerToRelocations
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // PointerToLinenumbers
        buf.extend_from_slice(&0_u32.to_le_bytes());
        // NumberOfRelocations
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // NumberOfLinenumbers
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // Characteristics: IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
        buf.extend_from_slice(&0xC000_0040_u32.to_le_bytes());

        // Align to file alignment (0x200)
        let headers_end = buf.len();
        let aligned_headers = (headers_end + 0x1FF) & !0x1FF;
        buf.resize(aligned_headers, 0);

        // Fix SizeOfHeaders
        let size_of_headers = aligned_headers as u32;
        buf[size_of_headers_offset..size_of_headers_offset + 4]
            .copy_from_slice(&size_of_headers.to_le_bytes());

        // --- Section Data ---
        // Layout within the section:
        //   1. IDT: (N+1) * 20 bytes  (N entries + null terminator)
        //   2. DLL name strings: variable
        //   3. ILT: N * 4 bytes  (one null u32 per DLL = empty import list)
        //   4. IAT: N * 4 bytes  (mirrors ILT)
        let section_data_start = buf.len();
        let section_file_offset = section_data_start as u32;
        let n = import_dlls.len();

        // Pre-calculate region sizes
        let idt_size = (n + 1) * 20;
        let mut names_blob: Vec<u8> = Vec::new();
        for dll in import_dlls {
            names_blob.extend_from_slice(dll.as_bytes());
            names_blob.push(0);
        }
        let names_size = names_blob.len();
        let ilt_size = n * 4; // one null-terminator u32 per DLL
        let _iat_size = n * 4;

        // Pre-calculate offsets from section start
        let names_start = idt_size;
        let ilt_start = names_start + names_size;
        let iat_start = ilt_start + ilt_size;

        // Collect RVAs for each DLL's name, ILT entry, and IAT entry
        let mut name_rvas: Vec<u32> = Vec::new();
        let mut offset = 0usize;
        for dll in import_dlls {
            name_rvas.push(section_rva + (names_start as u32) + (offset as u32));
            offset += dll.len() + 1;
        }

        // Write IDT entries
        for i in 0..n {
            let ilt_rva = section_rva + (ilt_start as u32) + (i as u32) * 4;
            let iat_rva = section_rva + (iat_start as u32) + (i as u32) * 4;
            // OriginalFirstThunk (ILT RVA)
            buf.extend_from_slice(&ilt_rva.to_le_bytes());
            // TimeDateStamp
            buf.extend_from_slice(&0_u32.to_le_bytes());
            // ForwarderChain
            buf.extend_from_slice(&0_u32.to_le_bytes());
            // Name RVA
            buf.extend_from_slice(&name_rvas[i].to_le_bytes());
            // FirstThunk (IAT RVA)
            buf.extend_from_slice(&iat_rva.to_le_bytes());
        }
        // Null terminator entry (20 zero bytes)
        buf.extend_from_slice(&[0u8; 20]);

        // Write DLL name strings
        buf.extend_from_slice(&names_blob);

        // Write ILT entries (one null u32 per DLL = empty import list)
        for _ in 0..n {
            buf.extend_from_slice(&0_u32.to_le_bytes());
        }

        // Write IAT entries (mirrors ILT)
        for _ in 0..n {
            buf.extend_from_slice(&0_u32.to_le_bytes());
        }

        let section_data_end = buf.len();
        let section_raw_size = (section_data_end - section_data_start) as u32;
        let section_virtual_size = section_raw_size;

        // Align section to file alignment
        let aligned_section_end = (section_data_end + 0x1FF) & !0x1FF;
        buf.resize(aligned_section_end, 0);

        // Fix up section header fields
        buf[virtual_size_offset..virtual_size_offset + 4]
            .copy_from_slice(&section_virtual_size.to_le_bytes());
        buf[raw_size_offset..raw_size_offset + 4]
            .copy_from_slice(&((aligned_section_end - section_data_start) as u32).to_le_bytes());
        buf[raw_ptr_offset..raw_ptr_offset + 4].copy_from_slice(&section_file_offset.to_le_bytes());

        // Fix up import directory data directory
        buf[import_dir_rva_offset..import_dir_rva_offset + 4]
            .copy_from_slice(&section_rva.to_le_bytes());
        buf[import_dir_size_offset..import_dir_size_offset + 4]
            .copy_from_slice(&(idt_size as u32).to_le_bytes());

        // Fix up SizeOfImage: must cover all sections
        let size_of_image = section_rva + ((section_virtual_size + 0xFFF) & !0xFFF);
        buf[size_of_image_offset..size_of_image_offset + 4]
            .copy_from_slice(&size_of_image.to_le_bytes());

        buf
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
        assert!(result.hash.starts_with("sha256:"));
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
        assert_eq!(
            result.anti_cheat,
            vec![AntiCheat::EasyAntiCheat {
                linux_supported: false
            }]
        );
    }

    #[test]
    fn test_battleye_detected() {
        let pe = build_minimal_pe32(&["KERNEL32.dll", "BattlEye.dll"]);
        let result = analyze_pe_bytes(&pe).expect("analysis should succeed");
        assert_eq!(
            result.anti_cheat,
            vec![AntiCheat::BattlEye {
                linux_supported: false
            }]
        );
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
