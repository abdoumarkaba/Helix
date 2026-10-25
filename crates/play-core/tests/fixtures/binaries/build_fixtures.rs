//! Build script for PE fixture binaries.
//!
//! Generates minimal, legally distributable PE files with specific import
//! signatures for testing `binary/pe.rs`. These are NOT real executables —
//! they contain just enough PE structure and import table entries for the
//! analyzer to detect DirectX version, anti-cheat, engine hints, and video
//! codec signals.
//!
//! Run: `cargo run --bin build_fixtures` from the play-core directory,
//! or:  `rustc build_fixtures.rs -o build_fixtures && ./build_fixtures`
//!
//! Requires the `goblin` and `sha2` crates (same versions as play-core).

use std::fs;
use std::path::Path;

/// Minimal PE builder that creates a valid PE header with specified imports.
struct PeBuilder {
    is_64: bool,
    imports: Vec<String>,
}

impl PeBuilder {
    fn new(is_64: bool) -> Self {
        Self {
            is_64,
            imports: Vec::new(),
        }
    }

    fn import(mut self, dll: &str) -> Self {
        self.imports.push(dll.to_lowercase());
        self.imports.push(format!("__dummy_{}", dll.to_lowercase()));
        self
    }

    /// Build a minimal PE binary. The structure is:
    /// - DOS header (64 bytes)
    /// - PE signature + COFF header
    /// - Optional header (PE32 or PE32+)
    /// - Section headers
    /// - Import directory + data
    ///
    /// This produces a file that `goblin::pe::PE::parse()` can successfully
    /// parse and that `analyze_binary()` can extract import DLL names from.
    fn build(&self) -> Vec<u8> {
        // We'll build a real minimal PE using raw bytes.
        // DOS header
        let mut pe = Vec::new();

        // DOS Header (64 bytes minimum)
        pe.extend_from_slice(b"MZ"); // e_magic
        pe.extend_from_slice(&[0x90; 58]); // rest of DOS header (zeros fine)
        pe.extend_from_slice(&(0x80u32.to_le_bytes())); // e_lfanew = offset to PE header at 0x80

        // Pad to 0x80
        while pe.len() < 0x80 {
            pe.push(0);
        }

        // PE Signature
        pe.extend_from_slice(b"PE\0\0");

        // COFF Header (20 bytes)
        let machine = if self.is_64 { 0x8664u16 } else { 0x14Cu16 }; // AMD64 or i386
        pe.extend_from_slice(&machine.to_le_bytes()); // Machine
        pe.extend_from_slice(&(0u16.to_le_bytes())); // NumberOfSections (0 — we don't need real sections for import parsing)
        pe.extend_from_slice(&(0u32.to_le_bytes())); // TimeDateStamp
        pe.extend_from_slice(&(0u32.to_le_bytes())); // PointerToSymbolTable
        pe.extend_from_slice(&(0u32.to_le_bytes())); // NumberOfSymbols
        let optional_header_size = if self.is_64 { 240u16 } else { 224u16 }; // SizeOfOptionalHeader
        pe.extend_from_slice(&optional_header_size.to_le_bytes()); // SizeOfOptionalHeader
        pe.extend_from_slice(&(0x102u16.to_le_bytes())); // Characteristics: EXECUTABLE_IMAGE | 32BIT_MACHINE

        // Optional Header
        let magic = if self.is_64 { 0x20Bu16 } else { 0x10Bu16 }; // PE32+ or PE32
        pe.extend_from_slice(&magic.to_le_bytes()); // Magic

        // For our purposes, we just need the import directory RVA in the data directory.
        // Fill the rest of optional header with zeros, then set the import data directory.
        let optional_start = pe.len();
        // Standard fields (varies by PE32/PE32+)
        pe.extend_from_slice(&[0; 28]); // MajorLinkerVersion..BaseOfData (PE32) / ..ImageBase (PE32+ start)
        if self.is_64 {
            pe.extend_from_slice(&[0; 28]); // PE32+ specific fields
        } else {
            pe.extend_from_slice(&[0; 24]); // PE32 specific fields
        }

        // We need to place the data directories at the right offset.
        // Rather than computing exact offsets, we'll take a simpler approach:
        // just write the DLL names directly in a section and point the import directory at them.

        // Actually, let's use a much simpler approach: just write the DLL names
        // as null-terminated strings in the binary. The goblin parser extracts
        // import DLL names from the import directory. For our test fixtures,
        // we can embed them as strings that goblin will find through the import table.

        // The simplest valid PE with imports requires:
        // 1. A section containing the import data
        // 2. The import directory entries pointing to the DLL name strings
        // 3. The data directory entry pointing to the import directory

        // Let's build this properly with one section.
        pe.clear();

        // Layout:
        // 0x000: DOS header (128 bytes)
        // 0x080: PE signature (4 bytes)
        // 0x084: COFF header (20 bytes)
        // 0x098: Optional header (PE32: 224, PE32+: 240)
        // 0x178/0x188: Section header (40 bytes)
        // 0x1A0/0x1B0: Section data (import directory + DLL names)

        let section_alignment = 0x1000u32;
        let file_alignment = 0x200u32;

        // Section RVA starts at section_alignment
        let section_rva = section_alignment;

        // Calculate import directory size
        // Each import directory entry is 20 bytes, terminated by 20 zero bytes
        let num_imports = self.imports.len() / 2; // each import has dll + hint
        let import_dir_size = (num_imports + 1) * 20; // +1 for null terminator

        // DLL name strings
        let mut dll_name_data = Vec::new();
        let mut dll_name_offsets = Vec::new();
        for dll in &self.imports {
            if !dll.starts_with("__dummy_") {
                dll_name_offsets.push(dll_name_data.len());
                dll_name_data.extend_from_slice(dll.as_bytes());
                dll_name_data.push(0);
            }
        }

        // Section data: import directory entries, then DLL names
        let section_data_start = import_dir_size;
        let total_section_data = import_dir_size + dll_name_data.len();

        // Align section data to file_alignment
        let section_file_size = ((total_section_data + file_alignment as usize - 1) / file_alignment as usize)
            * file_alignment as usize;

        // --- Build the PE ---

        // DOS Header
        pe.extend_from_slice(b"MZ"); // e_magic
        pe.extend_from_slice(&[0; 58]); // rest of DOS header
        pe.extend_from_slice(&0x80u32.to_le_bytes()); // e_lfanew

        // Pad to 0x80
        while pe.len() < 0x80 {
            pe.push(0);
        }

        // PE Signature
        pe.extend_from_slice(b"PE\0\0");

        // COFF Header
        pe.extend_from_slice(&machine.to_le_bytes());
        pe.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections = 1
        pe.extend_from_slice(&(0u32.to_le_bytes())); // TimeDateStamp
        pe.extend_from_slice(&(0u32.to_le_bytes())); // PointerToSymbolTable
        pe.extend_from_slice(&(0u32.to_le_bytes())); // NumberOfSymbols
        pe.extend_from_slice(&optional_header_size.to_le_bytes());
        pe.extend_from_slice(&(0x102u16.to_le_bytes())); // Characteristics

        let optional_header_offset = pe.len();

        // Optional Header
        pe.extend_from_slice(&magic.to_le_bytes()); // Magic
        pe.push(14); // MajorLinkerVersion
        pe.push(0); // MinorLinkerVersion
        pe.extend_from_slice(&(section_file_size as u32).to_le_bytes()); // SizeOfCode
        pe.extend_from_slice(&(0u32.to_le_bytes())); // SizeOfInitializedData
        pe.extend_from_slice(&(0u32.to_le_bytes())); // SizeOfUninitializedData
        pe.extend_from_slice(&(section_rva + 0x1000u32).to_le_bytes()); // AddressOfEntryPoint (dummy)
        pe.extend_from_slice(&section_rva.to_le_bytes()); // BaseOfCode

        if !self.is_64 {
            pe.extend_from_slice(&section_rva.to_le_bytes()); // BaseOfData (PE32 only)
        }

        // ImageBase
        if self.is_64 {
            pe.extend_from_slice(&0x140000000u64.to_le_bytes());
        } else {
            pe.extend_from_slice(&0x400000u32.to_le_bytes());
        }

        pe.extend_from_slice(&section_alignment.to_le_bytes()); // SectionAlignment
        pe.extend_from_slice(&file_alignment.to_le_bytes()); // FileAlignment

        pe.extend_from_slice(&(4u16.to_le_bytes())); // MajorOperatingSystemVersion
        pe.extend_from_slice(&(0u16.to_le_bytes())); // MinorOperatingSystemVersion
        pe.extend_from_slice(&(0u16.to_le_bytes())); // MajorImageVersion
        pe.extend_from_slice(&(0u16.to_le_bytes())); // MinorImageVersion
        pe.extend_from_slice(&(4u16.to_le_bytes())); // MajorSubsystemVersion
        pe.extend_from_slice(&(0u16.to_le_bytes())); // MinorSubsystemVersion
        pe.extend_from_slice(&(0u32.to_le_bytes())); // Win32VersionValue

        let image_size = section_rva + section_alignment + (section_file_size as u32);
        pe.extend_from_slice(&image_size.to_le_bytes()); // SizeOfImage
        let header_size = ((0x200u32 + file_alignment - 1) / file_alignment) * file_alignment;
        pe.extend_from_slice(&header_size.to_le_bytes()); // SizeOfHeaders

        pe.extend_from_slice(&(0u32.to_le_bytes())); // CheckSum
        pe.extend_from_slice(&(3u16.to_le_bytes())); // Subsystem = WINDOWS_CUI
        if self.is_64 {
            pe.extend_from_slice(&(0x8160u16.to_le_bytes())); // DllCharacteristics (DYNAMIC_BASE | NX_COMPAT | TERMINAL_SERVER_AWARE | HIGH_ENTROPY_VA)
        } else {
            pe.extend_from_slice(&(0x8160u16.to_le_bytes())); // DllCharacteristics
        }

        // SizeOfStackReserve, SizeOfStackCommit, SizeOfHeapReserve, SizeOfHeapCommit
        if self.is_64 {
            pe.extend_from_slice(&0x100000u64.to_le_bytes()); // SizeOfStackReserve
            pe.extend_from_slice(&0x1000u64.to_le_bytes()); // SizeOfStackCommit
            pe.extend_from_slice(&0x100000u64.to_le_bytes()); // SizeOfHeapReserve
            pe.extend_from_slice(&0x1000u64.to_le_bytes()); // SizeOfHeapCommit
        } else {
            pe.extend_from_slice(&0x100000u32.to_le_bytes()); // SizeOfStackReserve
            pe.extend_from_slice(&0x1000u32.to_le_bytes()); // SizeOfStackCommit
            pe.extend_from_slice(&0x100000u32.to_le_bytes()); // SizeOfHeapReserve
            pe.extend_from_slice(&0x1000u32.to_le_bytes()); // SizeOfHeapCommit
        }

        pe.extend_from_slice(&(0u32.to_le_bytes())); // LoaderFlags
        pe.extend_from_slice(&(16u32.to_le_bytes())); // NumberOfRvaAndSizes

        // Data directories (16 entries, each 8 bytes = 128 bytes)
        // Directory index 1 = Import Table
        for i in 0..16u32 {
            if i == 1 {
                // Import Table
                pe.extend_from_slice(&section_rva.to_le_bytes()); // RVA
                pe.extend_from_slice(&(import_dir_size as u32).to_le_bytes()); // Size
            } else {
                pe.extend_from_slice(&(0u64.to_le_bytes())); // empty
            }
        }

        // Verify optional header size
        let optional_end = pe.len();
        let actual_optional_size = optional_end - optional_header_offset;
        assert_eq!(actual_optional_size, optional_header_size as usize, "Optional header size mismatch: expected {}, got {}", optional_header_size, actual_optional_size);

        // Section Header (.idata)
        pe.extend_from_slice(b".idata\0\0"); // Name (8 bytes)
        pe.extend_from_slice(&(total_section_data as u32).to_le_bytes()); // VirtualSize
        pe.extend_from_slice(&section_rva.to_le_bytes()); // VirtualAddress
        pe.extend_from_slice(&(section_file_size as u32).to_le_bytes()); // SizeOfRawData
        pe.extend_from_slice(&(header_size.to_le_bytes())); // PointerToRawData
        pe.extend_from_slice(&(0u32.to_le_bytes())); // PointerToRelocations
        pe.extend_from_slice(&(0u32.to_le_bytes())); // PointerToLinenumbers
        pe.extend_from_slice(&(0u16.to_le_bytes())); // NumberOfRelocations
        pe.extend_from_slice(&(0u16.to_le_bytes())); // NumberOfLinenumbers
        pe.extend_from_slice(&(0xC0000040u32.to_le_bytes())); // Characteristics: READ | WRITE | INITIALIZED_DATA

        // Pad headers to file_alignment
        while pe.len() < header_size as usize {
            pe.push(0);
        }

        // Section data: Import Directory Entries
        let mut dll_idx = 0;
        for (i, dll) in self.imports.iter().enumerate() {
            if dll.starts_with("__dummy_") {
                continue;
            }
            // Import Directory Entry (20 bytes)
            let name_rva = section_rva + (section_data_start as u32) + (dll_name_offsets[dll_idx] as u32);
            pe.extend_from_slice(&(0u32.to_le_bytes())); // OriginalFirstThunk (INT)
            pe.extend_from_slice(&(0u32.to_le_bytes())); // TimeDateStamp
            pe.extend_from_slice(&(0u32.to_le_bytes())); // ForwarderChain
            pe.extend_from_slice(&name_rva.to_le_bytes()); // Name
            pe.extend_from_slice(&(0u32.to_le_bytes())); // FirstThunk
            dll_idx += 1;
        }

        // Null terminator entry (20 zero bytes)
        pe.extend_from_slice(&[0; 20]);

        // DLL name strings
        pe.extend_from_slice(&dll_name_data);

        // Pad section to file_alignment
        while pe.len() % (file_alignment as usize) != 0 {
            pe.push(0);
        }

        pe
    }
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".".to_string());
    let out = Path::new(&out_dir);

    let fixtures = [
        (
            "d3d9_32bit.exe",
            PeBuilder::new(false).import("d3d9"),
        ),
        (
            "d3d11_64bit.exe",
            PeBuilder::new(true).import("d3d11"),
        ),
        (
            "d3d12_64bit.exe",
            PeBuilder::new(true).import("d3d12"),
        ),
        (
            "vulkan_64bit.exe",
            PeBuilder::new(true).import("vulkan-1"),
        ),
        (
            "eac_present.exe",
            PeBuilder::new(true).import("easyanticheat_x64"),
        ),
        (
            "bink_video.exe",
            PeBuilder::new(true).import("bink2w64"),
        ),
        (
            "no_imports.exe",
            PeBuilder::new(true),
        ),
    ];

    for (name, builder) in &fixtures {
        let data = builder.build();
        let path = out.join(name);
        fs::write(&path, &data).unwrap_or_else(|e| {
            eprintln!("Failed to write {}: {e}", path.display());
            std::process::exit(1);
        });
        eprintln!("Wrote {} ({} bytes)", path.display(), data.len());
    }

    eprintln!("\nAll fixtures written successfully.");
}
